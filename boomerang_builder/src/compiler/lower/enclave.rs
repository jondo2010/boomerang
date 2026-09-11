//! Enclave-local stable selection and runtime image materialization.

use super::*;
use crate::compiler;

#[derive(Clone)]
/// Stable semantic owner of one execution scope before dense keys are allocated.
enum ScopeOwner {
    /// The root scope owned by a reactor.
    Reactor(compiler::ReactorId),
    /// A nested scope owned by a reactor mode.
    Mode(compiler::ModeId),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
/// Stable semantic owner of one required runtime binding.
enum BindingOwner {
    /// State initializer for the identified reactor.
    State(compiler::ReactorId),
    /// Callback implementation for the identified reaction.
    Reaction(compiler::ReactionId),
    /// Payload implementation for the identified representative port.
    Port(compiler::PortId),
    /// Payload implementation for the identified standard action.
    Action(compiler::ActionId),
}

/// Converts a packed-slice capacity error into an Enclave-scoped compile error.
fn packed_overflow(
    enclave: &compiler::StableEnclaveId,
) -> impl FnOnce(PackedSliceOverflow) -> CompileError + '_ {
    |error| CompileError::ResourceOverflow {
        enclave: enclave.clone(),
        resource: error.table(),
    }
}

/// Canonically ordered stable semantic records selected for one Enclave.
struct CanonicalEnclaveSelection<'a> {
    /// Reactors assigned to the Enclave.
    reactors: Vec<(&'a compiler::ReactorId, &'a compiler::Reactor)>,
    /// Actions owned by the selected reactors.
    actions: Vec<(&'a compiler::ActionId, &'a compiler::Action)>,
    /// Modes grouped by their canonical reactor order.
    modes: Vec<(&'a compiler::ModeId, &'a compiler::Mode)>,
    /// Canonical representatives of local port equivalence classes.
    representatives: Vec<compiler::PortId>,
    /// Reactions owned by the selected reactors.
    reactions: Vec<(&'a compiler::ReactionId, &'a compiler::Reaction)>,
}

