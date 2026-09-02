use super::identity::canonical_identity_text;
use super::{
    GlobalFederationImage, OwnedCompiledDeployment, OwnedEnclaveImage, OwnedFederateImage,
    RequiredBinding, RequiredBindings, ResolvedDeployment,
};
use crate::{
    descriptor::{ActionSlotId, DescriptorBound, PortSlotId, ReactionSlotId, ReactorSlotId},
    runtime::image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingSlotIndex,
        CompiledDeploymentImage, CompiledDeploymentView, CoordinationProjection, EnclaveIndex,
        FederateImage, FederateIndex, IdentityRange, LevelReactionImage, LifecycleReactionImage,
        ModeImage, ModeIndex, PortImage, PortIndex, ReactionImage, ReactionIndex, ReactorImage,
        ReactorIndex, RequiredBindingImage, RouteDirection, RouteImage, ScopeImage, ScopeIndex,
        StateSlotIndex, StorageBounds, TableRange, TimerStartupImage, TimingDomain,
    },
};
use std::collections::{BTreeMap, BTreeSet};

/// A canonical compiled-image lowering failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CompileError {
    /// This slice cannot project a distributed coordination backend.
    #[error("distributed coordination projection is not implemented")]
    UnsupportedCoordination,
    /// A reaction requests a mode transition that the compiled image cannot yet represent.
    #[error("reaction {reaction} requests an unsupported compiled mode transition")]
    UnsupportedModeTransition {
        /// Stable identity of the transition-bearing reaction.
        reaction: super::ReactionId,
    },
    /// A component did not declare a required finite resource bound.
    #[error("component {component} in Enclave {enclave} has no {resource} bound")]
    UnboundedResource {
        /// Stable component identity.
        component: super::ComponentInstanceId,
        /// Stable Enclave identity.
        enclave: super::StableEnclaveId,
        /// Missing resource category.
        resource: &'static str,
    },
    /// A bounded resource cannot be represented or aggregated.
    #[error("{resource} bound overflows in Enclave {enclave}")]
    ResourceOverflow {
        /// Stable Enclave identity.
        enclave: super::StableEnclaveId,
        /// Overflowing resource category.
        resource: &'static str,
    },
    /// Root deployment validation failed before an Enclave could be selected.
    #[error("compiled deployment is invalid: {message}")]
    InvalidDeployment {
        /// Root image validation detail.
        message: String,
    },
    /// Lowering produced an invalid target-facing image.
    #[error("compiled Enclave {enclave} is invalid: {message}")]
    InvalidImage {
        /// Stable Enclave identity.
        enclave: super::StableEnclaveId,
        /// Runtime image validation detail.
        message: String,
    },
    /// Same-tag reaction dependencies contain a cycle.
    #[error("reaction dependency cycle in Enclave {enclave}")]
    ReactionCycle {
        /// Stable Enclave identity.
        enclave: super::StableEnclaveId,
        /// Canonically ordered stable identities blocked by the cycle.
        reactions: Box<[super::ReactionId]>,
    },
    /// An implementation descriptor does not provide one root reactor slot.
    #[error(
        "implementation {implementation} for component {component} declares {roots} root reactor slots"
    )]
    DescriptorRoot {
        /// Logical component instance receiving the implementation.
        component: super::ComponentInstanceId,
        /// Selected implementation with the invalid descriptor root shape.
        implementation: super::ImplementationId,
        /// Number of parentless descriptor reactor slots.
        roots: usize,
    },
    /// A logical runtime binding has no matching descriptor-local slot.
    #[error(
        "implementation {implementation} for component {component} has no {kind} slot matching {logical}"
    )]
    MissingDescriptorSlot {
        /// Logical component instance receiving the implementation.
        component: super::ComponentInstanceId,
        /// Selected implementation lacking the descriptor slot.
        implementation: super::ImplementationId,
        /// Required direct binding category.
        kind: &'static str,
        /// Fully qualified logical path of the unmatched binding.
        logical: String,
    },
    /// A logical runtime binding matches more than one descriptor-local slot.
    #[error(
        "implementation {implementation} for component {component} has {candidates} {kind} slots matching {logical}"
    )]
    AmbiguousDescriptorSlot {
        /// Logical component instance receiving the implementation.
        component: super::ComponentInstanceId,
        /// Selected implementation with ambiguous descriptor slots.
        implementation: super::ImplementationId,
        /// Required direct binding category.
        kind: &'static str,
        /// Fully qualified logical path of the ambiguous binding.
        logical: String,
        /// Number of matching descriptor slots.
        candidates: usize,
    },
}

/// Canonical deployment-wide facts computed before image slicing.
struct GlobalAnalysis {
    /// Smallest stable port identity representing each zero-delay local equivalence class.
    port_representatives: BTreeMap<super::PortId, super::PortId>,
    /// Longest-predecessor dependency level for every reaction.
    reaction_levels: BTreeMap<super::ReactionId, u32>,
}

/// Resolves descriptor-local slots for one selected implementation.
struct DescriptorSlots<'a> {
    /// Logical component instance receiving the selected implementation.
    component: &'a super::ComponentInstanceId,
    /// Selected implementation exporting direct binding symbols.
    implementation: &'a super::ImplementationId,
    /// Canonical implementation descriptor.
    descriptor: &'a crate::descriptor::ComponentDescriptor,
    /// Unique parentless descriptor reactor slot.
    root: &'a crate::descriptor::ReactorSlot,
}

impl<'a> DescriptorSlots<'a> {
    /// Looks up the selected descriptor and its unique root reactor slot.
    fn for_component(
        deployment: &'a ResolvedDeployment,
        component: &'a super::ComponentInstanceId,
    ) -> Result<Self, CompileError> {
        let binding = deployment
            .binding(component)
            .expect("resolved deployment binds every topology component");
        let roots = binding
            .descriptor()
            .reactor_slots()
            .iter()
            .filter(|slot| slot.parent.is_none())
            .collect::<Vec<_>>();
        let [root] = roots.as_slice() else {
            return Err(CompileError::DescriptorRoot {
                component: component.clone(),
                implementation: binding.implementation().clone(),
                roots: roots.len(),
            });
        };
        Ok(Self {
            component,
            implementation: binding.implementation(),
            descriptor: binding.descriptor(),
            root,
        })
    }

    /// Returns whether a logical identity and descriptor slot have equal relative paths.
    fn matches_relative_path(
        &self,
        logical: &super::StablePath,
        descriptor_slot: &super::StablePath,
    ) -> bool {
        let Some(logical) = logical
            .segments()
            .strip_prefix(self.component.path().segments())
        else {
            return false;
        };
        let Some(descriptor_slot) = descriptor_slot
            .segments()
            .strip_prefix(self.root.id.path().segments())
        else {
            return false;
        };
        logical == descriptor_slot
    }

    /// Resolves the descriptor reactor slot for one logical reactor.
    fn reactor_slot(&self, logical: &super::ReactorId) -> Result<ReactorSlotId, CompileError> {
        let matches = self
            .descriptor
            .reactor_slots()
            .iter()
            .filter(|slot| self.matches_relative_path(logical.path(), slot.id.path()))
            .map(|slot| slot.id.clone())
            .collect::<Vec<_>>();
        self.one_slot("reactor", logical.path(), matches)
    }

    /// Resolves the descriptor reaction slot for one logical reaction.
    fn reaction_slot(&self, logical: &super::ReactionId) -> Result<ReactionSlotId, CompileError> {
        let matches = self
            .descriptor
            .reaction_slots()
            .iter()
            .filter(|slot| self.matches_relative_path(logical.path(), slot.id.path()))
            .map(|slot| slot.id.clone())
            .collect::<Vec<_>>();
        self.one_slot("reaction", logical.path(), matches)
    }

    /// Resolves the descriptor port slot for one logical port.
    fn port_slot(
        &self,
        logical: &super::PortId,
        bank: Option<super::BankMember>,
    ) -> Result<PortSlotId, CompileError> {
        let declaration = match bank {
            Some(_) => logical
                .path()
                .parent()
                .expect("validated bank member has a base"),
            None => logical.path().clone(),
        };
        self.one_slot(
            "port",
            logical.path(),
            self.descriptor
                .port_slots()
                .iter()
                .filter(|slot| self.matches_relative_path(&declaration, slot.id.path()))
                .map(|slot| slot.id.clone())
                .collect(),
        )
    }

    /// Resolves the descriptor action slot for one logical action.
    fn action_slot(&self, logical: &super::ActionId) -> Result<ActionSlotId, CompileError> {
        self.one_slot(
            "action",
            logical.path(),
            self.descriptor
                .action_slots()
                .iter()
                .filter(|slot| self.matches_relative_path(logical.path(), slot.id.path()))
                .map(|slot| slot.id.clone())
                .collect(),
        )
    }

    /// Converts a matching descriptor-slot set into one required binding slot.
    fn one_slot<T>(
        &self,
        kind: &'static str,
        logical: &super::StablePath,
        matches: Vec<T>,
    ) -> Result<T, CompileError> {
        match matches.len() {
            0 => Err(CompileError::MissingDescriptorSlot {
                component: self.component.clone(),
                implementation: self.implementation.clone(),
                kind,
                logical: logical.to_string(),
            }),
            1 => Ok(matches
                .into_iter()
                .next()
                .expect("one matching descriptor slot")),
            candidates => Err(CompileError::AmbiguousDescriptorSlot {
                component: self.component.clone(),
                implementation: self.implementation.clone(),
                kind,
                logical: logical.to_string(),
                candidates,
            }),
        }
    }
}

