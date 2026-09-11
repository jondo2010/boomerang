//! One-way materialization of resolved compiler semantics into owned runtime images.
//!
//! Lowering consumes [`ResolvedDeployment`], performs canonical stable-identity analysis and
//! selection, and produces [`OwnedCompiledDeployment`]. The final runtime image tables are the
//! authoritative dense-key owners: each `TinyMap` allocates its own keys, and temporary
//! stable-identity-to-key registries only retain keys returned by those owners.
//!
//! ```text
//! ResolvedDeployment
//!     -> canonical stable-ID selection and analysis
//!     -> owner-generated dense entity tables + packed anonymous relationships
//!     -> OwnedCompiledDeployment
//! ```
//!
//! Numeric casts, parallel ordinal counters, shared ordinals between key types, and
//! stringify/reparse bridges must not allocate or translate collection-owned keys. Code
//! generation renders the assignments made here; it does not perform a second lowering pass.

use super::{
    compiled::{OwnedBindingImage, OwnedRouteImage},
    coordination::project_central_rti,
    federation::{
        analyze_federation_graph, AnalyzedFederationGraph, FederationAnalysisError,
        FederationDelay, FederationEdge,
    },
    identity::canonical_identity_text,
    packed::{PackedSliceBuilder, PackedSliceOverflow},
    ApplicationTopology, BoundaryId, ComponentInstanceId, CoordinationProjectionError,
    GlobalFederationImage, ImplementationId, OwnedCompiledDeployment, OwnedCoordinationProjection,
    OwnedEnclaveImage, OwnedFederateImage, PortId, Reaction, ReactionId, ReactorId,
    RequiredBinding, RequiredBindings, ResolvedDeployment, StableEnclaveId, StablePath,
};
use crate::{
    descriptor::{ActionSlotId, DescriptorBound, PortSlotId, ReactionSlotId, ReactorSlotId},
    runtime::image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingSlotIndex,
        BoundaryFailurePolicy, EnclaveIndex, FederateIndex, LevelReactionImage,
        LifecycleReactionImage, ModeImage, ModeIndex, PortImage, PortIndex, ReactionImage,
        ReactionIndex, ReactorImage, ReactorIndex, RecoveryPolicy, RouteDirection, ScopeImage,
        ScopeIndex, SecurityPolicy, StateSlotIndex, StorageBounds, TimerStartupImage, TimingDomain,
        TimingPolicy,
    },
};
use std::collections::{BTreeMap, BTreeSet};

mod enclave;
#[cfg(test)]
mod tests;