impl<'a> CanonicalEnclaveSelection<'a> {
    /// Selects and canonically orders the stable semantic records owned by `enclave_id`.
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

/// Runtime binding rows and the table-local stable-owner lookup used while lowering.
struct LoweredBindings {
    /// Stable host-side requirements in binding-key order.
    entries: Box<[RequiredBinding]>,
    /// Dense binding key allocated for each stable semantic owner.
    indices: BTreeMap<BindingOwner, BindingSlotIndex>,
    /// Runtime image rows keyed by their owner-generated binding identities.
    images: tinymap::TinyMap<BindingSlotIndex, OwnedBindingImage>,
}

/// Lowers boundary routes incident on the Enclave's locally materialized ports.
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

/// Dense entity tables plus temporary stable-to-dense resolution state.
struct LoweredEntityTables {
    /// Runtime reactor rows in canonical stable-identity order.
    reactors: tinymap::TinyMap<ReactorIndex, ReactorImage>,
    /// Runtime action rows in canonical stable-identity order.
    actions: tinymap::TinyMap<ActionIndex, ActionImage>,
    /// Runtime representative-port rows in canonical stable-identity order.
    ports: tinymap::TinyMap<PortIndex, PortImage>,
    /// Runtime reaction rows in canonical stable-identity order.
    reactions: tinymap::TinyMap<ReactionIndex, ReactionImage>,
    /// Runtime mode rows grouped by their owning reactor.
    modes: tinymap::TinyMap<ModeIndex, ModeImage>,
    /// Runtime scope rows in owner-generated dense-key order.
    scopes: tinymap::TinyMap<ScopeIndex, ScopeImage>,
    /// Packed action and port trigger relationships.
    reaction_triggers: Box<[LevelReactionImage]>,
    /// Packed ordered reaction input-port relationships.
    reaction_use_ports: Box<[PortIndex]>,
    /// Packed ordered reaction output-port relationships.
    reaction_effect_ports: Box<[PortIndex]>,
    /// Packed ordered reaction action-effect relationships.
    reaction_actions: Box<[ActionIndex]>,
    /// Packed reaction enabled-mode filters.
    reaction_modes: Box<[ModeIndex]>,
}

/// Narrow resolution state required to build scope and lifecycle relationships.
struct RelationshipInputs {
    /// Dense reactor key for each stable reactor identity.
    reactor_indices: BTreeMap<compiler::ReactorId, ReactorIndex>,
    /// Dense action key for each stable action identity.
    action_indices: BTreeMap<compiler::ActionId, ActionIndex>,
    /// Dense mode key for each stable mode identity.
    mode_indices: BTreeMap<compiler::ModeId, ModeIndex>,
    /// Dense execution scope allocated for each stable mode identity.
    mode_scopes: BTreeMap<compiler::ModeId, ScopeIndex>,
    /// Dense reaction key for each stable reaction identity.
    reaction_indices: BTreeMap<compiler::ReactionId, ReactionIndex>,
    /// Parent relation for every final owner-generated execution scope.
    scope_parents: BTreeMap<ScopeIndex, Option<ScopeIndex>>,
    /// Stable semantic owner recorded for every final owner-generated scope key.
    scope_owners: BTreeMap<ScopeIndex, ScopeOwner>,
    /// Execution scope containing each final owner-generated action key.
    action_scopes: BTreeMap<ActionIndex, ScopeIndex>,
    /// Execution scope containing each final owner-generated reaction key.
    reaction_scopes: BTreeMap<ReactionIndex, ScopeIndex>,
}

/// Allocates dense entity keys and resolves stable intra-Enclave references.
fn lower_entity_tables(
    deployment: &ResolvedDeployment,
    enclave_id: &compiler::StableEnclaveId,
    analysis: &GlobalAnalysis,
    selection: &CanonicalEnclaveSelection<'_>,
    bindings: &LoweredBindings,
) -> Result<
    (
        LoweredEntityTables,
        RelationshipInputs,
        BTreeMap<compiler::PortId, PortIndex>,
    ),
    CompileError,
> {
    let topology = deployment.topology();
    let CanonicalEnclaveSelection {
        reactors,
        actions,
        modes,
        representatives,
        reactions,
    } = selection;
    let binding_indices = &bindings.indices;
    let empty_span = tinymap::IndexSpan::new(0, 0);
    let mut reactor_images = tinymap::TinyMap::<ReactorIndex, _>::new();
    let mut reactor_indices = BTreeMap::new();
    for (id, _) in reactors {
        let key = reactor_images
            .try_insert(ReactorImage::new(
                BindingSlotIndex::new(0),
                StateSlotIndex::new(0),
                ScopeIndex::new(0),
                empty_span,
                None,
                None,
            ))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "reactors",
            })?;
        reactor_indices.insert((*id).clone(), key);
    }
    let mut action_images = tinymap::TinyMap::<ActionIndex, _>::new();
    let mut action_indices = BTreeMap::new();
    for (id, _) in actions {
        let key = action_images
            .try_insert(ActionImage::new(
                ScopeIndex::new(0),
                ActionSlotIndex::new(0),
                ActionTiming::Shutdown,
                tinymap::SliceRange::new(0, 0),
                None,
            ))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "actions",
            })?;
        action_indices.insert((*id).clone(), key);
    }
    let mut mode_images = tinymap::TinyMap::<ModeIndex, ModeImage>::new();
    let mut mode_indices = BTreeMap::new();
    let mut reactor_mode_spans = BTreeMap::new();
    for (reactor_id, _) in reactors {
        let reactor_modes = modes
            .iter()
            .filter(|(_, mode)| mode.reactor() == *reactor_id)
            .copied()
            .collect::<Vec<_>>();
        let span = mode_images
            .try_extend_exact_with_key(reactor_modes, |key, (id, _)| {
                mode_indices.insert(id.clone(), key);
                ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(0))
            })
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "modes",
            })?;
        reactor_mode_spans.insert((*reactor_id).clone(), span);
    }
    let mut scope_images = tinymap::TinyMap::<ScopeIndex, ScopeImage>::new();
    let mut root_scopes = BTreeMap::new();
    let mut mode_scopes = BTreeMap::new();
    let mut scope_owners_by_key = BTreeMap::new();
    let scope_owners = reactors
        .iter()
        .map(|(id, _)| ScopeOwner::Reactor((*id).clone()))
        .chain(modes.iter().map(|(id, _)| ScopeOwner::Mode((*id).clone())));
    for owner in scope_owners {
        let scope = scope_images
            .try_insert(ScopeImage::new(
                None,
                ReactorIndex::new(0),
                None,
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
            ))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "scopes",
            })?;
        scope_owners_by_key.insert(scope, owner.clone());
        match owner {
            ScopeOwner::Reactor(id) => root_scopes.insert(id, scope),
            ScopeOwner::Mode(id) => mode_scopes.insert(id, scope),
        };
    }
    let mut port_images = tinymap::TinyMap::<PortIndex, PortImage>::new();
    let mut representative_indices = BTreeMap::new();
    for id in representatives {
        let key = port_images
            .try_insert(PortImage::new(
                ScopeIndex::new(0),
                tinymap::SliceRange::new(0, 0),
                BindingSlotIndex::new(0),
            ))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "ports",
            })?;
        representative_indices.insert(id.clone(), key);
    }
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
    let mut reaction_images = tinymap::TinyMap::<ReactionIndex, ReactionImage>::new();
    let mut reaction_indices = BTreeMap::new();
    for (id, _) in reactions {
        let key = reaction_images
            .try_insert(ReactionImage::new(
                ReactorIndex::new(0),
                ScopeIndex::new(0),
                0,
                BindingSlotIndex::new(0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
                tinymap::SliceRange::new(0, 0),
            ))
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "reactions",
            })?;
        reaction_indices.insert((*id).clone(), key);
    }
    let scope_for = |reactor: &compiler::ReactorId, mode: Option<&compiler::ModeId>| {
        mode.map_or(root_scopes[reactor], |mode| mode_scopes[mode])
    };
    let mut state_slots = tinymap::TinyMap::<StateSlotIndex, ()>::new();
    let mut state_slot_indices = BTreeMap::new();
    for (id, _) in reactors {
        let key = state_slots
            .try_insert(())
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "state-slots",
            })?;
        state_slot_indices.insert((*id).clone(), key);
    }
    for (id, reactor) in reactors {
        reactor_images[reactor_indices[*id]] = ReactorImage::new(
            binding_indices[&BindingOwner::State((*id).clone())],
            state_slot_indices[*id],
            root_scopes[*id],
            reactor_mode_spans[*id],
            modes
                .iter()
                .filter(|(_, mode)| mode.reactor() == *id)
                .find(|(_, mode)| mode.parent().is_none() && mode.is_initial())
                .map(|(id, _)| mode_indices[*id]),
            reactor
                .bank()
                .map(|bank| crate::runtime::image::BankInfoImage::new(bank.index(), bank.total())),
        );
    }
    let mut flattened_triggers = PackedSliceBuilder::new("reaction-triggers");
    let mut action_triggers = action_images
        .keys()
        .map(|key| (key, Vec::new()))
        .collect::<BTreeMap<_, Vec<LevelReactionImage>>>();
    for (id, reaction) in reactions {
        for relation in reaction.relations() {
            if !relation.flags().is_trigger() {
                continue;
            }
            if let compiler::ReactionRelationTarget::Action(action) = relation.target() {
                action_triggers
                    .get_mut(&action_indices[action])
                    .expect("action key belongs to the final action table")
                    .push(LevelReactionImage::new(
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
    let mut action_slots = tinymap::TinyMap::<ActionSlotIndex, ()>::new();
    let mut action_slot_indices = BTreeMap::new();
    for (id, _) in actions {
        let key = action_slots
            .try_insert(())
            .map_err(|_| CompileError::ResourceOverflow {
                enclave: enclave_id.clone(),
                resource: "action-slots",
            })?;
        action_slot_indices.insert((*id).clone(), key);
    }
    for (id, action) in actions {
        let triggers = &action_triggers[&action_indices[*id]];
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
        action_images[action_indices[*id]] = ActionImage::new(
            scope_for(action.reactor(), action.mode()),
            action_slot_indices[*id],
            timing,
            triggers,
            matches!(
                action.kind(),
                compiler::ActionKind::Logical { .. } | compiler::ActionKind::Physical { .. }
            )
            .then(|| binding_indices[&BindingOwner::Action((*id).clone())]),
        );
    }
    let mut port_triggers = port_images
        .keys()
        .map(|key| (key, Vec::new()))
        .collect::<BTreeMap<_, Vec<LevelReactionImage>>>();
    for (id, reaction) in reactions {
        for relation in reaction.relations() {
            if relation.flags().is_trigger() {
                if let compiler::ReactionRelationTarget::Port(port) = relation.target() {
                    port_triggers
                        .get_mut(&port_indices[port])
                        .expect("port key belongs to the final port table")
                        .push(LevelReactionImage::new(
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
    for id in representatives {
        let triggers = &port_triggers[&representative_indices[id]];
        let port = topology.port(id).expect("port representative exists");
        let triggers = flattened_triggers
            .try_extend_exact(triggers.iter().copied())
            .map_err(packed_overflow(enclave_id))?;
        port_images[representative_indices[id]] = PortImage::new(
            scope_for(port.reactor(), port.mode()),
            triggers,
            binding_indices[&BindingOwner::Port(id.clone())],
        );
    }
    let mut use_ports = PackedSliceBuilder::new("reaction-use-ports");
    let mut effect_ports = PackedSliceBuilder::new("reaction-effect-ports");
    let mut reaction_actions = PackedSliceBuilder::new("reaction-actions");
    let mut reaction_modes = PackedSliceBuilder::new("reaction-modes");
    for (id, reaction) in reactions {
        let mut use_values = Vec::new();
        let mut effect_values = Vec::new();
        let mut action_values = Vec::new();
        for relation in reaction.relations() {
            match relation.target() {
                compiler::ReactionRelationTarget::Port(port) => {
                    if relation.flags().is_use() && !use_values.contains(&port_indices[port]) {
                        use_values.push(port_indices[port]);
                    }
                    if relation.flags().is_effect() && !effect_values.contains(&port_indices[port])
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
        reaction_images[reaction_indices[*id]] =
            reaction.options().transition().map_or(image, |transition| {
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
            });
    }
    for (id, mode) in modes {
        mode_images[mode_indices[*id]] =
            ModeImage::new(reactor_indices[mode.reactor()], mode_scopes[*id]);
    }
    let mut scope_parents = BTreeMap::new();
    for (id, reactor) in reactors {
        scope_parents.insert(
            root_scopes[*id],
            reactor.parent().and_then(|parent| {
                root_scopes
                    .get(parent)
                    .map(|root| reactor.scope_mode().map_or(*root, |mode| mode_scopes[mode]))
            }),
        );
    }
    for (id, mode) in modes {
        scope_parents.insert(
            mode_scopes[*id],
            Some(
                mode.parent()
                    .map_or(root_scopes[mode.reactor()], |parent| mode_scopes[parent]),
            ),
        );
    }
    let action_scopes = actions
        .iter()
        .map(|(id, action)| {
            (
                action_indices[*id],
                scope_for(action.reactor(), action.mode()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let reaction_scopes = reactions
        .iter()
        .map(|(id, reaction)| {
            (
                reaction_indices[*id],
                scope_for(reaction.reactor(), reaction.options().mode()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok((
        LoweredEntityTables {
            reactors: reactor_images,
            actions: action_images,
            ports: port_images,
            reactions: reaction_images,
            modes: mode_images,
            scopes: scope_images,
            reaction_triggers: flattened_triggers.into_boxed_slice(),
            reaction_use_ports: use_ports.into_boxed_slice(),
            reaction_effect_ports: effect_ports.into_boxed_slice(),
            reaction_actions: reaction_actions.into_boxed_slice(),
            reaction_modes: reaction_modes.into_boxed_slice(),
        },
        RelationshipInputs {
            reactor_indices,
            action_indices,
            mode_indices,
            mode_scopes,
            reaction_indices,
            scope_parents,
            scope_owners: scope_owners_by_key,
            action_scopes,
            reaction_scopes,
        },
        port_indices,
    ))
}

/// Scope and lifecycle tables derived after all entity keys are known.
struct LoweredRelationshipTables {
    /// Packed transitive descendants owned by individual scopes.
    scope_descendants: Box<[ScopeIndex]>,
    /// Packed logical actions owned by individual scopes.
    scope_logical_actions: Box<[ActionIndex]>,
    /// Packed timer startup records owned by individual scopes.
    scope_timer_startups: Box<[TimerStartupImage]>,
    /// Packed reset reactions owned by individual scopes.
    scope_reset_reactions: Box<[LevelReactionImage]>,
    /// Packed startup reactions owned by individual scopes.
    scope_startup_reactions: Box<[LifecycleReactionImage]>,
    /// Packed shutdown reactions owned by individual scopes.
    scope_shutdown_reactions: Box<[LifecycleReactionImage]>,
    /// Enclave-wide one-shot startup actions.
    startup_actions: Box<[TimerStartupImage]>,
    /// Enclave-wide periodic timer startup actions.
    timer_startup_actions: Box<[TimerStartupImage]>,
    /// Enclave-wide shutdown reactions.
    shutdown_reactions: Box<[LifecycleReactionImage]>,
    /// Enclave-wide actions populated during shutdown.
    shutdown_actions: Box<[ActionIndex]>,
}

/// Builds packed scope and lifecycle relationships from resolved entity keys.
fn lower_relationship_tables(
    topology: &ApplicationTopology,
    enclave_id: &compiler::StableEnclaveId,
    analysis: &GlobalAnalysis,
    selection: &CanonicalEnclaveSelection<'_>,
    scopes: &mut tinymap::TinyMap<ScopeIndex, ScopeImage>,
    inputs: &RelationshipInputs,
) -> Result<LoweredRelationshipTables, CompileError> {
    let CanonicalEnclaveSelection {
        reactors: _,
        actions,
        modes: _,
        representatives: _,
        reactions,
    } = selection;
    let reactor_indices = &inputs.reactor_indices;
    let action_indices = &inputs.action_indices;
    let mode_indices = &inputs.mode_indices;
    let reaction_indices = &inputs.reaction_indices;
    let mode_scopes = &inputs.mode_scopes;
    let scope_parents = &inputs.scope_parents;
    let scope_owners = &inputs.scope_owners;
    let action_scopes = &inputs.action_scopes;
    let reaction_scopes = &inputs.reaction_scopes;
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
    let mut reset_by_scope = scopes
        .keys()
        .map(|scope| (scope, Vec::new()))
        .collect::<BTreeMap<_, Vec<LevelReactionImage>>>();
    let mut startup_by_scope = scopes
        .keys()
        .map(|scope| (scope, Vec::new()))
        .collect::<BTreeMap<_, Vec<LifecycleReactionImage>>>();
    let mut shutdown_by_scope = scopes
        .keys()
        .map(|scope| (scope, Vec::new()))
        .collect::<BTreeMap<_, Vec<LifecycleReactionImage>>>();
    for (id, reaction) in reactions {
        let scope = reaction_scopes[&reaction_indices[*id]];
        for mode in reaction.options().reset_modes() {
            reset_by_scope
                .get_mut(&mode_scopes[mode])
                .expect("mode scope belongs to the final scope table")
                .push(level_reaction(id));
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
                compiler::ActionKind::Startup => startup_by_scope
                    .get_mut(&scope)
                    .expect("reaction scope belongs to the final scope table")
                    .push(entry),
                compiler::ActionKind::Shutdown => shutdown_by_scope
                    .get_mut(&scope)
                    .expect("reaction scope belongs to the final scope table")
                    .push(entry),
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
        let Some(parent) = scope_parents[&candidate] else {
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
    for (&scope, &parent) in scope_parents {
        let descendants = scope_descendants
            .try_extend(
                scope_parents
                    .keys()
                    .copied()
                    .filter(|candidate| is_descendant(*candidate, scope)),
            )
            .map_err(packed_overflow(enclave_id))?;
        let logical_actions = scope_logical_actions
            .try_extend(
                actions
                    .iter()
                    .filter(|(id, action)| {
                        let action_scope = action_scopes[&action_indices[*id]];
                        !matches!(action.kind(), compiler::ActionKind::Physical { .. })
                            && is_descendant(action_scope, scope)
                    })
                    .map(|(id, _)| action_indices[*id]),
            )
            .map_err(packed_overflow(enclave_id))?;
        let timer_startups = scope_timer_startups
            .try_extend(
                timer_startup_actions
                    .iter()
                    .copied()
                    .filter(|entry| is_descendant(action_scopes[&entry.action()], scope)),
            )
            .map_err(packed_overflow(enclave_id))?;
        let reset_reactions = scope_reset_reactions
            .try_extend({
                let mut values = reset_by_scope
                    .iter()
                    .filter(|(candidate, _)| is_descendant(**candidate, scope))
                    .flat_map(|(_, values)| values.iter().copied())
                    .collect::<Vec<_>>();
                values.sort_unstable();
                values.dedup();
                values
            })
            .map_err(packed_overflow(enclave_id))?;
        let startup_reactions = scope_startup_reactions
            .try_extend_exact(startup_by_scope[&scope].iter().copied())
            .map_err(packed_overflow(enclave_id))?;
        let shutdown_reactions = scope_shutdown_reactions
            .try_extend_exact(shutdown_by_scope[&scope].iter().copied())
            .map_err(packed_overflow(enclave_id))?;
        let (reactor, mode) = match &scope_owners[&scope] {
            ScopeOwner::Reactor(reactor) => (reactor_indices[reactor], None),
            ScopeOwner::Mode(mode_id) => {
                let mode = topology.mode(mode_id).expect("selected mode exists");
                (reactor_indices[mode.reactor()], Some(mode_indices[mode_id]))
            }
        };
        scopes[scope] = ScopeImage::new(
            parent,
            reactor,
            mode,
            descendants,
            logical_actions,
            timer_startups,
            reset_reactions,
            startup_reactions,
            shutdown_reactions,
        );
    }
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

/// Assembles and validates the final owned runtime image from lowered table groups.
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
        scopes,
        reaction_triggers,
        reaction_use_ports,
        reaction_effect_ports,
        reaction_actions,
        reaction_modes,
    } = entities;
    let LoweredRelationshipTables {
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

impl ResolvedDeployment {
    /// Lowers one canonically ordered Enclave slice using deployment-wide analysis.
    pub(super) fn lower_enclave(
        &self,
        enclave_id: &compiler::StableEnclaveId,
        analysis: &GlobalAnalysis,
    ) -> Result<OwnedEnclaveImage, CompileError> {
        let topology = self.topology();
        let selection = CanonicalEnclaveSelection::select(topology, enclave_id, analysis);
        let bindings = self.lower_bindings(
            enclave_id,
            &selection.reactors,
            &selection.reactions,
            &selection.representatives,
            &selection.actions,
        )?;
        let (mut entities, relationship_inputs, port_indices) =
            lower_entity_tables(self, enclave_id, analysis, &selection, &bindings)?;
        let relationships = lower_relationship_tables(
            self.topology(),
            enclave_id,
            analysis,
            &selection,
            &mut entities.scopes,
            &relationship_inputs,
        )?;
        let route_images = lower_routes(topology, enclave_id, &port_indices)?;
        let storage_bounds = storage_bounds(
            self,
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

    /// Allocates the Enclave binding table from stable semantic owners.
    fn lower_bindings(
        &self,
        enclave_id: &compiler::StableEnclaveId,
        reactors: &[(&compiler::ReactorId, &compiler::Reactor)],
        reactions: &[(&compiler::ReactionId, &compiler::Reaction)],
        representatives: &[compiler::PortId],
        actions: &[(&compiler::ActionId, &compiler::Action)],
    ) -> Result<LoweredBindings, CompileError> {
        let topology = self.topology();
        let mut named = reactors
            .iter()
            .map(|(id, reactor)| {
                let slots = DescriptorSlots::for_component(self, reactor.component())?;
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
                    let slots = DescriptorSlots::for_component(self, component)?;
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
                    let slots = DescriptorSlots::for_component(self, component)?;
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
                        compiler::ActionKind::Logical { .. }
                            | compiler::ActionKind::Physical { .. }
                    )
                })
                .map(|(id, action)| {
                    let component = topology
                        .reactor(action.reactor())
                        .expect("validated action reactor exists")
                        .component();
                    let slots = DescriptorSlots::for_component(self, component)?;
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
}