/// Lowers a resolved local deployment into canonical immutable compiled images.
pub fn lower(deployment: &ResolvedDeployment) -> Result<OwnedCompiledDeployment, CompileError> {
    if !matches!(
        deployment.coordination(),
        super::CoordinationSelection::Local
    ) {
        return Err(CompileError::UnsupportedCoordination);
    }
    let mut federates = deployment.federates();
    let federate = federates
        .next()
        .expect("resolved local deployment has one Federate");
    if federates.next().is_some() {
        return Err(CompileError::UnsupportedCoordination);
    }
    let analysis = analyze(deployment)?;
    let mut enclaves = deployment
        .topology()
        .enclaves()
        .filter(|(_, enclave)| {
            let root = deployment
                .topology()
                .reactor(enclave.root())
                .expect("validated Enclave root exists");
            let group = root
                .placement_group()
                .expect("resolved Enclave root is placed");
            deployment
                .placement(group)
                .expect("resolved placement group is assigned")
                .federate()
                == federate.id()
        })
        .collect::<Vec<_>>();
    sort_by_encoded_identity(&mut enclaves);
    let enclaves = enclaves
        .into_iter()
        .map(|(id, _)| lower_enclave(deployment, id, &analysis))
        .collect::<Result<Box<[_]>, _>>()?;
    let owned_federate = OwnedFederateImage {
        id: federate.id().clone(),
        target: federate.target().clone(),
        runtime: federate.runtime().clone(),
        enclaves,
    };
    let compiled = OwnedCompiledDeployment {
        federation: GlobalFederationImage {
            members: vec![federate.id().clone()].into_boxed_slice(),
        },
        federates: vec![owned_federate].into_boxed_slice(),
        coordination: CoordinationProjection::Local,
    };
    validate_root_image(&compiled)?;
    Ok(compiled)
}

/// Computes canonical port equivalence and reaction levels over the complete deployment.
fn analyze(deployment: &ResolvedDeployment) -> Result<GlobalAnalysis, CompileError> {
    let topology = deployment.topology();
    let port_representatives = canonical_port_representatives(topology);
    let mut levels = BTreeMap::new();
    for (enclave, _) in topology.enclaves() {
        let reactions = topology
            .reactions()
            .filter(|(_, reaction)| {
                topology
                    .reactor(reaction.reactor())
                    .is_some_and(|reactor| reactor.enclave() == enclave)
            })
            .collect::<Vec<_>>();
        levels.extend(reaction_levels(
            topology,
            enclave,
            &reactions,
            &port_representatives,
        )?);
    }
    Ok(GlobalAnalysis {
        port_representatives,
        reaction_levels: levels,
    })
}

