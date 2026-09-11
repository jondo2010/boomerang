//! Enclave-local stable selection and runtime image materialization.

use super::*;
use crate::compiler;

#[derive(Clone)]
enum ScopeOwner {
    Reactor(compiler::ReactorId),
    Mode(compiler::ModeId),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BindingOwner {
    State(compiler::ReactorId),
    Reaction(compiler::ReactionId),
    Port(compiler::PortId),
    Action(compiler::ActionId),
}

fn packed_overflow(
    enclave: &compiler::StableEnclaveId,
) -> impl FnOnce(PackedSliceOverflow) -> CompileError + '_ {
    |error| CompileError::ResourceOverflow {
        enclave: enclave.clone(),
        resource: error.table(),
    }
}

struct CanonicalEnclaveSelection<'a> {
    reactors: Vec<(&'a compiler::ReactorId, &'a compiler::Reactor)>,
    actions: Vec<(&'a compiler::ActionId, &'a compiler::Action)>,
    modes: Vec<(&'a compiler::ModeId, &'a compiler::Mode)>,
    representatives: Vec<compiler::PortId>,
    reactions: Vec<(&'a compiler::ReactionId, &'a compiler::Reaction)>,
}

impl<'a> CanonicalEnclaveSelection<'a> {
    fn select(
        topology: &'a compiler::ApplicationTopology,
        enclave_id: &compiler::StableEnclaveId,
        analysis: &GlobalAnalysis,
    ) -> Self {
        let mut reactors = topology
            .reactors()
            .filter(|(_, reactor)| reactor.enclave() == enclave_id)
            .collect::<Vec<_>>();
        sort_by_encoded_identity(&mut reactors);
        let reactor_ids = reactors
            .iter()
            .map(|(id, _)| (*id).clone())
            .collect::<BTreeSet<_>>();
        let mut actions = topology
            .actions()
            .filter(|(_, action)| reactor_ids.contains(action.reactor()))
            .collect::<Vec<_>>();
        sort_by_encoded_identity(&mut actions);
        let modes = reactors
            .iter()
            .flat_map(|(reactor_id, _)| {
                let mut values = topology
                    .modes()
                    .filter(move |(_, mode)| mode.reactor() == *reactor_id)
                    .collect::<Vec<_>>();
                sort_by_encoded_identity(&mut values);
                values
            })
            .collect::<Vec<_>>();
        let mut local_ports = topology
            .ports()
            .filter(|(_, port)| reactor_ids.contains(port.reactor()))
            .collect::<Vec<_>>();
        sort_by_encoded_identity(&mut local_ports);
        let port_representatives = local_ports
            .iter()
            .map(|(id, _)| ((*id).clone(), analysis.port_representatives[*id].clone()))
            .collect::<BTreeMap<_, _>>();
        let mut representatives = port_representatives.values().cloned().collect::<Vec<_>>();
        representatives.sort_by_cached_key(canonical_identity_text);
        representatives.dedup();
        let mut reactions = topology
            .reactions()
            .filter(|(_, reaction)| reactor_ids.contains(reaction.reactor()))
            .collect::<Vec<_>>();
        sort_by_encoded_identity(&mut reactions);
        Self {
            reactors,
            actions,
            modes,
            representatives,
            reactions,
        }
    }
}

struct LoweredBindings {
    entries: Box<[RequiredBinding]>,
    indices: BTreeMap<BindingOwner, BindingSlotIndex>,
    images: tinymap::TinyMap<BindingSlotIndex, OwnedBindingImage>,
}