/// A canonical compiled-image lowering failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CompileError {
    /// This slice cannot project a distributed coordination backend.
    #[error("distributed coordination projection is not implemented")]
    UnsupportedCoordination,
    /// A physical connection crosses Federates before physical-time coordination is defined.
    #[error("cross-Federate physical connection '{boundary}' is unsupported")]
    UnsupportedPhysicalFederation {
        /// Stable boundary reserved for a later physical-time slice.
        boundary: BoundaryId,
    },
    /// Backend-neutral federation analysis rejected the resolved deployment graph.
    #[error(transparent)]
    FederationAnalysis(#[from] FederationAnalysisError),
    /// The analyzed federation cannot be represented by the selected coordination image.
    #[error(transparent)]
    CoordinationProjection(#[from] CoordinationProjectionError),
    /// A known roadmap policy is not supported by this compiler slice.
    #[error("unsupported {category} policy '{selection}'")]
    UnsupportedPolicy {
        /// Policy category being selected.
        category: &'static str,
        /// Canonical spelling of the known but unavailable policy.
        selection: &'static str,
    },
    /// A reaction requests a mode transition that the compiled image cannot yet represent.
    #[error("reaction {reaction} requests an unsupported compiled mode transition")]
    UnsupportedModeTransition {
        /// Stable identity of the transition-bearing reaction.
        reaction: ReactionId,
    },
    /// A component did not declare a required finite resource bound.
    #[error("component {component} in Enclave {enclave} has no {resource} bound")]
    UnboundedResource {
        /// Stable component identity.
        component: ComponentInstanceId,
        /// Stable Enclave identity.
        enclave: StableEnclaveId,
        /// Missing resource category.
        resource: &'static str,
    },
    /// A bounded resource cannot be represented or aggregated.
    #[error("{resource} bound overflows in Enclave {enclave}")]
    ResourceOverflow {
        /// Stable Enclave identity.
        enclave: StableEnclaveId,
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
        enclave: StableEnclaveId,
        /// Runtime image validation detail.
        message: String,
    },
    /// Same-tag reaction dependencies contain a cycle.
    #[error("reaction dependency cycle in Enclave {enclave}")]
    ReactionCycle {
        /// Stable Enclave identity.
        enclave: StableEnclaveId,
        /// Canonically ordered stable identities blocked by the cycle.
        reactions: Box<[ReactionId]>,
    },
    /// An implementation descriptor does not provide one root reactor slot.
    #[error(
        "implementation {implementation} for component {component} declares {roots} root reactor slots"
    )]
    DescriptorRoot {
        /// Logical component instance receiving the implementation.
        component: ComponentInstanceId,
        /// Selected implementation with the invalid descriptor root shape.
        implementation: ImplementationId,
        /// Number of parentless descriptor reactor slots.
        roots: usize,
    },
    /// A logical runtime binding has no matching descriptor-local slot.
    #[error(
        "implementation {implementation} for component {component} has no {kind} slot matching {logical}"
    )]
    MissingDescriptorSlot {
        /// Logical component instance receiving the implementation.
        component: ComponentInstanceId,
        /// Selected implementation lacking the descriptor slot.
        implementation: ImplementationId,
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
        component: ComponentInstanceId,
        /// Selected implementation with ambiguous descriptor slots.
        implementation: ImplementationId,
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
    /// Canonical backend-neutral federation facts projected by coordination adapters.
    federation: AnalyzedFederationGraph,
    /// Smallest stable port identity representing each zero-delay local equivalence class.
    port_representatives: BTreeMap<PortId, PortId>,
    /// Longest-predecessor dependency level for every reaction.
    reaction_levels: BTreeMap<ReactionId, u32>,
}

/// Resolves descriptor-local slots for one selected implementation.
struct DescriptorSlots<'a> {
    /// Logical component instance receiving the selected implementation.
    component: &'a ComponentInstanceId,
    /// Selected implementation exporting direct binding symbols.
    implementation: &'a ImplementationId,
    /// Canonical implementation descriptor.
    descriptor: &'a crate::descriptor::ComponentDescriptor,
    /// Unique parentless descriptor reactor slot.
    root: &'a crate::descriptor::ReactorSlot,
}