/// Lowers one canonically ordered Enclave slice using deployment-wide analysis.
fn lower_enclave(
    deployment: &ResolvedDeployment,
    enclave_id: &super::StableEnclaveId,
    analysis: &GlobalAnalysis,
) -> Result<OwnedEnclaveImage, CompileError> {
    let topology = deployment.topology();
    let mut reactors = topology
        .reactors()
        .filter(|(_, reactor)| reactor.enclave() == enclave_id)
        .collect::<Vec<_>>();
    sort_by_encoded_identity(&mut reactors);
    checked_u32(reactors.len(), enclave_id, "reactors")?;
    let reactor_indices = reactors
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| ((*id).clone(), ReactorIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let mut actions = topology
        .actions()
        .filter(|(_, action)| reactor_indices.contains_key(action.reactor()))
        .collect::<Vec<_>>();
    sort_by_encoded_identity(&mut actions);
    checked_u32(actions.len(), enclave_id, "actions")?;
    let action_indices = actions
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| ((*id).clone(), ActionIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let modes = reactors
        .iter()
        .flat_map(|(reactor_id, _)| {
            let mut modes = topology
                .modes()
                .filter(move |(_, mode)| mode.reactor() == *reactor_id)
                .collect::<Vec<_>>();
            sort_by_encoded_identity(&mut modes);
            modes
        })
        .collect::<Vec<_>>();
    let reactor_count = checked_u32(reactors.len(), enclave_id, "scopes")?;
    checked_u32(reactors.len() + modes.len(), enclave_id, "scopes")?;
    let mode_indices = modes
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| ((*id).clone(), ModeIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let root_scopes = reactors
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| ((*id).clone(), ScopeIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let mode_scopes = modes
        .iter()
        .zip(reactor_count..)
        .map(|((id, _), index)| ((*id).clone(), ScopeIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let mut local_ports = topology
        .ports()
        .filter(|(_, port)| reactor_indices.contains_key(port.reactor()))
        .collect::<Vec<_>>();
    sort_by_encoded_identity(&mut local_ports);
    let port_representatives = local_ports
        .iter()
        .map(|(id, _)| ((*id).clone(), analysis.port_representatives[*id].clone()))
        .collect::<BTreeMap<_, _>>();
    let mut representatives = port_representatives.values().cloned().collect::<Vec<_>>();
    representatives.sort_by_cached_key(canonical_identity_text);
    representatives.dedup();
    checked_u32(representatives.len(), enclave_id, "ports")?;
    let representative_indices = representatives
        .iter()
        .zip(0u32..)
        .map(|(id, index)| (id.clone(), PortIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let port_indices = port_representatives
        .iter()
        .map(|(id, representative)| (id.clone(), representative_indices[representative]))
        .collect::<BTreeMap<_, _>>();
    let mut identity_data = String::new();
    let enclave_range = push_identity(&mut identity_data, &enclave_id.to_string(), enclave_id)?;
    let mut reactions = topology
        .reactions()
        .filter(|(_, reaction)| reactor_indices.contains_key(reaction.reactor()))
        .collect::<Vec<_>>();
    sort_by_encoded_identity(&mut reactions);
    checked_u32(reactions.len(), enclave_id, "reactions")?;
    let reaction_indices = reactions
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| ((*id).clone(), ReactionIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let mut named_bindings = reactors
        .iter()
        .map(|(id, reactor)| {
            let slots = DescriptorSlots::for_component(deployment, reactor.component())?;
            Ok((
                format!("state/{id}"),
                RequiredBinding::State {
                    component: reactor.component().clone(),
                    implementation: slots.implementation.clone(),
                    reactor: slots.reactor_slot(id)?,
                },
            ))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    named_bindings.extend(
        reactions
            .iter()
            .map(|(id, reaction)| {
                let component = topology
                    .reactor(reaction.reactor())
                    .expect("validated reaction reactor exists")
                    .component();
                let slots = DescriptorSlots::for_component(deployment, component)?;
                Ok((
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
    named_bindings.extend(
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
    named_bindings.extend(
        actions
            .iter()
            .filter(|(_, action)| {
                matches!(
                    action.kind(),
                    super::ActionKind::Logical { .. } | super::ActionKind::Physical { .. }
                )
            })
            .map(|(id, action)| {
                let component = topology
                    .reactor(action.reactor())
                    .expect("validated action reactor exists")
                    .component();
                let slots = DescriptorSlots::for_component(deployment, component)?;
                Ok((
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
    named_bindings.sort_by(|left, right| left.0.cmp(&right.0));
    let binding_entries = named_bindings
        .iter()
        .map(|(_, binding)| binding.clone())
        .collect::<Vec<_>>();
    checked_u32(binding_entries.len(), enclave_id, "bindings")?;
    let binding_indices = named_bindings
        .iter()
        .zip(0u32..)
        .map(|((id, _), index)| (id.clone(), BindingSlotIndex::new(index)))
        .collect::<BTreeMap<_, _>>();
    let binding_images = named_bindings
        .iter()
        .map(|(id, binding)| {
            let range = push_identity(&mut identity_data, id, enclave_id)?;
            Ok(RequiredBindingImage::new(range, binding.kind()))
        })
        .collect::<Result<tinymap::TinyMap<BindingSlotIndex, _>, CompileError>>()?;
    let scope_for = |reactor: &super::ReactorId, mode: Option<&super::ModeId>| {
        mode.map_or(root_scopes[reactor], |mode| mode_scopes[mode])
    };
    let mut mode_cursor = 0u32;
    let reactor_images = reactors
        .iter()
        .zip(0u32..)
        .map(|((id, reactor), index)| {
            let reactor_modes = modes
                .iter()
                .filter(|(_, mode)| mode.reactor() == *id)
                .collect::<Vec<_>>();
            let mode_count = checked_u32(reactor_modes.len(), enclave_id, "modes")?;
            let image = ReactorImage::new(
                binding_indices[&format!("state/{id}")],
                StateSlotIndex::new(index),
                root_scopes[*id],
                TableRange::new(mode_cursor, mode_count),
                reactor_modes
                    .iter()
                    .find(|(_, mode)| mode.parent().is_none() && mode.is_initial())
                    .map(|(id, _)| mode_indices[*id]),
                reactor.bank().map(|bank| {
                    crate::runtime::image::BankInfoImage::new(bank.index(), bank.total())
                }),
            );
            mode_cursor += mode_count;
            Ok(image)
        })
        .collect::<Result<tinymap::TinyMap<ReactorIndex, _>, CompileError>>()?;
    let mut flattened_triggers = Vec::new();
    let mut action_triggers = (0..actions.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<ActionIndex, Vec<LevelReactionImage>>>();
    for (id, reaction) in &reactions {
        for relation in reaction.relations() {
            if !relation.flags().is_trigger() {
                continue;
            }
            if let super::ReactionRelationTarget::Action(action) = relation.target() {
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
    let action_images = actions
        .iter()
        .zip(action_triggers.values())
        .enumerate()
        .map(|(index, ((id, action), triggers))| {
            let start = checked_u32(flattened_triggers.len(), enclave_id, "reaction-triggers")?;
            flattened_triggers.extend_from_slice(triggers);
            let len = checked_u32(triggers.len(), enclave_id, "reaction-triggers")?;
            let timing = match action.kind() {
                super::ActionKind::Logical { minimum_delay } => ActionTiming::Standard {
                    domain: TimingDomain::Logical,
                    min_delay_nanos: duration_nanos(minimum_delay, enclave_id)?,
                },
                super::ActionKind::Physical { minimum_delay } => ActionTiming::Standard {
                    domain: TimingDomain::Physical,
                    min_delay_nanos: duration_nanos(minimum_delay, enclave_id)?,
                },
                super::ActionKind::Timer { period, .. } => ActionTiming::Timer {
                    period_nanos: period
                        .map(|period| duration_nanos(Some(period), enclave_id))
                        .transpose()?,
                },
                super::ActionKind::Startup => ActionTiming::Timer { period_nanos: None },
                super::ActionKind::Shutdown => ActionTiming::Shutdown,
            };
            Ok(ActionImage::new(
                scope_for(action.reactor(), action.mode()),
                ActionSlotIndex::new(checked_u32(index, enclave_id, "actions")?),
                timing,
                TableRange::new(start, len),
                matches!(
                    action.kind(),
                    super::ActionKind::Logical { .. } | super::ActionKind::Physical { .. }
                )
                .then(|| binding_indices[&format!("action/{id}")]),
            ))
        })
        .collect::<Result<tinymap::TinyMap<ActionIndex, _>, CompileError>>()?;
    let mut port_triggers = (0..representatives.len())
        .map(|_| Vec::new())
        .collect::<tinymap::TinyMap<PortIndex, Vec<LevelReactionImage>>>();
    for (id, reaction) in &reactions {
        for relation in reaction.relations() {
            if relation.flags().is_trigger() {
                if let super::ReactionRelationTarget::Port(port) = relation.target() {
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
            let start = checked_u32(flattened_triggers.len(), enclave_id, "reaction-triggers")?;
            flattened_triggers.extend_from_slice(triggers);
            let len = checked_u32(triggers.len(), enclave_id, "reaction-triggers")?;
            Ok(PortImage::new(
                scope_for(port.reactor(), port.mode()),
                TableRange::new(start, len),
                binding_indices[&format!("port/{id}")],
            ))
        })
        .collect::<Result<tinymap::TinyMap<PortIndex, _>, CompileError>>()?;
    let mut use_ports = Vec::new();
    let mut effect_ports = Vec::new();
    let mut reaction_actions = Vec::new();
    let mut reaction_modes = Vec::new();
    let reaction_images = reactions
        .iter()
        .map(|(id, reaction)| {
            let use_start = use_ports.len();
            let effect_start = effect_ports.len();
            let action_start = reaction_actions.len();
            let mode_start = reaction_modes.len();
            for relation in reaction.relations() {
                match relation.target() {
                    super::ReactionRelationTarget::Port(port) => {
                        if relation.flags().is_use()
                            && !use_ports[use_start..].contains(&port_indices[port])
                        {
                            use_ports.push(port_indices[port]);
                        }
                        if relation.flags().is_effect()
                            && !effect_ports[effect_start..].contains(&port_indices[port])
                        {
                            effect_ports.push(port_indices[port]);
                        }
                    }
                    super::ReactionRelationTarget::Action(action) => {
                        if relation.flags().is_use() || relation.flags().is_effect() {
                            reaction_actions.push(action_indices[action]);
                        }
                    }
                }
            }
            reaction_modes.extend(
                reaction
                    .options()
                    .enabled_modes()
                    .iter()
                    .map(|mode| mode_indices[mode]),
            );
            let image = ReactionImage::new(
                reactor_indices[reaction.reactor()],
                scope_for(reaction.reactor(), reaction.options().mode()),
                analysis.reaction_levels[*id],
                binding_indices[&format!("reaction/{id}")],
                checked_range(use_start, use_ports.len(), enclave_id, "reaction-use-ports")?,
                checked_range(
                    effect_start,
                    effect_ports.len(),
                    enclave_id,
                    "reaction-effect-ports",
                )?,
                checked_range(
                    action_start,
                    reaction_actions.len(),
                    enclave_id,
                    "reaction-actions",
                )?,
                checked_range(
                    mode_start,
                    reaction_modes.len(),
                    enclave_id,
                    "reaction-modes",
                )?,
            );
            Ok(reaction.options().transition().map_or(image, |transition| {
                image.with_mode_effect(crate::runtime::CompiledModeEffectRef {
                    target: mode_indices[transition.target()],
                    transition: match transition.kind() {
                        super::ModeTransitionKind::Reset => crate::runtime::TransitionKind::Reset,
                        super::ModeTransitionKind::History => {
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
    for (_, reactor) in &reactors {
        scope_parents.insert(reactor.parent().and_then(|parent| {
            root_scopes
                .get(parent)
                .map(|root| reactor.scope_mode().map_or(*root, |mode| mode_scopes[mode]))
        }));
    }
    for (_, mode) in &modes {
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
    let level_reaction = |reaction: &super::ReactionId| {
        LevelReactionImage::new(
            analysis.reaction_levels[reaction],
            reaction_indices[reaction],
        )
    };
    let mut startup_actions = Vec::new();
    let mut timer_startup_actions = Vec::new();
    for (id, action) in &actions {
        match action.kind() {
            super::ActionKind::Startup => {
                startup_actions.push(TimerStartupImage::new(action_indices[*id], 0));
            }
            super::ActionKind::Timer { offset, .. } => {
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
            let super::ReactionRelationTarget::Action(action_id) = relation.target() else {
                continue;
            };
            let entry = LifecycleReactionImage::new(level_reaction(id), action_indices[action_id]);
            match topology
                .action(action_id)
                .expect("reaction action exists")
                .kind()
            {
                super::ActionKind::Startup => startup_by_scope[scope].push(entry),
                super::ActionKind::Shutdown => shutdown_by_scope[scope].push(entry),
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
    let mut scope_descendants = Vec::new();
    let mut scope_logical_actions = Vec::new();
    let mut scope_timer_startups = Vec::new();
    let mut scope_reset_reactions = Vec::new();
    let mut scope_startup_reactions = Vec::new();
    let mut scope_shutdown_reactions = Vec::new();
    let scope_images = scope_parents
        .iter()
        .enumerate()
        .map(|(position, (scope, parent))| {
            let descendants = push_range(
                &mut scope_descendants,
                scope_parents
                    .keys()
                    .filter(|candidate| is_descendant(*candidate, scope)),
                enclave_id,
                "scope-descendants",
            )?;
            let logical_actions = push_range(
                &mut scope_logical_actions,
                actions
                    .iter()
                    .zip(action_scopes.values().copied())
                    .filter(|((_, action), action_scope)| {
                        !matches!(action.kind(), super::ActionKind::Physical { .. })
                            && is_descendant(*action_scope, scope)
                    })
                    .map(|((id, _), _)| action_indices[*id]),
                enclave_id,
                "scope-logical-actions",
            )?;
            let timer_startups = push_range(
                &mut scope_timer_startups,
                timer_startup_actions
                    .iter()
                    .copied()
                    .filter(|entry| is_descendant(action_scopes[entry.action()], scope)),
                enclave_id,
                "scope-timer-startups",
            )?;
            let reset_reactions = push_range(
                &mut scope_reset_reactions,
                {
                    let mut values = reset_by_scope
                        .iter()
                        .filter(|(candidate, _)| is_descendant(*candidate, scope))
                        .flat_map(|(_, values)| values.iter().copied())
                        .collect::<Vec<_>>();
                    values.sort_unstable();
                    values.dedup();
                    values
                },
                enclave_id,
                "scope-reset-reactions",
            )?;
            let startup_reactions = push_range(
                &mut scope_startup_reactions,
                startup_by_scope[scope].iter().copied(),
                enclave_id,
                "scope-startup-reactions",
            )?;
            let shutdown_reactions = push_range(
                &mut scope_shutdown_reactions,
                shutdown_by_scope[scope].iter().copied(),
                enclave_id,
                "scope-shutdown-reactions",
            )?;
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
            super::ConnectionSemantics::Logical { after } => (
                TimingDomain::Logical,
                duration_nanos(after, enclave_id)?,
                after.is_some_and(|delay| delay > crate::runtime::Duration::ZERO)
                    || source_enclave != target_enclave,
            ),
            super::ConnectionSemantics::Physical { after } => (
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
    checked_u32(routes.len(), enclave_id, "routes")?;
    let route_images = routes
        .into_iter()
        .map(|(boundary, local_port, direction, timing, delay)| {
            let range = push_identity(&mut identity_data, &boundary.to_string(), enclave_id)?;
            Ok(RouteImage::new(range, local_port, direction, timing, delay))
        })
        .collect::<Result<tinymap::TinyMap<crate::runtime::image::RouteIndex, _>, CompileError>>(
        )?;
    let storage_bounds = storage_bounds(deployment, enclave_id, &reactors, actions.len())?;
    let owned = OwnedEnclaveImage {
        id: enclave_id.clone(),
        identity_data: identity_data.into_boxed_str(),
        enclave_id: enclave_range,
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
        routes: route_images,
        binding_images,
        required_bindings: RequiredBindings {
            entries: binding_entries.into_boxed_slice(),
        },
        storage_bounds,
    };
    owned.view().map_err(|error| CompileError::InvalidImage {
        enclave: enclave_id.clone(),
        message: error.to_string(),
    })?;
    Ok(owned)
}

/// Selects the smallest stable identity for each direct local port equivalence class.
fn canonical_port_representatives(
    topology: &super::ApplicationTopology,
) -> BTreeMap<super::PortId, super::PortId> {
    let mut parents = topology
        .ports()
        .map(|(id, _)| (id.clone(), id.clone()))
        .collect::<BTreeMap<_, _>>();
    for (_, connection) in topology.connections() {
        let source_enclave = topology
            .port(connection.source())
            .and_then(|port| topology.reactor(port.reactor()))
            .map(|reactor| reactor.enclave());
        let target_enclave = topology
            .port(connection.target())
            .and_then(|port| topology.reactor(port.reactor()))
            .map(|reactor| reactor.enclave());
        let zero_delay = matches!(
            connection.semantics(),
            super::ConnectionSemantics::Logical { after }
                if !after.is_some_and(|delay| delay > crate::runtime::Duration::ZERO)
        );
        if !zero_delay || source_enclave != target_enclave {
            continue;
        }
        let source = port_root(&parents, connection.source());
        let target = port_root(&parents, connection.target());
        let (representative, other) =
            if canonical_identity_text(&source) <= canonical_identity_text(&target) {
                (source, target)
            } else {
                (target, source)
            };
        parents.insert(other, representative);
    }
    let keys = parents.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        let root = port_root(&parents, &key);
        parents.insert(key, root);
    }
    parents
}

/// Computes stable-tie-broken longest-path reaction levels within one Enclave.
fn reaction_levels(
    topology: &super::ApplicationTopology,
    enclave: &super::StableEnclaveId,
    reactions: &[(&super::ReactionId, &super::Reaction)],
    ports: &BTreeMap<super::PortId, super::PortId>,
) -> Result<BTreeMap<super::ReactionId, u32>, CompileError> {
    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum Target {
        /// Stable action dependency identity.
        Action(super::ActionId),
        /// Canonical local port dependency identity.
        Port(super::PortId),
    }
    let mut graph = petgraph::prelude::StableDiGraph::<&super::ReactionId, ()>::new();
    let nodes = reactions
        .iter()
        .map(|(id, _)| (*id, graph.add_node(*id)))
        .collect::<BTreeMap<_, _>>();
    let reactions_by_id = reactions.iter().copied().collect::<BTreeMap<_, _>>();
    let mut producers = BTreeMap::<Target, Vec<_>>::new();
    let mut consumers = BTreeMap::<Target, Vec<_>>::new();
    for (id, reaction) in reactions {
        for relation in reaction.relations() {
            let target = match relation.target() {
                super::ReactionRelationTarget::Action(action) => Target::Action(action.clone()),
                super::ReactionRelationTarget::Port(port) => Target::Port(ports[port].clone()),
            };
            if relation.flags().is_effect() {
                producers
                    .entry(target.clone())
                    .or_default()
                    .push(nodes[*id]);
            }
            if relation.flags().is_trigger() {
                consumers.entry(target).or_default().push(nodes[*id]);
            }
        }
    }
    for (target, sources) in producers {
        for source in sources {
            for consumer in consumers.get(&target).into_iter().flatten() {
                if !reactions_are_mutually_exclusive(
                    topology,
                    reactions_by_id[graph[source]],
                    reactions_by_id[graph[*consumer]],
                ) {
                    graph.update_edge(source, *consumer, ());
                }
            }
        }
    }
    let order = match petgraph::algo::toposort(&graph, None) {
        Ok(order) => order,
        Err(_) => {
            let mut blocked = BTreeSet::new();
            for component in petgraph::algo::kosaraju_scc(&graph) {
                let cyclic =
                    component.len() > 1 || graph.find_edge(component[0], component[0]).is_some();
                if cyclic {
                    blocked.extend(component.into_iter().map(|node| graph[node].clone()));
                }
            }
            debug_assert!(!blocked.is_empty());
            let mut reactions = blocked.into_iter().collect::<Vec<_>>();
            reactions.sort_by_cached_key(canonical_identity_text);
            return Err(CompileError::ReactionCycle {
                enclave: enclave.clone(),
                reactions: reactions.into_boxed_slice(),
            });
        }
    };
    let mut levels = graph
        .node_weights()
        .map(|id| ((*id).clone(), 0u32))
        .collect::<BTreeMap<_, _>>();
    for source in order {
        let next_level = levels[graph[source]] + 1;
        for target in graph.neighbors(source) {
            levels
                .entry(graph[target].clone())
                .and_modify(|level| *level = (*level).max(next_level));
        }
    }
    Ok(levels)
}

/// Reports whether two reactions are enclosed by distinct sibling modes.
fn reactions_are_mutually_exclusive(
    topology: &super::ApplicationTopology,
    left: &super::Reaction,
    right: &super::Reaction,
) -> bool {
    let left_modes = enclosing_modes(topology, left);
    let right_modes = enclosing_modes(topology, right);
    left_modes.iter().any(|left_mode| {
        right_modes.iter().any(|right_mode| {
            left_mode != right_mode
                && topology.mode(left_mode).map(super::Mode::reactor)
                    == topology.mode(right_mode).map(super::Mode::reactor)
        })
    })
}

/// Collects a reaction's direct and structurally inherited mode scopes.
fn enclosing_modes(
    topology: &super::ApplicationTopology,
    reaction: &super::Reaction,
) -> Vec<super::ModeId> {
    let mut modes = reaction
        .options()
        .mode()
        .cloned()
        .into_iter()
        .collect::<Vec<_>>();
    let mut reactor = topology.reactor(reaction.reactor());
    while let Some(current) = reactor {
        modes.extend(current.scope_mode().cloned());
        reactor = current.parent().and_then(|parent| topology.reactor(parent));
    }
    modes
}

/// Follows one union-find chain to its canonical stable representative.
fn port_root(
    parents: &BTreeMap<super::PortId, super::PortId>,
    port: &super::PortId,
) -> super::PortId {
    let mut root = port;
    while &parents[root] != root {
        root = &parents[root];
    }
    root.clone()
}

/// Converts an optional duration into a checked non-negative nanosecond value.
fn duration_nanos(
    duration: Option<crate::runtime::Duration>,
    enclave: &super::StableEnclaveId,
) -> Result<u64, CompileError> {
    duration.map_or(Ok(0), |duration| {
        u64::try_from(duration.whole_nanoseconds()).map_err(|_| CompileError::ResourceOverflow {
            enclave: enclave.clone(),
            resource: "connection-delay",
        })
    })
}

/// Aggregates component-wide bounds for every component touching one Enclave.
fn storage_bounds(
    deployment: &ResolvedDeployment,
    enclave: &super::StableEnclaveId,
    reactors: &[(&super::ReactorId, &super::Reactor)],
    action_count: usize,
) -> Result<StorageBounds, CompileError> {
    let components = reactors
        .iter()
        .map(|(_, reactor)| reactor.component().clone())
        .collect::<BTreeSet<_>>();
    let mut queue = 0u64;
    let mut payload = 0u64;
    let mut state = 0u64;
    let mut scratch = 0u64;
    for component in components {
        let bounds = deployment
            .binding(&component)
            .expect("resolved component has an implementation binding")
            .descriptor()
            .bounds();
        queue = checked_bound(
            queue,
            bounds.queue_capacity,
            "event-queue",
            &component,
            enclave,
        )?;
        payload = checked_bound(
            payload,
            bounds.payload_bytes,
            "payload-bytes",
            &component,
            enclave,
        )?;
        state = checked_bound(
            state,
            bounds.state_bytes,
            "state-bytes",
            &component,
            enclave,
        )?;
        scratch = checked_bound(
            scratch,
            bounds.scratch_bytes,
            "scratch-bytes",
            &component,
            enclave,
        )?;
    }
    Ok(StorageBounds::new(
        u32::try_from(reactors.len()).map_err(|_| CompileError::ResourceOverflow {
            enclave: enclave.clone(),
            resource: "state-slots",
        })?,
        u32::try_from(action_count).map_err(|_| CompileError::ResourceOverflow {
            enclave: enclave.clone(),
            resource: "action-slots",
        })?,
        u32::try_from(queue).map_err(|_| CompileError::ResourceOverflow {
            enclave: enclave.clone(),
            resource: "event-queue",
        })?,
        payload,
        state,
        scratch,
    ))
}

/// Adds one declared bound or reports the missing/overflowing resource identity.
fn checked_bound(
    total: u64,
    bound: DescriptorBound,
    resource: &'static str,
    component: &super::ComponentInstanceId,
    enclave: &super::StableEnclaveId,
) -> Result<u64, CompileError> {
    let DescriptorBound::Known(value) = bound else {
        return Err(CompileError::UnboundedResource {
            component: component.clone(),
            enclave: enclave.clone(),
            resource,
        });
    };
    total
        .checked_add(value)
        .ok_or_else(|| CompileError::ResourceOverflow {
            enclave: enclave.clone(),
            resource,
        })
}

/// Converts one dense table cardinality without truncation.
fn checked_u32(
    value: usize,
    enclave: &super::StableEnclaveId,
    resource: &'static str,
) -> Result<u32, CompileError> {
    u32::try_from(value).map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave.clone(),
        resource,
    })
}

/// Builds a typed dense range from checked platform-sized offsets.
fn checked_range<T>(
    start: usize,
    end: usize,
    enclave: &super::StableEnclaveId,
    resource: &'static str,
) -> Result<TableRange<T>, CompileError> {
    Ok(TableRange::new(
        checked_u32(start, enclave, resource)?,
        checked_u32(end - start, enclave, resource)?,
    ))
}

/// Orders stable-identity records by their canonical encoded text.
fn sort_by_encoded_identity<I: std::fmt::Display, T>(values: &mut [(&I, &T)]) {
    values.sort_by_cached_key(|(identity, _)| canonical_identity_text(*identity));
}

/// Appends a stable identity to the image blob and returns its checked byte range.
fn push_identity(
    data: &mut String,
    value: &str,
    enclave: &super::StableEnclaveId,
) -> Result<IdentityRange, CompileError> {
    let start = u32::try_from(data.len()).map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave.clone(),
        resource: "identity-bytes",
    })?;
    let len = u32::try_from(value.len()).map_err(|_| CompileError::ResourceOverflow {
        enclave: enclave.clone(),
        resource: "identity-bytes",
    })?;
    data.push_str(value);
    Ok(IdentityRange::new(start, len))
}

/// Appends a deterministic flattened table segment and returns its checked range.
fn push_range<T>(
    target: &mut Vec<T>,
    values: impl IntoIterator<Item = T>,
    enclave: &super::StableEnclaveId,
    resource: &'static str,
) -> Result<TableRange<T>, CompileError> {
    let start = target.len();
    target.extend(values);
    Ok(TableRange::new(
        checked_u32(start, enclave, resource)?,
        checked_u32(target.len() - start, enclave, resource)?,
    ))
}

/// Returns the canonical secondary ordering for a route pair.
const fn route_direction_rank(direction: RouteDirection) -> u8 {
    match direction {
        RouteDirection::Inbound => 0,
        RouteDirection::Outbound => 1,
    }
}

/// Constructs and validates the borrowed root deployment image before success escapes.
fn validate_root_image(compiled: &OwnedCompiledDeployment) -> Result<(), CompileError> {
    let mut identity_data = String::new();
    let checked_root = |value| {
        u32::try_from(value).map_err(|_| CompileError::InvalidDeployment {
            message: "dense deployment table exceeds u32".to_owned(),
        })
    };
    let mut push_root_identity = |value: &str| {
        let start =
            u32::try_from(identity_data.len()).map_err(|_| CompileError::InvalidDeployment {
                message: "identity bytes exceed u32".to_owned(),
            })?;
        let len = u32::try_from(value.len()).map_err(|_| CompileError::InvalidDeployment {
            message: "identity bytes exceed u32".to_owned(),
        })?;
        identity_data.push_str(value);
        Ok::<_, CompileError>(IdentityRange::new(start, len))
    };
    let mut federates = Vec::new();
    let mut enclave_images = Vec::new();
    for federate in &compiled.federates {
        let id = push_root_identity(federate.id.as_str())?;
        let target = push_root_identity(federate.target.as_str())?;
        let runtime = push_root_identity(federate.runtime.as_str())?;
        let start = enclave_images.len();
        enclave_images.extend(federate.enclaves.iter().map(OwnedEnclaveImage::image));
        federates.push(FederateImage::new(
            id,
            target,
            runtime,
            TableRange::<EnclaveIndex>::new(
                checked_root(start)?,
                checked_root(enclave_images.len() - start)?,
            ),
        ));
    }
    let members = (0..federates.len())
        .map(|index| checked_root(index).map(FederateIndex::new))
        .collect::<Result<Vec<_>, CompileError>>()?;
    checked_root(enclave_images.len())?;
    let federation = crate::runtime::image::GlobalFederationImage::new(&members, &[]);
    let image = CompiledDeploymentImage {
        identity_data: &identity_data,
        federation,
        federates: tinymap::TinyMapView::new(&federates),
        enclaves: tinymap::TinyMapView::new(&enclave_images),
        coordination: compiled.coordination,
    };
    CompiledDeploymentView::new(&image).map_err(|error| CompileError::InvalidDeployment {
        message: error.to_string(),
    })?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::{checked_u32, lower, CompileError};
    use crate::{
        compiler::{
            ActionId, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, BankMember,
            BoundaryBinding, BoundaryId, CodecCapabilityId, ComponentInstance, ComponentInstanceId,
            ConnectionSemantics, CoordinationBackendId, CoordinationSelection, FederateConfig,
            FederateId, ImplementationBinding, ImplementationId, ModeId, ModeTransition,
            ModeTransitionKind, PlacementAssignment, PlacementGroupId, PortDirection, PortId,
            ReactionId, ReactionOptions, ReactionRelation, ReactionRelationFlags,
            ReactionRelationTarget, Reactor, ReactorId, RequiredBinding, ResolvedDeployment,
            RuntimeBackendId, StableEnclaveId, TargetTriple, TransportCapabilityId,
        },
        descriptor::{
            ActionSlot, ActionSlotId, ComponentDescriptor, DescriptorBound, DescriptorBounds,
            PortSlot, PortSlotId, ReactionSlot, ReactionSlotId, ReactorSlot, ReactorSlotId,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
        },
        runtime::image::{
            ActionIndex, ActionTiming, BindingKind, ModeIndex, ReactionIndex, ReactorIndex,
            RouteDirection, RouteIndex, ScopeIndex, TimingDomain,
        },
    };
    fn descriptor(contract: &str, bounds: DescriptorBounds) -> ComponentDescriptor {
        let root = match contract {
            "controller.v1" => "Controller",
            "sensor.v1" => "Sensor",
            _ => panic!("unexpected test descriptor contract {contract}"),
        };
        descriptor_with_root(contract, bounds, root)
    }

    fn descriptor_with_root(
        contract: &str,
        bounds: DescriptorBounds,
        root: &str,
    ) -> ComponentDescriptor {
        let reactions = match contract {
            "controller.v1" => [
                "emit",
                "reset_active",
                "shutdown",
                "start",
                "timer_fired",
                "#g0",
                "#g1",
                "#g2",
                "#g3",
                "#g4",
                "#g5",
                "#g6",
                "#g7",
                "#g8",
                "#g9",
                "#g10",
                "%23generated%5B0%5D",
            ]
            .as_slice(),
            "sensor.v1" => ["receive"].as_slice(),
            _ => panic!("unexpected test descriptor contract {contract}"),
        };
        let reactor = ReactorSlotId::new(root).unwrap();
        let port = |name| PortSlot {
            id: PortSlotId::new(format!("{root}/{name}")).unwrap(),
            reactor: reactor.clone(),
            direction: if name == "output" {
                PortDirection::Output
            } else {
                PortDirection::Input
            },
        };
        let action = |name| ActionSlot {
            id: ActionSlotId::new(format!("{root}/{name}")).unwrap(),
            reactor: reactor.clone(),
        };
        let (ports, actions) = match contract {
            "controller.v1" => (
                vec![port("output"), port("array_in"), port("bank_in")],
                vec![action("pulse")],
            ),
            "sensor.v1" => (vec![port("input")], vec![action("ack")]),
            _ => unreachable!(),
        };
        ComponentDescriptor::try_new(
            contract.parse().unwrap(),
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            vec![ReactorSlot {
                id: reactor.clone(),
                parent: None,
            }],
            ports,
            actions,
            reactions
                .iter()
                .map(|reaction| ReactionSlot {
                    id: ReactionSlotId::new(format!("{root}/{reaction}")).unwrap(),
                    reactor: reactor.clone(),
                })
                .collect(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            bounds,
        )
        .unwrap()
    }
    fn ordered<T>(mut values: Vec<T>, reverse: bool) -> Vec<T> {
        if reverse {
            values.reverse();
        }
        values
    }
    #[derive(Clone, Copy)]
    enum DependencyCase {
        None,
        ReactionCycle,
        PortSelfCycle,
        MutuallyExclusiveModes,
        EncodedOrdering,
        ModeTransition,
    }
    fn topology(
        reverse: bool,
        shared_enclave: bool,
        semantics: ConnectionSemantics,
        dependency_case: DependencyCase,
    ) -> ApplicationTopology {
        let mut topology = ApplicationTopologyBuilder::new("vehicle").unwrap();
        let controller = ComponentInstanceId::new("vehicle/controller").unwrap();
        let sensor = ComponentInstanceId::new("vehicle/sensor").unwrap();
        let controller_reactor = ReactorId::new("vehicle/controller").unwrap();
        let sensor_reactor = ReactorId::new("vehicle/sensor").unwrap();
        let controller_enclave = StableEnclaveId::new("vehicle/controller").unwrap();
        let sensor_enclave = StableEnclaveId::new("vehicle/sensor").unwrap();
        let controller_group = PlacementGroupId::new("placement/controller").unwrap();
        let sensor_group = PlacementGroupId::new("placement/sensor").unwrap();
        for component in ordered(
            vec![
                ComponentInstance::new("vehicle/controller", "controller.v1", 1).unwrap(),
                ComponentInstance::new("vehicle/sensor", "sensor.v1", 1).unwrap(),
            ],
            reverse,
        ) {
            topology.add_component(component).unwrap();
        }
        for group in ordered(
            vec![controller_group.clone(), sensor_group.clone()],
            reverse,
        ) {
            topology.add_placement_group(group, None).unwrap();
        }
        for reactor in ordered(
            vec![
                Reactor::new(
                    controller_reactor.clone(),
                    controller.clone(),
                    None,
                    None,
                    controller_enclave.clone(),
                    Some(controller_group.clone()),
                    None,
                ),
                Reactor::new(
                    sensor_reactor.clone(),
                    sensor.clone(),
                    None,
                    None,
                    if shared_enclave {
                        controller_enclave.clone()
                    } else {
                        sensor_enclave.clone()
                    },
                    Some(sensor_group.clone()),
                    None,
                ),
            ],
            reverse,
        ) {
            topology.add_reactor(reactor).unwrap();
        }
        let mut enclaves = vec![(controller_enclave, controller_reactor.clone())];
        if !shared_enclave {
            enclaves.push((sensor_enclave, sensor_reactor.clone()));
        }
        for (id, root) in ordered(enclaves, reverse) {
            topology.add_enclave(id, root).unwrap();
        }
        let active = ModeId::new("vehicle/controller/active").unwrap();
        let idle = ModeId::new("vehicle/controller/idle").unwrap();
        for (id, initial) in ordered(vec![(active.clone(), false), (idle.clone(), true)], reverse) {
            topology
                .add_mode(id, controller_reactor.clone(), None, initial)
                .unwrap();
        }
        let pulse = ActionId::new("vehicle/controller/pulse").unwrap();
        let shutdown = ActionId::new("vehicle/controller/shutdown").unwrap();
        let startup = ActionId::new("vehicle/controller/startup").unwrap();
        let timer = ActionId::new("vehicle/controller/timer").unwrap();
        let ack = ActionId::new("vehicle/sensor/ack").unwrap();
        let ns = crate::runtime::Duration::nanoseconds;
        for (id, reactor, kind, position, mode) in ordered(
            vec![
                (
                    startup.clone(),
                    controller_reactor.clone(),
                    ActionKind::Startup,
                    0,
                    None,
                ),
                (
                    shutdown.clone(),
                    controller_reactor.clone(),
                    ActionKind::Shutdown,
                    1,
                    None,
                ),
                (
                    pulse.clone(),
                    controller_reactor.clone(),
                    ActionKind::Logical {
                        minimum_delay: Some(ns(3)),
                    },
                    2,
                    None,
                ),
                (
                    timer.clone(),
                    controller_reactor.clone(),
                    ActionKind::Timer {
                        offset: Some(ns(5)),
                        period: Some(ns(7)),
                    },
                    3,
                    Some(idle.clone()),
                ),
                (
                    ack.clone(),
                    sensor_reactor.clone(),
                    ActionKind::Physical {
                        minimum_delay: Some(ns(11)),
                    },
                    0,
                    None,
                ),
            ],
            reverse,
        ) {
            topology
                .add_action(id, reactor, kind, position, mode)
                .unwrap();
        }
        let output = PortId::new("vehicle/controller/output").unwrap();
        let input = PortId::new("vehicle/sensor/input").unwrap();
        for (id, reactor, direction) in ordered(
            vec![
                (
                    output.clone(),
                    controller_reactor.clone(),
                    PortDirection::Output,
                ),
                (input.clone(), sensor_reactor.clone(), PortDirection::Input),
            ],
            reverse,
        ) {
            topology
                .add_port(id, reactor, direction, None, 0, None)
                .unwrap();
        }
        for (position, name) in ["array_in", "array_in", "bank_in", "bank_in"]
            .into_iter()
            .enumerate()
        {
            let index = position as u32 % 2;
            topology
                .add_port(
                    PortId::new(format!("vehicle/controller/{name}/#b{index}")).unwrap(),
                    controller_reactor.clone(),
                    PortDirection::Input,
                    Some(BankMember::new(index, 2).unwrap()),
                    position as u32 + 1,
                    None,
                )
                .unwrap();
        }
        topology
            .add_connection(
                BoundaryId::new("controller-to-sensor").unwrap(),
                output,
                input,
                semantics,
            )
            .unwrap();
        let mut emit_relations = vec![
            ReactionRelation::new(
                ReactionRelationTarget::Action(pulse.clone()),
                ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                0,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Port(PortId::new("vehicle/controller/output").unwrap()),
                if matches!(dependency_case, DependencyCase::PortSelfCycle) {
                    ReactionRelationFlags::TRIGGER | ReactionRelationFlags::EFFECT
                } else {
                    ReactionRelationFlags::EFFECT
                },
                0,
            ),
        ];
        if matches!(dependency_case, DependencyCase::ReactionCycle) {
            emit_relations.insert(
                1,
                ReactionRelation::new(
                    ReactionRelationTarget::Action(startup.clone()),
                    ReactionRelationFlags::EFFECT,
                    1,
                ),
            );
        }
        let reset_active_relations =
            matches!(dependency_case, DependencyCase::MutuallyExclusiveModes)
                .then(|| {
                    vec![ReactionRelation::new(
                        ReactionRelationTarget::Action(timer.clone()),
                        ReactionRelationFlags::EFFECT,
                        0,
                    )]
                })
                .unwrap_or_default();
        let timer_relations = vec![ReactionRelation::new(
            ReactionRelationTarget::Action(timer),
            ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
            0,
        )];
        let mut reactions = vec![
            (
                ReactionId::new("vehicle/controller/emit").unwrap(),
                controller_reactor.clone(),
                emit_relations,
                ReactionOptions::default(),
            ),
            (
                ReactionId::new("vehicle/controller/reset_active").unwrap(),
                controller_reactor.clone(),
                reset_active_relations,
                ReactionOptions {
                    mode: Some(active.clone()),
                    enabled_modes: vec![active.clone()],
                    reset_modes: vec![active.clone()],
                    transition: matches!(dependency_case, DependencyCase::ModeTransition)
                        .then(|| ModeTransition::new(idle.clone(), ModeTransitionKind::Reset)),
                },
            ),
            (
                ReactionId::new("vehicle/controller/shutdown").unwrap(),
                controller_reactor.clone(),
                vec![ReactionRelation::new(
                    ReactionRelationTarget::Action(shutdown),
                    ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                    0,
                )],
                ReactionOptions::default(),
            ),
            (
                ReactionId::new("vehicle/controller/start").unwrap(),
                controller_reactor.clone(),
                vec![
                    ReactionRelation::new(
                        ReactionRelationTarget::Action(startup),
                        ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                        0,
                    ),
                    ReactionRelation::new(
                        ReactionRelationTarget::Action(pulse),
                        ReactionRelationFlags::EFFECT,
                        1,
                    ),
                ],
                ReactionOptions::default(),
            ),
            (
                ReactionId::new("vehicle/controller/timer_fired").unwrap(),
                controller_reactor,
                timer_relations,
                ReactionOptions {
                    mode: Some(idle.clone()),
                    enabled_modes: vec![idle],
                    reset_modes: vec![],
                    transition: None,
                },
            ),
            (
                ReactionId::new("vehicle/sensor/receive").unwrap(),
                sensor_reactor,
                vec![
                    ReactionRelation::new(
                        ReactionRelationTarget::Action(ack),
                        ReactionRelationFlags::EFFECT,
                        0,
                    ),
                    ReactionRelation::new(
                        ReactionRelationTarget::Port(PortId::new("vehicle/sensor/input").unwrap()),
                        ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                        0,
                    ),
                ],
                ReactionOptions::default(),
            ),
        ];
        if matches!(dependency_case, DependencyCase::EncodedOrdering) {
            reactions.extend((0..=10).map(|ordinal| {
                (
                    ReactionId::new(format!("vehicle/controller/#g{ordinal}")).unwrap(),
                    ReactorId::new("vehicle/controller").unwrap(),
                    vec![],
                    ReactionOptions::default(),
                )
            }));
            reactions.push((
                ReactionId::new("vehicle/controller/%23generated%5B0%5D").unwrap(),
                ReactorId::new("vehicle/controller").unwrap(),
                vec![],
                ReactionOptions::default(),
            ));
        }
        for (id, reactor, relations, options) in ordered(reactions, reverse) {
            topology
                .add_reaction(id, reactor, relations, options)
                .unwrap();
        }
        topology.finish().unwrap()
    }
    fn known_bounds() -> DescriptorBounds {
        DescriptorBounds {
            queue_capacity: DescriptorBound::Known(8),
            payload_bytes: DescriptorBound::Known(16),
            state_bytes: DescriptorBound::Known(32),
            scratch_bytes: DescriptorBound::Known(64),
        }
    }
    fn deployment_with_bounds(
        reverse: bool,
        distributed: bool,
        shared_enclave: bool,
        semantics: ConnectionSemantics,
        bounds: [DescriptorBounds; 2],
        dependency_case: DependencyCase,
    ) -> ResolvedDeployment {
        let mut bindings = vec![
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/controller").unwrap(),
                ImplementationId::new("controller-host").unwrap(),
                descriptor("controller.v1", bounds[0]),
            ),
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/sensor").unwrap(),
                ImplementationId::new("sensor-host").unwrap(),
                descriptor("sensor.v1", bounds[1]),
            ),
        ];
        let mut placements = vec![
            PlacementAssignment::new(
                PlacementGroupId::new("placement/controller").unwrap(),
                FederateId::new("host").unwrap(),
            ),
            PlacementAssignment::new(
                PlacementGroupId::new("placement/sensor").unwrap(),
                FederateId::new(if distributed { "edge" } else { "host" }).unwrap(),
            ),
        ];
        let mut federates = vec![FederateConfig::new(
            FederateId::new("host").unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("native").unwrap(),
        )];
        let mut boundary_bindings = vec![];
        if distributed {
            federates.push(FederateConfig::new(
                FederateId::new("edge").unwrap(),
                TargetTriple::new("aarch64-unknown-none").unwrap(),
                RuntimeBackendId::new("rtic").unwrap(),
            ));
            boundary_bindings.push(BoundaryBinding::new(
                BoundaryId::new("controller-to-sensor").unwrap(),
                CodecCapabilityId::new("postcard").unwrap(),
                TransportCapabilityId::new("udp").unwrap(),
            ));
        }
        if reverse {
            bindings.reverse();
            placements.reverse();
            federates.reverse();
            boundary_bindings.reverse();
        }
        ResolvedDeployment::new(
            topology(reverse, shared_enclave, semantics, dependency_case),
            bindings,
            placements,
            federates,
            if distributed {
                CoordinationSelection::Distributed {
                    backend: CoordinationBackendId::new("rti").unwrap(),
                }
            } else {
                CoordinationSelection::Local
            },
            boundary_bindings,
        )
        .unwrap()
    }
    fn deployment(reverse: bool, distributed: bool) -> ResolvedDeployment {
        deployment_with_bounds(
            reverse,
            distributed,
            false,
            ConnectionSemantics::Logical { after: None },
            [known_bounds(); 2],
            DependencyCase::None,
        )
    }
    fn local_deployment(
        shared_enclave: bool,
        semantics: ConnectionSemantics,
        dependency_case: DependencyCase,
    ) -> ResolvedDeployment {
        deployment_with_bounds(
            false,
            false,
            shared_enclave,
            semantics,
            [known_bounds(); 2],
            dependency_case,
        )
    }

    fn shared_implementation_deployment(reverse: bool) -> ResolvedDeployment {
        let mut bindings = vec![
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/controller").unwrap(),
                ImplementationId::new("shared-host").unwrap(),
                descriptor_with_root("controller.v1", known_bounds(), "Shared"),
            ),
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/sensor").unwrap(),
                ImplementationId::new("shared-host").unwrap(),
                descriptor_with_root("sensor.v1", known_bounds(), "Shared"),
            ),
        ];
        let mut placements = vec![
            PlacementAssignment::new(
                PlacementGroupId::new("placement/controller").unwrap(),
                FederateId::new("host").unwrap(),
            ),
            PlacementAssignment::new(
                PlacementGroupId::new("placement/sensor").unwrap(),
                FederateId::new("host").unwrap(),
            ),
        ];
        if reverse {
            bindings.reverse();
            placements.reverse();
        }
        ResolvedDeployment::new(
            topology(
                reverse,
                true,
                ConnectionSemantics::Logical { after: None },
                DependencyCase::None,
            ),
            bindings,
            placements,
            [FederateConfig::new(
                FederateId::new("host").unwrap(),
                TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                RuntimeBackendId::new("native").unwrap(),
            )],
            CoordinationSelection::Local,
            [],
        )
        .unwrap()
    }

    fn deployment_with_controller_descriptor(
        controller_descriptor: ComponentDescriptor,
    ) -> ResolvedDeployment {
        ResolvedDeployment::new(
            topology(
                false,
                false,
                ConnectionSemantics::Logical { after: None },
                DependencyCase::None,
            ),
            [
                ImplementationBinding::new(
                    ComponentInstanceId::new("vehicle/controller").unwrap(),
                    ImplementationId::new("controller-host").unwrap(),
                    controller_descriptor,
                ),
                ImplementationBinding::new(
                    ComponentInstanceId::new("vehicle/sensor").unwrap(),
                    ImplementationId::new("sensor-host").unwrap(),
                    descriptor("sensor.v1", known_bounds()),
                ),
            ],
            [
                PlacementAssignment::new(
                    PlacementGroupId::new("placement/controller").unwrap(),
                    FederateId::new("host").unwrap(),
                ),
                PlacementAssignment::new(
                    PlacementGroupId::new("placement/sensor").unwrap(),
                    FederateId::new("host").unwrap(),
                ),
            ],
            [FederateConfig::new(
                FederateId::new("host").unwrap(),
                TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                RuntimeBackendId::new("native").unwrap(),
            )],
            CoordinationSelection::Local,
            [],
        )
        .unwrap()
    }

    fn controller_descriptor_with(
        reactor_slots: Vec<ReactorSlot>,
        reaction_slots: Vec<ReactionSlot>,
    ) -> ComponentDescriptor {
        ComponentDescriptor::try_new(
            "controller.v1".parse().unwrap(),
            1,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            reactor_slots,
            vec![],
            vec![],
            reaction_slots,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            known_bounds(),
        )
        .unwrap()
    }

    #[test]
    fn lowering_keeps_reused_symbols_and_runtime_state_slots_distinct() {
        let forward = lower(&shared_implementation_deployment(false)).unwrap();
        let reverse = lower(&shared_implementation_deployment(true)).unwrap();
        assert_eq!(forward, reverse);

        let enclave = &forward.federates()[0].enclaves()[0];
        assert_eq!(
            enclave
                .required_bindings()
                .iter()
                .map(RequiredBinding::symbol)
                .collect::<Vec<_>>(),
            [
                "action_Shared_2fpulse",
                "action_Shared_2fack",
                "port_Shared_2farray_5fin",
                "port_Shared_2farray_5fin",
                "port_Shared_2fbank_5fin",
                "port_Shared_2fbank_5fin",
                "port_Shared_2foutput",
                "reaction_Shared_2femit",
                "reaction_Shared_2freset_5factive",
                "reaction_Shared_2fshutdown",
                "reaction_Shared_2fstart",
                "reaction_Shared_2ftimer_5ffired",
                "reaction_Shared_2freceive",
                "state_Shared",
                "state_Shared",
            ]
        );
        let states = enclave
            .required_bindings()
            .iter()
            .filter_map(|binding| match binding {
                RequiredBinding::State {
                    component,
                    implementation,
                    ..
                } => Some((
                    component.to_string(),
                    implementation.to_string(),
                    binding.symbol(),
                )),
                RequiredBinding::Reaction { .. }
                | RequiredBinding::Port { .. }
                | RequiredBinding::Action { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            [
                (
                    "vehicle/controller".to_owned(),
                    "shared-host".to_owned(),
                    "state_Shared".to_owned(),
                ),
                (
                    "vehicle/sensor".to_owned(),
                    "shared-host".to_owned(),
                    "state_Shared".to_owned(),
                ),
            ]
        );

        let image = enclave.view().unwrap();
        assert_eq!(
            image
                .ports()
                .values()
                .map(|port| port.binding())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            5
        );
        assert_ne!(
            image.reactors()[ReactorIndex::new(0)].state_binding(),
            image.reactors()[ReactorIndex::new(1)].state_binding()
        );
        assert_eq!(image.storage_bounds().state_slots(), 2);
    }

    #[test]
    fn lowering_reports_invalid_descriptor_slot_mappings() {
        let rootless = controller_descriptor_with(vec![], vec![]);
        assert!(matches!(
            lower(&deployment_with_controller_descriptor(rootless)),
            Err(CompileError::DescriptorRoot { roots: 0, .. })
        ));

        let root = ReactorSlotId::new("Controller").unwrap();
        let missing_reaction = controller_descriptor_with(
            vec![ReactorSlot {
                id: root,
                parent: None,
            }],
            vec![],
        );
        assert!(matches!(
            lower(&deployment_with_controller_descriptor(missing_reaction)),
            Err(CompileError::MissingDescriptorSlot {
                kind: "reaction",
                logical,
                ..
            }) if logical == "vehicle/controller/emit"
        ));
    }

    #[test]
    fn lowering_is_canonical_under_selection_reordering() {
        let forward = lower(&deployment(false, false)).unwrap();
        let reverse = lower(&deployment(true, false)).unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(forward.federates()[0].id().as_str(), "host");
        let enclaves = forward.federates()[0].enclaves();
        assert_eq!(enclaves.len(), 2);
        assert_eq!(enclaves[0].id().to_string(), "vehicle/controller");
        assert_eq!(enclaves[1].id().to_string(), "vehicle/sensor");
        for enclave in enclaves {
            enclave.view().unwrap();
        }
    }
    #[test]
    fn lowering_rejects_distributed_coordination_in_this_slice() {
        let error = lower(&deployment(false, true)).unwrap_err();
        assert!(matches!(error, CompileError::UnsupportedCoordination));
    }
    #[test]
    fn lowering_preserves_canonical_mode_transition_identity() {
        let compiled = lower(&local_deployment(
            false,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::ModeTransition,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();

        assert_eq!(
            enclave.reactions()[ReactionIndex::new(1)].mode_effect(),
            Some(crate::runtime::CompiledModeEffectRef {
                target: ModeIndex::new(1),
                transition: crate::runtime::TransitionKind::Reset,
            })
        );
    }
    #[test]
    fn bounded_lowering_rejects_unbounded_resources() {
        let cases = [
            (
                DescriptorBounds {
                    queue_capacity: DescriptorBound::Unknown,
                    ..known_bounds()
                },
                "event-queue",
            ),
            (
                DescriptorBounds {
                    payload_bytes: DescriptorBound::Unknown,
                    ..known_bounds()
                },
                "payload-bytes",
            ),
        ];
        for (bounds, expected) in cases {
            let error = lower(&deployment_with_bounds(
                false,
                false,
                false,
                ConnectionSemantics::Logical { after: None },
                [bounds; 2],
                DependencyCase::None,
            ))
            .unwrap_err();
            assert!(matches!(
                error,
                CompileError::UnboundedResource { resource, .. } if resource == expected
            ));
        }
    }
    #[test]
    fn bounded_lowering_rejects_queue_and_byte_overflow() {
        let cases = [
            (
                [DescriptorBounds {
                    queue_capacity: DescriptorBound::Known(u32::MAX as u64),
                    ..known_bounds()
                }; 2],
                "event-queue",
            ),
            (
                [
                    DescriptorBounds {
                        payload_bytes: DescriptorBound::Known(u64::MAX),
                        ..known_bounds()
                    },
                    DescriptorBounds {
                        payload_bytes: DescriptorBound::Known(1),
                        ..known_bounds()
                    },
                ],
                "payload-bytes",
            ),
        ];
        for (bounds, expected) in cases {
            let error = lower(&deployment_with_bounds(
                false,
                false,
                true,
                ConnectionSemantics::Logical { after: None },
                bounds,
                DependencyCase::None,
            ))
            .unwrap_err();
            assert!(matches!(
                error,
                CompileError::ResourceOverflow { resource, .. } if resource == expected
            ));
        }
    }
    #[test]
    fn cross_enclave_connection_lowers_to_paired_scheduler_routes() {
        let compiled = lower(&deployment(false, false)).unwrap();
        let source = compiled.federates()[0].enclaves()[0].view().unwrap();
        let target = compiled.federates()[0].enclaves()[1].view().unwrap();
        assert_eq!(source.routes().len(), 1);
        assert_eq!(
            source.routes()[RouteIndex::new(0)].direction(),
            RouteDirection::Outbound
        );
        assert_eq!(
            source.route_boundary_id(RouteIndex::new(0)).as_str(),
            "controller-to-sensor"
        );
        assert_eq!(target.routes().len(), 1);
        assert_eq!(
            target.routes()[RouteIndex::new(0)].direction(),
            RouteDirection::Inbound
        );
    }
    #[test]
    fn same_enclave_zero_delay_connection_collapses_to_one_port_slot() {
        for after in [None, Some(crate::runtime::Duration::ZERO)] {
            let compiled = lower(&local_deployment(
                true,
                ConnectionSemantics::Logical { after },
                DependencyCase::None,
            ))
            .unwrap();
            let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
            assert_eq!(enclave.ports().len(), 5);
            assert!(enclave.routes().is_empty());
        }
    }
    #[test]
    fn placement_only_deployment_with_no_enclaves_lowers_to_an_empty_root_image() {
        let mut topology = ApplicationTopologyBuilder::new("placement-only").unwrap();
        topology
            .add_placement_group(PlacementGroupId::new("placement/host").unwrap(), None)
            .unwrap();
        let deployment = ResolvedDeployment::new(
            topology.finish().unwrap(),
            [],
            [PlacementAssignment::new(
                PlacementGroupId::new("placement/host").unwrap(),
                FederateId::new("host").unwrap(),
            )],
            [FederateConfig::new(
                FederateId::new("host").unwrap(),
                TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                RuntimeBackendId::new("native").unwrap(),
            )],
            CoordinationSelection::Local,
            [],
        )
        .unwrap();
        let compiled = lower(&deployment).unwrap();
        assert!(compiled.federates()[0].enclaves().is_empty());
        compiled.validate().unwrap();
    }
    #[test]
    fn delayed_same_enclave_connection_lowers_to_a_valid_route_pair() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical {
                after: Some(crate::runtime::Duration::nanoseconds(2)),
            },
            DependencyCase::None,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(enclave.routes().len(), 2);
        for route in enclave.routes().values() {
            assert_eq!(route.timing_domain(), TimingDomain::Logical);
            assert_eq!(route.delay_nanos(), 2);
        }
    }
    #[test]
    fn physical_connection_is_routed_and_breaks_same_tag_dependency() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Physical { after: None },
            DependencyCase::None,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(enclave.routes().len(), 2);
        assert!(enclave
            .routes()
            .values()
            .all(|route| route.timing_domain() == TimingDomain::Physical));
        assert_eq!(
            enclave.reactions()[ReactionIndex::new(5)].dependency_level(),
            0
        );
    }
    #[test]
    fn same_tag_reaction_dependencies_are_precomputed_before_slicing() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::None,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(enclave.reactions().len(), 6);
        assert_eq!(
            enclave.reactions()[ReactionIndex::new(0)].dependency_level(),
            1
        );
        assert_eq!(
            enclave.reactions()[ReactionIndex::new(3)].dependency_level(),
            0
        );
        assert_eq!(
            enclave.reactions()[ReactionIndex::new(5)].dependency_level(),
            2
        );
        assert_eq!(enclave.required_bindings().len(), 15);
    }
    #[test]
    fn modes_actions_lifecycle_and_scopes_are_fully_lowered() {
        let compiled = lower(&deployment(false, false)).unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(enclave.actions().len(), 4);
        assert_eq!(enclave.modes().len(), 2);
        assert_eq!(enclave.scopes().len(), 3);
        assert_eq!(
            enclave.reactors()[crate::runtime::image::ReactorIndex::new(0)].initial_mode(),
            Some(ModeIndex::new(1))
        );
        assert_eq!(
            enclave.actions()[ActionIndex::new(0)].timing(),
            ActionTiming::Standard {
                domain: TimingDomain::Logical,
                min_delay_nanos: 3,
            }
        );
        let standard_action = enclave.actions()[ActionIndex::new(0)];
        assert_eq!(
            enclave.required_bindings()[standard_action.binding().unwrap()].kind(),
            BindingKind::Action
        );
        assert_eq!(enclave.actions()[ActionIndex::new(3)].binding(), None);
        let port_binding = enclave.ports()[crate::runtime::image::PortIndex::new(0)].binding();
        assert_eq!(
            enclave.required_bindings()[port_binding].kind(),
            BindingKind::Port
        );
        assert_eq!(
            enclave.actions()[ActionIndex::new(3)].timing(),
            ActionTiming::Timer {
                period_nanos: Some(7)
            }
        );
        assert_eq!(enclave.scope_descendants(ScopeIndex::new(0)).len(), 3);
        assert_eq!(
            enclave.scope_reset_reactions(ScopeIndex::new(1))[0].reaction(),
            ReactionIndex::new(1)
        );
        assert_eq!(enclave.startup_actions()[0].action(), ActionIndex::new(2));
        assert_eq!(
            enclave.timer_startup_actions()[0].action(),
            ActionIndex::new(3)
        );
        assert_eq!(enclave.timer_startup_actions()[0].logical_delay_nanos(), 5);
        assert_eq!(
            enclave.shutdown_reactions()[0].action(),
            ActionIndex::new(1)
        );
        assert_eq!(enclave.shutdown_actions(), &[ActionIndex::new(1)]);
        assert_eq!(enclave.storage_bounds().action_slots(), 4);
    }
    #[test]
    fn reaction_cycles_report_stable_reaction_identities() {
        let error = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::ReactionCycle,
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            CompileError::ReactionCycle { reactions, .. }
                if reactions.iter().any(|id| id.to_string() == "vehicle/controller/emit")
                    && reactions.iter().any(|id| id.to_string() == "vehicle/controller/start")
        ));
    }
    #[test]
    fn a_reaction_triggered_by_its_own_effect_reports_a_cycle() {
        let error = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::PortSelfCycle,
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            CompileError::ReactionCycle { reactions, .. }
                if reactions.as_ref() == [ReactionId::new("vehicle/controller/emit").unwrap()]
        ));
    }
    #[test]
    fn mutually_exclusive_modal_reactions_do_not_create_dependencies() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::MutuallyExclusiveModes,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert_eq!(
            enclave.reactions()[ReactionIndex::new(4)].dependency_level(),
            0
        );
    }
    #[test]
    fn dense_reaction_order_uses_encoded_stable_identity_text() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::EncodedOrdering,
        ))
        .unwrap();
        let enclave = compiled.federates()[0].enclaves()[0].view().unwrap();
        assert!(enclave
            .reactions()
            .values()
            .enumerate()
            .all(|(index, reaction)| {
                reaction.binding().as_u32() == u32::try_from(index).unwrap() + 7
            }));
    }
    #[test]
    fn dense_cardinality_overflow_is_reported_before_conversion() {
        let enclave = StableEnclaveId::new("vehicle/controller").unwrap();
        assert!(matches!(
            checked_u32(usize::MAX, &enclave, "reactions"),
            Err(CompileError::ResourceOverflow {
                resource: "reactions",
                ..
            })
        ));
    }
}
