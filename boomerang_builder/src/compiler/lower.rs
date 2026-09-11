use super::compiled::{OwnedBindingImage, OwnedRouteImage};
use super::coordination::project_central_rti;
use super::identity::canonical_identity_text;
use super::packed::{PackedSliceBuilder, PackedSliceOverflow};
use super::{
    federation::{
        analyze_federation_graph, AnalyzedFederationGraph, FederationDelay, FederationEdge,
    },
    GlobalFederationImage, OwnedCompiledDeployment, OwnedCoordinationProjection, OwnedEnclaveImage,
    OwnedFederateImage, RequiredBinding, RequiredBindings, ResolvedDeployment,
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
        boundary: super::BoundaryId,
    },
    /// Backend-neutral federation analysis rejected the resolved deployment graph.
    #[error(transparent)]
    FederationAnalysis(#[from] super::federation::FederationAnalysisError),
    /// The analyzed federation cannot be represented by the selected coordination image.
    #[error(transparent)]
    CoordinationProjection(#[from] super::CoordinationProjectionError),
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
    /// Canonical backend-neutral federation facts projected by coordination adapters.
    federation: AnalyzedFederationGraph,
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

/// Lowers a resolved deployment into canonical immutable compiled images.
pub fn lower(deployment: &ResolvedDeployment) -> Result<OwnedCompiledDeployment, CompileError> {
    validate_policies(deployment)?;
    let analysis = analyze(deployment)?;
    let mut federates = deployment.federates().collect::<Vec<_>>();
    federates.sort_by_cached_key(|federate| canonical_identity_text(federate.id()));
    let members = analysis.federation.members().to_vec().into_boxed_slice();
    let federation_edges = analysis.federation.edges().to_vec().into_boxed_slice();
    let mut owned_federates: tinymap::TinyMap<FederateIndex, _> = tinymap::TinyMap::new();
    let mut owned_enclaves: tinymap::TinyMap<EnclaveIndex, _> = tinymap::TinyMap::new();
    for federate in federates {
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
        coordination: match deployment.coordination() {
            super::CoordinationSelection::Local => OwnedCoordinationProjection::Local,
            super::CoordinationSelection::Distributed {
                backend: super::CoordinationBackend::CentralRti,
            } => OwnedCoordinationProjection::CentralRti(Box::new(project_central_rti(
                &analysis.federation,
                deployment,
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
fn validate_policies(deployment: &ResolvedDeployment) -> Result<(), CompileError> {
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
    for federate in deployment.federates() {
        require!("recovery", federate.recovery(), RecoveryPolicy::FailStop);
    }
    for boundary in deployment.boundary_bindings() {
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
fn analyze(deployment: &ResolvedDeployment) -> Result<GlobalAnalysis, CompileError> {
    let topology = deployment.topology();
    let federation = analyze_federation_graph(
        deployment.federates().map(|federate| federate.id().clone()),
        topology
            .connections()
            .map(|(_, connection)| federation_edge(deployment, connection))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten(),
    )?;
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
        federation,
        port_representatives,
        reaction_levels: levels,
    })
}

/// Resolves one cross-Federate connection into a backend-neutral graph edge.
fn federation_edge(
    deployment: &ResolvedDeployment,
    connection: &super::Connection,
) -> Result<Option<FederationEdge>, CompileError> {
    let topology = deployment.topology();
    let owner = |port: &super::PortId| {
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

mod enclave;
use enclave::lower_enclave;
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
#[cfg(test)]
mod tests {
    use super::{lower, CompileError};
    use crate::compiler::compiled::FederateSliceError;
    use crate::{
        compiler::{
            ActionId, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, BankMember,
            BoundaryBinding, BoundaryId, BoundaryPolicies, CodecCapabilityId, ComponentInstance,
            ComponentInstanceId, ConnectionSemantics, CoordinationBackend, CoordinationSelection,
            FederateConfig, FederateId, FlowId, ImplementationBinding, ImplementationId, ModeId,
            ModeTransition, ModeTransitionKind, OwnedCompiledDeployment, PhysicalBoundaryId,
            PhysicalBoundaryMetadata, PlacementAssignment, PlacementGroupId, PortDirection, PortId,
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
            ActionIndex, ActionTiming, BindingKind, BoundaryFailurePolicy, CodecPolicy,
            CoordinationProjection, FederateIndex, ModeIndex, ReactionIndex, ReactorIndex,
            RecoveryPolicy, RouteDirection, RouteIndex, RtiImage, RtiRouteIndex, ScopeIndex,
            SecurityPolicy, TimingDomain, TimingPolicy, TransportPolicy,
        },
    };

    fn federate_enclaves(
        deployment: &OwnedCompiledDeployment,
        federate: FederateIndex,
    ) -> &[crate::compiler::OwnedEnclaveImage] {
        deployment
            .enclaves()
            .get_span(deployment.federates()[federate].enclaves())
            .expect("lowered Federate span belongs to the deployment Enclave table")
    }

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
        parallel: bool,
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
                BoundaryId::new(if parallel {
                    "route/-"
                } else {
                    "controller-to-sensor"
                })
                .unwrap(),
                output.clone(),
                input.clone(),
                semantics,
            )
            .unwrap();
        if parallel {
            topology
                .add_connection(
                    BoundaryId::new("route/%2F").unwrap(),
                    output,
                    input,
                    semantics,
                )
                .unwrap();
        }
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
    #[allow(clippy::too_many_arguments, reason = "shared lowering test fixture")]
    fn deployment_with_bounds(
        reverse: bool,
        distributed: bool,
        shared_enclave: bool,
        semantics: ConnectionSemantics,
        bounds: [DescriptorBounds; 2],
        dependency_case: DependencyCase,
        recovery: RecoveryPolicy,
        parallel: bool,
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
            recovery,
        )];
        let mut boundary_bindings = vec![];
        if distributed {
            federates.push(FederateConfig::new(
                FederateId::new("edge").unwrap(),
                TargetTriple::new("aarch64-unknown-none").unwrap(),
                RuntimeBackendId::new("rtic").unwrap(),
                RecoveryPolicy::FailStop,
            ));
            let boundary_binding = |boundary| {
                BoundaryBinding::new(
                    BoundaryId::new(boundary).unwrap(),
                    FlowId::new("sensor-control").unwrap(),
                    PhysicalBoundaryMetadata::new(
                        Some(PhysicalBoundaryId::new("plant/z").unwrap()),
                        Some(PhysicalBoundaryId::new("plant/#g1").unwrap()),
                    ),
                    CodecCapabilityId::new("postcard").unwrap(),
                    TransportCapabilityId::new("udp").unwrap(),
                    BoundaryPolicies::new(
                        BoundaryFailurePolicy::PropagateStop,
                        TransportPolicy::ReliableOrderedFramed,
                        CodecPolicy::CanonicalBounded,
                        TimingPolicy::BestEffort,
                        SecurityPolicy::None,
                    ),
                )
            };
            boundary_bindings.push(boundary_binding(if parallel {
                "route/-"
            } else {
                "controller-to-sensor"
            }));
            if parallel {
                boundary_bindings.push(boundary_binding("route/%2F"));
            }
        }
        if reverse {
            bindings.reverse();
            placements.reverse();
            federates.reverse();
            boundary_bindings.reverse();
        }
        ResolvedDeployment::new(
            topology(
                reverse,
                shared_enclave,
                semantics,
                dependency_case,
                parallel,
            ),
            bindings,
            placements,
            federates,
            if distributed {
                CoordinationSelection::Distributed {
                    backend: CoordinationBackend::CentralRti,
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
            RecoveryPolicy::FailStop,
            false,
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
            RecoveryPolicy::FailStop,
            false,
        )
    }
    fn distributed_with(
        semantics: ConnectionSemantics,
        recovery: RecoveryPolicy,
        parallel: bool,
    ) -> ResolvedDeployment {
        deployment_with_bounds(
            false,
            true,
            false,
            semantics,
            [known_bounds(); 2],
            DependencyCase::None,
            recovery,
            parallel,
        )
    }
    fn with_central_rti<T>(
        compiled: &OwnedCompiledDeployment,
        f: impl FnOnce(RtiImage<'_>) -> T,
    ) -> T {
        compiled.with_coordination(|coordination| {
            let CoordinationProjection::CentralRti(rti) = coordination else {
                panic!("distributed deployment must select the central RTI projection");
            };
            f(rti)
        })
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
                false,
            ),
            bindings,
            placements,
            [FederateConfig::new(
                FederateId::new("host").unwrap(),
                TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
                RuntimeBackendId::new("native").unwrap(),
                RecoveryPolicy::FailStop,
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
                false,
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
                RecoveryPolicy::FailStop,
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

        let enclave = &federate_enclaves(&forward, FederateIndex::new(0))[0];
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

        enclave
            .with_view(|image| {
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
            })
            .unwrap();
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
        assert_eq!(
            forward.federates()[FederateIndex::new(0)].id().as_str(),
            "host"
        );
        let enclaves = federate_enclaves(&forward, FederateIndex::new(0));
        assert_eq!(enclaves.len(), 2);
        assert_eq!(enclaves[0].id().to_string(), "vehicle/controller");
        assert_eq!(enclaves[1].id().to_string(), "vehicle/sensor");
        for enclave in enclaves {
            enclave.with_view(|_| ()).unwrap();
        }
    }
    #[test]
    fn distributed_lowering_preserves_canonical_member_identities() {
        let forward = lower(&deployment(false, true)).unwrap();
        let reverse = lower(&deployment(true, true)).unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .federation()
                .members()
                .iter()
                .map(FederateId::as_str)
                .collect::<Vec<_>>(),
            ["edge", "host"]
        );
    }

    /// Projects a real lowered Federate without retaining sibling Enclaves or bindings.
    #[test]
    fn federate_slice_from_lowered_deployment_preserves_selected_root_rows() {
        let compiled = lower(&deployment(false, true)).unwrap();
        let federate = FederateIndex::new(1);
        let selected = &compiled.federates()[federate];
        let expected_range = selected.enclaves();
        let expected_enclaves = compiled.enclaves().get_span(expected_range).unwrap();

        let slice = compiled.federate_slice(federate).unwrap();
        assert_eq!(slice.federate(), federate);
        assert_eq!(slice.enclave_range(), expected_range);
        assert!(std::ptr::eq(slice.enclaves(), expected_enclaves));
        assert_eq!(slice.enclaves()[0].id().to_string(), "vehicle/controller");
        assert_eq!(
            slice
                .enclaves()
                .iter()
                .flat_map(|enclave| enclave.required_bindings().iter())
                .map(|binding| match binding {
                    RequiredBinding::State {
                        component,
                        implementation,
                        ..
                    }
                    | RequiredBinding::Reaction {
                        component,
                        implementation,
                        ..
                    }
                    | RequiredBinding::Port {
                        component,
                        implementation,
                        ..
                    }
                    | RequiredBinding::Action {
                        component,
                        implementation,
                        ..
                    } => (
                        component.to_string(),
                        implementation.to_string(),
                        binding.symbol(),
                    ),
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "action_Controller_2fpulse".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "port_Controller_2farray_5fin".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "port_Controller_2farray_5fin".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "port_Controller_2fbank_5fin".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "port_Controller_2fbank_5fin".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "port_Controller_2foutput".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "reaction_Controller_2femit".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "reaction_Controller_2freset_5factive".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "reaction_Controller_2fshutdown".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "reaction_Controller_2fstart".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "reaction_Controller_2ftimer_5ffired".to_owned(),
                ),
                (
                    "vehicle/controller".to_owned(),
                    "controller-host".to_owned(),
                    "state_Controller".to_owned(),
                ),
            ]
        );
        assert!(matches!(
            compiled.federate_slice(FederateIndex::new(2)),
            Err(FederateSliceError::FederateNotFound {
                federate: missing,
            }) if missing == FederateIndex::new(2)
        ));
    }

    #[test]
    fn central_rti_projection_uses_precomputed_dense_dependencies() {
        let compiled = lower(&deployment(false, true)).unwrap();
        with_central_rti(&compiled, |rti| {
            let edge = FederateIndex::new(0);
            let host = FederateIndex::new(1);
            assert_eq!(
                rti.direct_incoming(edge)
                    .iter()
                    .map(|dependency| (dependency.source(), dependency.delay_nanos()))
                    .collect::<Vec<_>>(),
                [(host, 0)]
            );
            assert_eq!(
                rti.transitive_incoming(edge)
                    .iter()
                    .map(|dependency| (dependency.source(), dependency.delay_nanos()))
                    .collect::<Vec<_>>(),
                [(host, 0)]
            );
            assert_eq!(rti.affected_downstream(host), [edge]);
        });
    }

    #[test]
    fn central_rti_projection_preserves_route_identity_and_delay() {
        let compiled = lower(&distributed_with(
            ConnectionSemantics::Logical {
                after: Some(crate::runtime::Duration::milliseconds(5)),
            },
            RecoveryPolicy::FailStop,
            false,
        ))
        .unwrap();
        with_central_rti(&compiled, |rti| {
            let route_index = RtiRouteIndex::new(0);
            let route = rti.routes()[route_index];
            assert_eq!(
                rti.route_boundary(route_index).as_str(),
                "controller-to-sensor"
            );
            assert_eq!(route.source(), FederateIndex::new(1));
            assert_eq!(route.target(), FederateIndex::new(0));
            assert_eq!(route.delay_nanos(), 5_000_000);
        });
    }

    #[test]
    fn backend_neutral_federation_remains_authoritative_after_projection() {
        let compiled = lower(&deployment(false, true)).unwrap();
        let edge = &compiled.federation().edges()[0];
        assert_eq!(edge.id().to_string(), "controller-to-sensor");
        assert_eq!(edge.source().as_str(), "host");
        assert_eq!(edge.target().as_str(), "edge");
        assert_eq!(edge.delay().as_nanos(), 0);
    }

    #[test]
    fn central_rti_projection_preserves_flow_and_physical_boundary_identities() {
        let compiled = lower(&distributed_with(
            ConnectionSemantics::Logical { after: None },
            RecoveryPolicy::FailStop,
            false,
        ))
        .unwrap();
        with_central_rti(&compiled, |rti| {
            let route = RtiRouteIndex::new(0);
            assert_eq!(rti.route_flow(route), "sensor-control");
            assert_eq!(rti.route_physical_input(route), Some("plant/z"));
            assert_eq!(rti.route_physical_output(route), Some("plant/#g1"));
        });
    }

    #[test]
    fn cross_federate_physical_connection_is_reserved_for_later_slices() {
        let error = lower(&distributed_with(
            ConnectionSemantics::Physical { after: None },
            RecoveryPolicy::FailStop,
            false,
        ))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "cross-Federate physical connection 'controller-to-sensor' is unsupported"
        );
    }

    #[test]
    fn one_flow_identity_may_span_parallel_boundary_identities() {
        let compiled = lower(&distributed_with(
            ConnectionSemantics::Logical { after: None },
            RecoveryPolicy::FailStop,
            true,
        ))
        .unwrap();
        with_central_rti(&compiled, |rti| {
            assert_eq!(rti.flow_count(), 1);
            assert_eq!(
                rti.routes()
                    .keys()
                    .map(|route| (rti.route_boundary(route).as_str(), rti.route_flow(route)))
                    .collect::<Vec<_>>(),
                [
                    ("route/%2F", "sensor-control"),
                    ("route/-", "sensor-control"),
                ]
            );
        });
    }

    #[test]
    fn central_rti_projection_preserves_typed_policies_and_dense_capability_references() {
        let compiled = lower(&deployment(false, true)).unwrap();
        with_central_rti(&compiled, |rti| {
            let route = RtiRouteIndex::new(0);
            assert_eq!(
                rti.member_recovery_policy(FederateIndex::new(1)),
                RecoveryPolicy::FailStop
            );
            assert_eq!(
                rti.route_failure_policy(route),
                BoundaryFailurePolicy::PropagateStop
            );
            assert_eq!(
                rti.route_transport_policy(route),
                TransportPolicy::ReliableOrderedFramed
            );
            assert_eq!(rti.route_codec_policy(route), CodecPolicy::CanonicalBounded);
            assert_eq!(rti.route_timing_policy(route), TimingPolicy::BestEffort);
            assert_eq!(rti.route_security_policy(route), SecurityPolicy::None);
            assert_eq!(rti.route_transport_capability(route), "udp");
            assert_eq!(rti.route_codec_capability(route), "postcard");
        });
    }

    #[test]
    fn known_unsupported_policy_selection_fails_closed() {
        let error = lower(&distributed_with(
            ConnectionSemantics::Logical { after: None },
            RecoveryPolicy::RestartReset,
            false,
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            CompileError::UnsupportedPolicy { category: "recovery", selection }
                if selection == "restart-reset"
        ));
    }
    #[test]
    fn lowering_preserves_canonical_mode_transition_identity() {
        let compiled = lower(&local_deployment(
            false,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::ModeTransition,
        ))
        .unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert_eq!(
                    enclave.reactions()[ReactionIndex::new(1)].mode_effect(),
                    Some(crate::runtime::CompiledModeEffectRef {
                        target: ModeIndex::new(1),
                        transition: crate::runtime::TransitionKind::Reset,
                    })
                );
            })
            .unwrap();
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
                RecoveryPolicy::FailStop,
                false,
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
                RecoveryPolicy::FailStop,
                false,
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
        let enclaves = federate_enclaves(&compiled, FederateIndex::new(0));
        enclaves[0]
            .with_view(|source| {
                assert_eq!(source.routes().len(), 1);
                assert_eq!(
                    source.routes()[RouteIndex::new(0)].direction(),
                    RouteDirection::Outbound
                );
                assert_eq!(
                    source.route_boundary_id(RouteIndex::new(0)).as_str(),
                    "controller-to-sensor"
                );
            })
            .unwrap();
        enclaves[1]
            .with_view(|target| {
                assert_eq!(target.routes().len(), 1);
                assert_eq!(
                    target.routes()[RouteIndex::new(0)].direction(),
                    RouteDirection::Inbound
                );
            })
            .unwrap();
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
            federate_enclaves(&compiled, FederateIndex::new(0))[0]
                .with_view(|enclave| {
                    assert_eq!(enclave.ports().len(), 5);
                    assert!(enclave.routes().is_empty());
                })
                .unwrap();
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
                RecoveryPolicy::FailStop,
            )],
            CoordinationSelection::Local,
            [],
        )
        .unwrap();
        let compiled = lower(&deployment).unwrap();
        assert!(compiled.federates()[FederateIndex::new(0)]
            .enclaves()
            .is_empty());
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
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert_eq!(enclave.routes().len(), 2);
                for route in enclave.routes().values() {
                    assert_eq!(route.timing_domain(), TimingDomain::Logical);
                    assert_eq!(route.delay_nanos(), 2);
                }
            })
            .unwrap();
    }
    #[test]
    fn physical_connection_is_routed_and_breaks_same_tag_dependency() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Physical { after: None },
            DependencyCase::None,
        ))
        .unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert_eq!(enclave.routes().len(), 2);
                assert!(enclave
                    .routes()
                    .values()
                    .all(|route| route.timing_domain() == TimingDomain::Physical));
                assert_eq!(
                    enclave.reactions()[ReactionIndex::new(5)].dependency_level(),
                    0
                );
            })
            .unwrap();
    }
    #[test]
    fn same_tag_reaction_dependencies_are_precomputed_before_slicing() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::None,
        ))
        .unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
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
            })
            .unwrap();
    }
    #[test]
    fn modes_actions_lifecycle_and_scopes_are_fully_lowered() {
        let compiled = lower(&deployment(false, false)).unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
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
                let port_binding =
                    enclave.ports()[crate::runtime::image::PortIndex::new(0)].binding();
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
            })
            .unwrap();
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
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert_eq!(
                    enclave.reactions()[ReactionIndex::new(4)].dependency_level(),
                    0
                );
            })
            .unwrap();
    }
    #[test]
    fn dense_reaction_order_uses_encoded_stable_identity_text() {
        let compiled = lower(&local_deployment(
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::EncodedOrdering,
        ))
        .unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert!(enclave
                    .reactions()
                    .values()
                    .enumerate()
                    .all(|(index, reaction)| {
                        reaction.binding().as_u32() == u32::try_from(index).unwrap() + 7
                    }));
            })
            .unwrap();
    }
    #[test]
    fn dense_cardinality_overflow_is_reported_before_conversion() {
        #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
        struct SmallKey(usize);
        impl From<usize> for SmallKey {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }
        impl tinymap::Key for SmallKey {
            const MAX_LEN: usize = 1;

            fn index(&self) -> usize {
                self.0
            }
        }

        let enclave = StableEnclaveId::new("vehicle/controller").unwrap();
        let result =
            tinymap::TinyMap::<SmallKey, _>::try_from_iter(["first", "second"]).map_err(|_| {
                CompileError::ResourceOverflow {
                    enclave: enclave.clone(),
                    resource: "reactions",
                }
            });
        assert!(matches!(
            result,
            Err(CompileError::ResourceOverflow {
                resource: "reactions",
                ..
            })
        ));
    }
}