fn lower_bindings(
    deployment: &ResolvedDeployment,
    enclave_id: &compiler::StableEnclaveId,
    reactors: &[(&compiler::ReactorId, &compiler::Reactor)],
    reactions: &[(&compiler::ReactionId, &compiler::Reaction)],
    representatives: &[compiler::PortId],
    actions: &[(&compiler::ActionId, &compiler::Action)],
) -> Result<LoweredBindings, CompileError> {
    let topology = deployment.topology();
    let mut named = reactors
        .iter()
        .map(|(id, reactor)| {
            let slots = DescriptorSlots::for_component(deployment, reactor.component())?;
            Ok((
                BindingOwner::State((*id).clone()),
                format!("state/{id}"),
                RequiredBinding::State {
                    component: reactor.component().clone(),
                    implementation: slots.implementation.clone(),
                    reactor: slots.reactor_slot(id)?,
                },
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    named.extend(
        reactions
            .iter()
            .map(|(id, reaction)| {
                let component = topology
                    .reactor(reaction.reactor())
                    .expect("validated reaction reactor exists")
                    .component();
                let slots = DescriptorSlots::for_component(deployment, component)?;
                Ok((
                    BindingOwner::Reaction((*id).clone()),
                    format!("reaction/{id}"),
                    RequiredBinding::Reaction {
                        component: component.clone(),
                        implementation: slots.implementation.clone(),
                        reaction: slots.reaction_slot(id)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, CompileError>>()?,
    );
    named.extend(
        representatives
            .iter()
            .map(|id| {
                let port = topology.port(id).expect("port representative exists");
                let component = topology
                    .reactor(port.reactor())
                    .expect("validated port reactor exists")
                    .component();
                let slots = DescriptorSlots::for_component(deployment, component)?;
                Ok((
                    BindingOwner::Port(id.clone()),
                    format!("port/{id}"),
                    RequiredBinding::Port {
                        component: component.clone(),
                        implementation: slots.implementation.clone(),
                        port: slots.port_slot(id, port.bank())?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, CompileError>>()?,
    );
    named.extend(
        actions
            .iter()
            .filter(|(_, action)| {
                matches!(
                    action.kind(),
                    compiler::ActionKind::Logical { .. } | compiler::ActionKind::Physical { .. }
                )
            })
            .map(|(id, action)| {
                let component = topology
                    .reactor(action.reactor())
                    .expect("validated action reactor exists")
                    .component();
                let slots = DescriptorSlots::for_component(deployment, component)?;
                Ok((
                    BindingOwner::Action((*id).clone()),
                    format!("action/{id}"),
                    RequiredBinding::Action {
                        component: component.clone(),
                        implementation: slots.implementation.clone(),
                        action: slots.action_slot(id)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, CompileError>>()?,
    );
    named.sort_by(|left, right| left.1.cmp(&right.1));
    let entries = named
        .iter()
        .map(|(_, _, binding)| binding.clone())
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let mut indices = BTreeMap::new();
    let mut images = tinymap::TinyMap::<BindingSlotIndex, _>::new();
    for (owner, id, binding) in &named {
        let key = images
            .try_insert(OwnedBindingImage::new(id, binding.kind()))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "bindings",
            })?;
        indices.insert(owner.clone(), key);
    }
    Ok(LoweredBindings {
        entries,
        indices,
        images,
    })
}

fn lower_routes(
    topology: &compiler::ApplicationTopology,
    enclave_id: &compiler::StableEnclaveId,
    port_indices: &BTreeMap<compiler::PortId, PortIndex>,
) -> Result<tinymap::TinyMap<crate::runtime::image::RouteIndex, OwnedRouteImage>, CompileError> {
    let mut routes = Vec::new();
    for (boundary, connection) in topology.connections() {
        let source_reactor = topology
            .port(connection.source())
            .expect("validated connection source exists")
            .reactor();
        let target_reactor = topology
            .port(connection.target())
            .expect("validated connection target exists")
            .reactor();
        let source_enclave = topology
            .reactor(source_reactor)
            .expect("validated source reactor exists")
            .enclave();
        let target_enclave = topology
            .reactor(target_reactor)
            .expect("validated target reactor exists")
            .enclave();
        let (timing_domain, delay_nanos, scheduled) = match connection.semantics() {
            compiler::ConnectionSemantics::Logical { after } => (
                TimingDomain::Logical,
                duration_nanos(after, enclave_id)?,
                after.is_some_and(|delay| delay > crate::runtime::Duration::ZERO)
                    || source_enclave != target_enclave,
            ),
            compiler::ConnectionSemantics::Physical { after } => (
                TimingDomain::Physical,
                duration_nanos(after, enclave_id)?,
                true,
            ),
        };
        if !scheduled {
            continue;
        }
        if let Some(&local_port) = port_indices.get(connection.source()) {
            routes.push((
                boundary.clone(),
                local_port,
                RouteDirection::Outbound,
                timing_domain,
                delay_nanos,
            ));
        }
        if let Some(&local_port) = port_indices.get(connection.target()) {
            routes.push((
                boundary.clone(),
                local_port,
                RouteDirection::Inbound,
                timing_domain,
                delay_nanos,
            ));
        }
    }
    routes.sort_by_cached_key(|route| {
        (
            canonical_identity_text(&route.0),
            route_direction_rank(route.2),
        )
    });
    tinymap::TinyMap::try_from_iter(routes.into_iter().map(
        |(boundary, local_port, direction, timing_domain, delay_nanos)| OwnedRouteImage {
            boundary,
            local_port,
            direction,
            timing_domain,
            delay_nanos,
        },
    ))
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "routes",
    })
}

struct LoweredEntityTables {
    reactors: tinymap::TinyMap<ReactorIndex, ReactorImage>,
    actions: tinymap::TinyMap<ActionIndex, ActionImage>,
    ports: tinymap::TinyMap<PortIndex, PortImage>,
    reactions: tinymap::TinyMap<ReactionIndex, ReactionImage>,
    modes: tinymap::TinyMap<ModeIndex, ModeImage>,
    reactor_indices: BTreeMap<compiler::ReactorId, ReactorIndex>,
    action_indices: BTreeMap<compiler::ActionId, ActionIndex>,
    mode_indices: BTreeMap<compiler::ModeId, ModeIndex>,
    mode_scopes: BTreeMap<compiler::ModeId, ScopeIndex>,
    port_indices: BTreeMap<compiler::PortId, PortIndex>,
    reaction_indices: BTreeMap<compiler::ReactionId, ReactionIndex>,
    scope_parents: tinymap::TinyMap<ScopeIndex, Option<ScopeIndex>>,
    action_scopes: tinymap::TinyMap<ActionIndex, ScopeIndex>,
    reaction_scopes: tinymap::TinyMap<ReactionIndex, ScopeIndex>,
    reaction_triggers: Box<[LevelReactionImage]>,
    reaction_use_ports: Box<[PortIndex]>,
    reaction_effect_ports: Box<[PortIndex]>,
    reaction_actions: Box<[ActionIndex]>,
    reaction_modes: Box<[ModeIndex]>,
}

fn lower_entity_tables(
    deployment: &ResolvedDeployment,
    enclave_id: &compiler::StableEnclaveId,
    analysis: &GlobalAnalysis,
    selection: &CanonicalEnclaveSelection<'_>,
    bindings: &LoweredBindings,
) -> Result<LoweredEntityTables, CompileError> {
    let topology = deployment.topology();
    let CanonicalEnclaveSelection {
        reactors,
        actions,
        modes,
        representatives,
        reactions,
    } = selection;
    let binding_indices = &bindings.indices;
    let reactor_domain = tinymap::TinyMap::<ReactorIndex, _>::try_from_iter(
        reactors.iter().map(|(id, _)| (*id).clone()),
    )
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "reactors",
    })?;
    let reactor_indices = reactor_domain
        .iter()
        .map(|(key, id)| (id.clone(), key))
        .collect::<BTreeMap<_, _>>();
    let action_domain = tinymap::TinyMap::<ActionIndex, _>::try_from_iter(
        actions.iter().map(|(id, _)| (*id).clone()),
    )
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "actions",
    })?;
    let action_indices = action_domain
        .iter()
        .map(|(key, id)| (id.clone(), key))
        .collect::<BTreeMap<_, _>>();
    let mut mode_domain = tinymap::TinyMap::<ModeIndex, compiler::ModeId>::new();
    let mut reactor_mode_spans = BTreeMap::new();
    for (reactor_id, _) in reactors {
        let reactor_modes = modes
            .iter()
            .filter(|(_, mode)| mode.reactor() == *reactor_id)
            .map(|(id, _)| (*id).clone())
            .collect::<Vec<_>>();
        let span = mode_domain.try_extend_exact(reactor_modes).map_err(|_| {
            CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "modes",
            }
        })?;
        reactor_mode_spans.insert((*reactor_id).clone(), span);
    }
    let mode_indices = mode_domain
        .iter()
        .map(|(key, id)| (id.clone(), key))
        .collect::<BTreeMap<_, _>>();
    let scope_domain = tinymap::TinyMap::<ScopeIndex, ScopeOwner>::try_from_iter(
        reactors
            .iter()
            .map(|(id, _)| ScopeOwner::Reactor((*id).clone()))
            .chain(modes.iter().map(|(id, _)| ScopeOwner::Mode((*id).clone()))),
    )
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "scopes",
    })?;
    let mut root_scopes = BTreeMap::new();
    let mut mode_scopes = BTreeMap::new();
    for (scope, owner) in scope_domain.iter() {
        match owner {
            ScopeOwner::Reactor(id) => root_scopes.insert(id.clone(), scope),
            ScopeOwner::Mode(id) => mode_scopes.insert(id.clone(), scope),
        };
    }
    let representative_domain = tinymap::TinyMap::<PortIndex, _>::try_from_iter(
        representatives.iter().cloned(),
    )
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "ports",
    })?;
    let representative_indices = representative_domain
        .iter()
        .map(|(key, id)| (id.clone(), key))
        .collect::<BTreeMap<_, _>>();
    let port_indices = topology
        .ports()
        .filter(|(_, port)| reactor_indices.contains_key(port.reactor()))
        .map(|(id, _)| {
            (
                id.clone(),
                representative_indices[&analysis.port_representatives[id]],
            )
        })
        .collect::<BTreeMap<_, _>>();
    let reaction_domain = tinymap::TinyMap::<ReactionIndex, _>::try_from_iter(
        reactions.iter().map(|(id, _)| (*id).clone()),
    )
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "reactions",
    })?;
    let reaction_indices = reaction_domain
        .iter()
        .map(|(key, id)| (id.clone(), key))
        .collect::<BTreeMap<_, _>>();
    let scope_for = |reactor: &compiler::ReactorId, mode: Option<&compiler::ModeId>| {
        mode.map_or(root_scopes[reactor], |mode| mode_scopes[mode])
    };
    let state_slots = tinymap::TinyMap::<StateSlotIndex, ()>::try_from_iter(std::iter::repeat_n(
        (),
        reactors.len(),
    ))
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "state-slots",
    })?;
    let reactor_images = reactors
        .iter()
        .zip(state_slots.keys())
        .map(|((id, reactor), state_slot)| {
            ReactorImage::new(
                binding_indices[&BindingOwner::State((*id).clone())],
                state_slot,
                root_scopes[*id],
                reactor_mode_spans[*id],
                modes
                    .iter()
                    .filter(|(_, mode)| mode.reactor() == *id)
                    .find(|(_, mode)| mode.parent().is_none() && mode.is_initial())
                    .map(|(id, _)| mode_indices[*id]),
                reactor.bank().map(|bank| {
                    crate::runtime::image::BankInfoImage::new(bank.index(), bank.total())
                }),
            )
        })
        .collect::<Vec<_>>();
    let reactor_images = tinymap::TinyMap::<ReactorIndex, _>::try_from_iter(reactor_images)
        .map_err(|_| CompileError::ResourceOverflow {
            enclave: enclave_id.clone(),
            resource: "reactors",
        })?;
    let mut flattened_triggers = PackedSliceBuilder::new("reaction-triggers");
    let mut action_triggers = (0..actions.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<ActionIndex, Vec<LevelReactionImage>>>();
    for (id, reaction) in reactions {
        for relation in reaction.relations() {
            if !relation.flags().is_trigger() {
                continue;
            }
            if let compiler::ReactionRelationTarget::Action(action) = relation.target() {
                action_triggers[action_indices[action]].push(LevelReactionImage::new(
                    analysis.reaction_levels[*id],
                    reaction_indices[*id],
                ));
            }
        }
    }
    for triggers in action_triggers.values_mut() {
        triggers.sort_unstable();
        triggers.dedup();
    }
    let action_slots = tinymap::TinyMap::<ActionSlotIndex, ()>::try_from_iter(std::iter::repeat_n(
        (),
        actions.len(),
    ))
    .map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave_id.clone(),
        resource: "action-slots",
    })?;
    let action_images = actions
        .iter()
        .zip(action_triggers.values())
        .zip(action_slots.keys())
        .map(|(((id, action), triggers), action_slot)| {
            let triggers = flattened_triggers
                .try_extend_exact(triggers.iter().copied())
                .map_err(packed_overflow(enclave_id))?;
            let timing = match action.kind() {
                compiler::ActionKind::Logical { minimum_delay } => ActionTiming::Standard {
                    domain: TimingDomain::Logical,
                    min_delay_nanos: duration_nanos(minimum_delay, enclave_id)?,
                },
                compiler::ActionKind::Physical { minimum_delay } => ActionTiming::Standard {
                    domain: TimingDomain::Physical,
                    min_delay_nanos: duration_nanos(minimum_delay, enclave_id)?,
                },
                compiler::ActionKind::Timer { period, .. } => ActionTiming::Timer {
                    period_nanos: period
                        .map(|period| duration_nanos(Some(period), enclave_id))
                        .transpose()?,
                },
                compiler::ActionKind::Startup => ActionTiming::Timer { period_nanos: None },
                compiler::ActionKind::Shutdown => ActionTiming::Shutdown,
            };
            Ok(ActionImage::new(
                scope_for(action.reactor(), action.mode()),
                action_slot,
                timing,
                triggers,
                matches!(
                    action.kind(),
                    compiler::ActionKind::Logical { .. } | compiler::ActionKind::Physical { .. }
                )
                .then(|| binding_indices[&BindingOwner::Action((*id).clone())]),
            ))
        })
        .collect::<Result<tinymap::TinyMap<ActionIndex, _>, CompileError>>()?;
    let mut port_triggers = (0..representatives.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<PortIndex, Vec<LevelReactionImage>>>();
    for (id, reaction) in reactions {
        for relation in reaction.relations() {
            if relation.flags().is_trigger() {
                if let compiler::ReactionRelationTarget::Port(port) = relation.target() {
                    port_triggers[port_indices[port]].push(LevelReactionImage::new(
                        analysis.reaction_levels[*id],
                        reaction_indices[*id],
                    ));
                }
            }
        }
    }
    for triggers in port_triggers.values_mut() {
        triggers.sort_unstable();
        triggers.dedup();
    }
    let port_images = representatives
        .iter()
        .zip(port_triggers.values())
        .map(|(id, triggers)| {
            let port = topology.port(id).expect("port representative exists");
            let triggers = flattened_triggers
                .try_extend_exact(triggers.iter().copied())
                .map_err(packed_overflow(enclave_id))?;
            Ok(PortImage::new(
                scope_for(port.reactor(), port.mode()),
                triggers,
                binding_indices[&BindingOwner::Port(id.clone())],
            ))
        })
        .collect::<Result<tinymap::TinyMap<PortIndex, _>, CompileError>>()?;
    let mut use_ports = PackedSliceBuilder::new("reaction-use-ports");
    let mut effect_ports = PackedSliceBuilder::new("reaction-effect-ports");
    let mut reaction_actions = PackedSliceBuilder::new("reaction-actions");
    let mut reaction_modes = PackedSliceBuilder::new("reaction-modes");
    let reaction_images = reactions
        .iter()
        .map(|(id, reaction)| {
            let mut use_values = Vec::new();
            let mut effect_values = Vec::new();
            let mut action_values = Vec::new();
            for relation in reaction.relations() {
                match relation.target() {
                    compiler::ReactionRelationTarget::Port(port) => {
                        if relation.flags().is_use() && !use_values.contains(&port_indices[port]) {
                            use_values.push(port_indices[port]);
                        }
                        if relation.flags().is_effect()
                            && !effect_values.contains(&port_indices[port])
                        {
                            effect_values.push(port_indices[port]);
                        }
                    }
                    compiler::ReactionRelationTarget::Action(action) => {
                        if relation.flags().is_use() || relation.flags().is_effect() {
                            action_values.push(action_indices[action]);
                        }
                    }
                }
            }
            let use_range = use_ports
                .try_extend_exact(use_values)
                .map_err(packed_overflow(enclave_id))?;
            let effect_range = effect_ports
                .try_extend_exact(effect_values)
                .map_err(packed_overflow(enclave_id))?;
            let action_range = reaction_actions
                .try_extend_exact(action_values)
                .map_err(packed_overflow(enclave_id))?;
            let mode_range = reaction_modes
                .try_extend_exact(
                    reaction
                        .options()
                        .enabled_modes()
                        .iter()
                        .map(|mode| mode_indices[mode]),
                )
                .map_err(packed_overflow(enclave_id))?;
            let image = ReactionImage::new(
                reactor_indices[reaction.reactor()],
                scope_for(reaction.reactor(), reaction.options().mode()),
                analysis.reaction_levels[*id],
                binding_indices[&BindingOwner::Reaction((*id).clone())],
                use_range,
                effect_range,
                action_range,
                mode_range,
            );
            Ok(reaction.options().transition().map_or(image, |transition| {
                image.with_mode_effect(crate::runtime::CompiledModeEffectRef {
                    target: mode_indices[transition.target()],
                    transition: match transition.kind() {
                        compiler::ModeTransitionKind::Reset => {
                            crate::runtime::TransitionKind::Reset
                        }
                        compiler::ModeTransitionKind::History => {
                            crate::runtime::TransitionKind::History
                        }
                    },
                })
            }))
        })
        .collect::<Result<tinymap::TinyMap<ReactionIndex, _>, CompileError>>()?;
    let mode_images = modes
        .iter()
        .map(|(id, mode)| ModeImage::new(reactor_indices[mode.reactor()], mode_scopes[*id]))
        .collect::<tinymap::TinyMap<ModeIndex, _>>();
    let mut scope_parents = tinymap::TinyMap::<ScopeIndex, Option<ScopeIndex>>::with_capacity(
        reactors.len() + modes.len(),
    );
    for (_, reactor) in reactors {
        scope_parents.insert(reactor.parent().and_then(|parent| {
            root_scopes
                .get(parent)
                .map(|root| reactor.scope_mode().map_or(*root, |mode| mode_scopes[mode]))
        }));
    }
    for (_, mode) in modes {
        scope_parents.insert(Some(
            mode.parent()
                .map_or(root_scopes[mode.reactor()], |parent| mode_scopes[parent]),
        ));
    }
    let action_scopes = actions
        .iter()
        .map(|(_, action)| scope_for(action.reactor(), action.mode()))
        .collect::<tinymap::TinyMap<ActionIndex, _>>();
    let reaction_scopes = reactions
        .iter()
        .map(|(_, reaction)| scope_for(reaction.reactor(), reaction.options().mode()))
        .collect::<tinymap::TinyMap<ReactionIndex, _>>();
    Ok(LoweredEntityTables {
        reactors: reactor_images,
        actions: action_images,
        ports: port_images,
        reactions: reaction_images,
        modes: mode_images,
        reactor_indices,
        action_indices,
        mode_indices,
        mode_scopes,
        port_indices,
        reaction_indices,
        scope_parents,
        action_scopes,
        reaction_scopes,
        reaction_triggers: flattened_triggers.into_boxed_slice(),
        reaction_use_ports: use_ports.into_boxed_slice(),
        reaction_effect_ports: effect_ports.into_boxed_slice(),
        reaction_actions: reaction_actions.into_boxed_slice(),
        reaction_modes: reaction_modes.into_boxed_slice(),
    })
}

struct LoweredRelationshipTables {
    scopes: tinymap::TinyMap<ScopeIndex, ScopeImage>,
    scope_descendants: Box<[ScopeIndex]>,
    scope_logical_actions: Box<[ActionIndex]>,
    scope_timer_startups: Box<[TimerStartupImage]>,
    scope_reset_reactions: Box<[LevelReactionImage]>,
    scope_startup_reactions: Box<[LifecycleReactionImage]>,
    scope_shutdown_reactions: Box<[LifecycleReactionImage]>,
    startup_actions: Box<[TimerStartupImage]>,
    timer_startup_actions: Box<[TimerStartupImage]>,
    shutdown_reactions: Box<[LifecycleReactionImage]>,
    shutdown_actions: Box<[ActionIndex]>,
}

fn lower_relationship_tables(
    deployment: &ResolvedDeployment,
    enclave_id: &compiler::StableEnclaveId,
    analysis: &GlobalAnalysis,
    selection: &CanonicalEnclaveSelection<'_>,
    entities: &LoweredEntityTables,
) -> Result<LoweredRelationshipTables, CompileError> {
    let topology = deployment.topology();
    let CanonicalEnclaveSelection {
        reactors,
        actions,
        modes,
        representatives: _,
        reactions,
    } = selection;
    let reactor_indices = &entities.reactor_indices;
    let action_indices = &entities.action_indices;
    let mode_indices = &entities.mode_indices;
    let reaction_indices = &entities.reaction_indices;
    let mode_scopes = &entities.mode_scopes;
    let scope_parents = &entities.scope_parents;
    let action_scopes = &entities.action_scopes;
    let reaction_scopes = &entities.reaction_scopes;
    let level_reaction = |reaction: &compiler::ReactionId| {
        LevelReactionImage::new(
            analysis.reaction_levels[reaction],
            reaction_indices[reaction],
        )
    };
    let mut startup_actions = Vec::new();
    let mut timer_startup_actions = Vec::new();
    for (id, action) in actions {
        match action.kind() {
            compiler::ActionKind::Startup => {
                startup_actions.push(TimerStartupImage::new(action_indices[*id], 0));
            }
            compiler::ActionKind::Timer { offset, .. } => {
                timer_startup_actions.push(TimerStartupImage::new(
                    action_indices[*id],
                    duration_nanos(offset, enclave_id)?,
                ));
            }
            _ => {}
        }
    }
    let mut reset_by_scope = (0..scope_parents.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<ScopeIndex, Vec<LevelReactionImage>>>();
    let mut startup_by_scope = (0..scope_parents.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<ScopeIndex, Vec<LifecycleReactionImage>>>();
    let mut shutdown_by_scope = (0..scope_parents.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<ScopeIndex, Vec<LifecycleReactionImage>>>();
    for ((id, reaction), scope) in reactions.iter().zip(reaction_scopes.values().copied()) {
        for mode in reaction.options().reset_modes() {
            reset_by_scope[mode_scopes[mode]].push(level_reaction(id));
        }
        for relation in reaction.relations() {
            if !relation.flags().is_trigger() {
                continue;
            }
            let compiler::ReactionRelationTarget::Action(action_id) = relation.target() else {
                continue;
            };
            let entry = LifecycleReactionImage::new(level_reaction(id), action_indices[action_id]);
            match topology
                .action(action_id)
                .expect("reaction action exists")
                .kind()
            {
                compiler::ActionKind::Startup => startup_by_scope[scope].push(entry),
                compiler::ActionKind::Shutdown => shutdown_by_scope[scope].push(entry),
                _ => {}
            }
        }
    }
    for values in reset_by_scope.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    for values in startup_by_scope
        .values_mut()
        .chain(shutdown_by_scope.values_mut())
    {
        values.sort_by_key(|entry| entry.reaction());
        values.dedup_by_key(|entry| entry.reaction());
    }
    let is_descendant = |mut candidate: ScopeIndex, ancestor: ScopeIndex| loop {
        if candidate == ancestor {
            break true;
        }
        let Some(parent) = scope_parents[candidate] else {
            break false;
        };
        candidate = parent;
    };
    let mut scope_descendants = PackedSliceBuilder::new("scope-descendants");
    let mut scope_logical_actions = PackedSliceBuilder::new("scope-logical-actions");
    let mut scope_timer_startups = PackedSliceBuilder::new("scope-timer-startups");
    let mut scope_reset_reactions = PackedSliceBuilder::new("scope-reset-reactions");
    let mut scope_startup_reactions = PackedSliceBuilder::new("scope-startup-reactions");
    let mut scope_shutdown_reactions = PackedSliceBuilder::new("scope-shutdown-reactions");
    let scope_images = scope_parents
        .iter()
        .enumerate()
        .map(|(position, (scope, parent))| {
            let descendants = scope_descendants
                .try_extend(
                    scope_parents
                        .keys()
                        .filter(|candidate| is_descendant(*candidate, scope)),
                )
                .map_err(packed_overflow(enclave_id))?;
            let logical_actions = scope_logical_actions
                .try_extend(
                    actions
                        .iter()
                        .zip(action_scopes.values().copied())
                        .filter(|((_, action), action_scope)| {
                            !matches!(action.kind(), compiler::ActionKind::Physical { .. })
                                && is_descendant(*action_scope, scope)
                        })
                        .map(|((id, _), _)| action_indices[*id]),
                )
                .map_err(packed_overflow(enclave_id))?;
            let timer_startups = scope_timer_startups
                .try_extend(
                    timer_startup_actions
                        .iter()
                        .copied()
                        .filter(|entry| is_descendant(action_scopes[entry.action()], scope)),
                )
                .map_err(packed_overflow(enclave_id))?;
            let reset_reactions = scope_reset_reactions
                .try_extend({
                    let mut values = reset_by_scope
                        .iter()
                        .filter(|(candidate, _)| is_descendant(*candidate, scope))
                        .flat_map(|(_, values)| values.iter().copied())
                        .collect::<Vec<_>>();
                    values.sort_unstable();
                    values.dedup();
                    values
                })
                .map_err(packed_overflow(enclave_id))?;
            let startup_reactions = scope_startup_reactions
                .try_extend_exact(startup_by_scope[scope].iter().copied())
                .map_err(packed_overflow(enclave_id))?;
            let shutdown_reactions = scope_shutdown_reactions
                .try_extend_exact(shutdown_by_scope[scope].iter().copied())
                .map_err(packed_overflow(enclave_id))?;
            let (reactor, mode) = if position < reactors.len() {
                (reactor_indices[reactors[position].0], None)
            } else {
                let (mode_id, mode) = modes[position - reactors.len()];
                (reactor_indices[mode.reactor()], Some(mode_indices[mode_id]))
            };
            Ok(ScopeImage::new(
                *parent,
                reactor,
                mode,
                descendants,
                logical_actions,
                timer_startups,
                reset_reactions,
                startup_reactions,
                shutdown_reactions,
            ))
        })
        .collect::<Result<tinymap::TinyMap<ScopeIndex, _>, CompileError>>()?;
    let mut shutdown_reactions = shutdown_by_scope
        .values()
        .flat_map(|values| values.iter().copied())
        .collect::<Vec<_>>();
    shutdown_reactions.sort_by_key(|entry| entry.reaction());
    shutdown_reactions.dedup_by_key(|entry| entry.reaction());
    let mut shutdown_actions = shutdown_reactions
        .iter()
        .map(|entry| entry.action())
        .collect::<Vec<_>>();
    shutdown_actions.sort_unstable();
    shutdown_actions.dedup();
    Ok(LoweredRelationshipTables {
        scopes: scope_images,
        scope_descendants: scope_descendants.into_boxed_slice(),
        scope_logical_actions: scope_logical_actions.into_boxed_slice(),
        scope_timer_startups: scope_timer_startups.into_boxed_slice(),
        scope_reset_reactions: scope_reset_reactions.into_boxed_slice(),
        scope_startup_reactions: scope_startup_reactions.into_boxed_slice(),
        scope_shutdown_reactions: scope_shutdown_reactions.into_boxed_slice(),
        startup_actions: startup_actions.into_boxed_slice(),
        timer_startup_actions: timer_startup_actions.into_boxed_slice(),
        shutdown_reactions: shutdown_reactions.into_boxed_slice(),
        shutdown_actions: shutdown_actions.into_boxed_slice(),
    })
}

fn assemble_enclave_image(
    enclave_id: &compiler::StableEnclaveId,
    bindings: LoweredBindings,
    entities: LoweredEntityTables,
    relationships: LoweredRelationshipTables,
    routes: tinymap::TinyMap<crate::runtime::image::RouteIndex, OwnedRouteImage>,
    storage_bounds: StorageBounds,
) -> Result<OwnedEnclaveImage, CompileError> {
    let LoweredBindings {
        entries: binding_entries,
        indices: _,
        images: binding_images,
    } = bindings;
    let LoweredEntityTables {
        reactors,
        actions,
        ports,
        reactions,
        modes,
        reactor_indices: _,
        action_indices: _,
        mode_indices: _,
        mode_scopes: _,
        port_indices: _,
        reaction_indices: _,
        scope_parents: _,
        action_scopes: _,
        reaction_scopes: _,
        reaction_triggers,
        reaction_use_ports,
        reaction_effect_ports,
        reaction_actions,
        reaction_modes,
    } = entities;
    let LoweredRelationshipTables {
        scopes,
        scope_descendants,
        scope_logical_actions,
        scope_timer_startups,
        scope_reset_reactions,
        scope_startup_reactions,
        scope_shutdown_reactions,
        startup_actions,
        timer_startup_actions,
        shutdown_reactions,
        shutdown_actions,
    } = relationships;
    let owned = OwnedEnclaveImage {
        id: enclave_id.clone(),
        reactors,
        actions,
        ports,
        reactions,
        modes,
        scopes,
        reaction_triggers,
        reaction_use_ports,
        reaction_effect_ports,
        reaction_actions,
        reaction_modes,
        scope_descendants,
        scope_logical_actions,
        scope_timer_startups,
        scope_reset_reactions,
        scope_startup_reactions,
        scope_shutdown_reactions,
        startup_actions,
        timer_startup_actions,
        shutdown_reactions,
        shutdown_actions,
        routes,
        binding_images,
        required_bindings: RequiredBindings {
            entries: binding_entries,
        },
        storage_bounds,
    };
    owned
        .with_view(|_| ())
        .map_err(|error| CompileError::InvalidImage {
            enclave: enclave_id.clone(),
            message: error.to_string(),
        })?;
    Ok(owned)
}

/// Lowers one canonically ordered Enclave slice using deployment-wide analysis.
pub(super) fn lower_enclave(
    deployment: &ResolvedDeployment,
    enclave_id: &compiler::StableEnclaveId,
    analysis: &GlobalAnalysis,
) -> Result<OwnedEnclaveImage, CompileError> {
    let topology = deployment.topology();
    let selection = CanonicalEnclaveSelection::select(topology, enclave_id, analysis);
    let bindings = lower_bindings(
        deployment,
        enclave_id,
        &selection.reactors,
        &selection.reactions,
        &selection.representatives,
        &selection.actions,
    )?;
    let entities = lower_entity_tables(deployment, enclave_id, analysis, &selection, &bindings)?;
    let relationships =
        lower_relationship_tables(deployment, enclave_id, analysis, &selection, &entities)?;
    let route_images = lower_routes(topology, enclave_id, &entities.port_indices)?;
    let storage_bounds = storage_bounds(
        deployment,
        enclave_id,
        &selection.reactors,
        selection.actions.len(),
    )?;
    assemble_enclave_image(
        enclave_id,
        bindings,
        entities,
        relationships,
        route_images,
        storage_bounds,
    )
}