impl<'a> DescriptorSlots<'a> {
    /// Looks up the selected descriptor and its unique root reactor slot.
    fn for_component(
        deployment: &'a ResolvedDeployment,
        component: &'a ComponentInstanceId,
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
    fn matches_relative_path(&self, logical: &StablePath, descriptor_slot: &StablePath) -> bool {
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
    fn reactor_slot(&self, logical: &ReactorId) -> Result<ReactorSlotId, CompileError> {
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
    fn reaction_slot(&self, logical: &ReactionId) -> Result<ReactionSlotId, CompileError> {
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
        logical: &PortId,
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
        logical: &StablePath,
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

impl ResolvedDeployment {
    /// Lowers a resolved deployment into canonical immutable compiled images.
    pub fn lower(&self) -> Result<OwnedCompiledDeployment, CompileError> {
        self.validate_policies()?;
        let analysis = self.analyze()?;
        let mut federates = self.federates().collect::<Vec<_>>();
        federates.sort_by_cached_key(|federate| canonical_identity_text(federate.id()));
        let members = analysis.federation.members().to_vec().into_boxed_slice();
        let federation_edges = analysis.federation.edges().to_vec().into_boxed_slice();
        let mut owned_federates: tinymap::TinyMap<FederateIndex, _> = tinymap::TinyMap::new();
        let mut owned_enclaves: tinymap::TinyMap<EnclaveIndex, _> = tinymap::TinyMap::new();
        for federate in federates {
            let mut enclaves = self
                .topology()
                .enclaves()
                .filter(|(_, enclave)| {
                    let root = self
                        .topology()
                        .reactor(enclave.root())
                        .expect("validated Enclave root exists");
                    let group = root
                        .placement_group()
                        .expect("resolved Enclave root is placed");
                    self.placement(group)
                        .expect("resolved placement group is assigned")
                        .federate()
                        == federate.id()
                })
                .collect::<Vec<_>>();
            sort_by_encoded_identity(&mut enclaves);
            let enclaves = enclaves
                .into_iter()
                .map(|(id, _)| self.lower_enclave(id, &analysis))
                .collect::<Result<Vec<_>, _>>()?;
            let enclave_span = owned_enclaves.try_extend_exact(enclaves).map_err(|error| {
                CompileError::InvalidDeployment {
                    message: format!("Enclave table {error}"),
                }
            })?;
            owned_federates
                .try_insert(OwnedFederateImage {
                    id: federate.id().clone(),
                    target: federate.target().clone(),
                    runtime: federate.runtime().clone(),
                    enclaves: enclave_span,
                })
                .map_err(|error| CompileError::InvalidDeployment {
                    message: format!("Federate table {error}"),
                })?;
        }
        let compiled = OwnedCompiledDeployment {
            federation: GlobalFederationImage {
                members,
                edges: federation_edges,
            },
            federates: owned_federates,
            enclaves: owned_enclaves,
            coordination: match self.coordination() {
                super::CoordinationSelection::Local => OwnedCoordinationProjection::Local,
                super::CoordinationSelection::Distributed {
                    backend: super::CoordinationBackend::CentralRti,
                } => OwnedCoordinationProjection::CentralRti(Box::new(project_central_rti(
                    &analysis.federation,
                    self,
                )?)),
                super::CoordinationSelection::Distributed { .. } => {
                    return Err(CompileError::UnsupportedCoordination);
                }
            },
        };
        validate_root_image(&compiled)?;
        Ok(compiled)
    }

    /// Fails closed on typed roadmap policies not supported by this compiler slice.
    fn validate_policies(&self) -> Result<(), CompileError> {
        macro_rules! require {
            ($category:literal, $selection:expr, $supported:path) => {{
                let selection = $selection;
                if selection != $supported {
                    return Err(CompileError::UnsupportedPolicy {
                        category: $category,
                        selection: selection.as_str(),
                    });
                }
            }};
        }
        for federate in self.federates() {
            require!("recovery", federate.recovery(), RecoveryPolicy::FailStop);
        }
        for boundary in self.boundary_bindings() {
            let policies = boundary.policies();
            require!(
                "boundary-failure",
                policies.failure(),
                BoundaryFailurePolicy::PropagateStop
            );
            require!("timing", policies.timing(), TimingPolicy::BestEffort);
            require!("security", policies.security(), SecurityPolicy::None);
        }
        Ok(())
    }

    /// Computes canonical port equivalence and reaction levels over the complete deployment.
    fn analyze(&self) -> Result<GlobalAnalysis, CompileError> {
        let topology = self.topology();
        let federation = analyze_federation_graph(
            self.federates().map(|federate| federate.id().clone()),
            topology
                .connections()
                .map(|(_, connection)| federation_edge(self, connection))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten(),
        )?;
        let port_representatives = topology.canonical_port_representatives();
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
            levels.extend(topology.reaction_levels(enclave, &reactions, &port_representatives)?);
        }
        Ok(GlobalAnalysis {
            federation,
            port_representatives,
            reaction_levels: levels,
        })
    }
}
/// Resolves one cross-Federate connection into a backend-neutral graph edge.
fn federation_edge(
    deployment: &ResolvedDeployment,
    connection: &super::Connection,
) -> Result<Option<FederationEdge>, CompileError> {
    let topology = deployment.topology();
    let owner = |port: &PortId| {
        let reactor = topology
            .port(port)
            .and_then(|port| topology.reactor(port.reactor()))?;
        let enclave = topology.enclave(reactor.enclave())?;
        let group = topology.reactor(enclave.root())?.placement_group()?;
        Some(deployment.placement(group)?.federate())
    };
    let Some(source) = owner(connection.source()) else {
        return Ok(None);
    };
    let Some(target) = owner(connection.target()) else {
        return Ok(None);
    };
    if source == target {
        return Ok(None);
    }
    let after = match connection.semantics() {
        super::ConnectionSemantics::Logical { after } => after,
        super::ConnectionSemantics::Physical { .. } => {
            return Err(CompileError::UnsupportedPhysicalFederation {
                boundary: connection.id().clone(),
            });
        }
    };
    let Ok(delay) = u64::try_from(after.map_or(0, |delay| delay.whole_nanoseconds())) else {
        return Ok(None);
    };
    Ok(Some(FederationEdge::new(
        source.clone(),
        target.clone(),
        connection.id().clone(),
        FederationDelay::from_nanos(delay),
    )))
}

impl ApplicationTopology {
    /// Selects the smallest stable identity for each direct local port equivalence class.
    fn canonical_port_representatives(&self) -> BTreeMap<PortId, PortId> {
        let mut parents = self
            .ports()
            .map(|(id, _)| (id.clone(), id.clone()))
            .collect::<BTreeMap<_, _>>();
        for (_, connection) in self.connections() {
            let source_enclave = self
                .port(connection.source())
                .and_then(|port| self.reactor(port.reactor()))
                .map(|reactor| reactor.enclave());
            let target_enclave = self
                .port(connection.target())
                .and_then(|port| self.reactor(port.reactor()))
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
        &self,
        enclave: &StableEnclaveId,
        reactions: &[(&ReactionId, &Reaction)],
        ports: &BTreeMap<PortId, PortId>,
    ) -> Result<BTreeMap<ReactionId, u32>, CompileError> {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        enum Target {
            /// Stable action dependency identity.
            Action(super::ActionId),
            /// Canonical local port dependency identity.
            Port(PortId),
        }
        let mut graph = petgraph::prelude::StableDiGraph::<&ReactionId, ()>::new();
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
                        self,
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
                    let cyclic = component.len() > 1
                        || graph.find_edge(component[0], component[0]).is_some();
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
}

/// Reports whether two reactions are enclosed by distinct sibling modes.
fn reactions_are_mutually_exclusive(
    topology: &ApplicationTopology,
    left: &Reaction,
    right: &Reaction,
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
fn enclosing_modes(topology: &ApplicationTopology, reaction: &Reaction) -> Vec<super::ModeId> {
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
fn port_root(parents: &BTreeMap<PortId, PortId>, port: &PortId) -> PortId {
    let mut root = port;
    while &parents[root] != root {
        root = &parents[root];
    }
    root.clone()
}

/// Converts an optional duration into a checked non-negative nanosecond value.
fn duration_nanos(
    duration: Option<crate::runtime::Duration>,
    enclave: &StableEnclaveId,
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
    enclave: &StableEnclaveId,
    reactors: &[(&ReactorId, &super::Reactor)],
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
    component: &ComponentInstanceId,
    enclave: &StableEnclaveId,
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

/// Orders stable-identity records by their canonical encoded text.
fn sort_by_encoded_identity<I: std::fmt::Display, T>(values: &mut [(&I, &T)]) {
    values.sort_by_cached_key(|(identity, _)| canonical_identity_text(*identity));
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
    compiled
        .validate()
        .map_err(|error| CompileError::InvalidDeployment {
            message: error.to_string(),
        })
}
