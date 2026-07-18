use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure, RtiToFederate,
    TopologyEdge, WireDelay, WireTag,
};

mod index;

use index::{
    CompiledEndpoint, CompiledFederate, EndpointKey, FederateKey, IncomingDependency, IncomingPath,
};

/// Validated static RTI topology with deterministic coordination indexes.
///
/// Construct this once when lowering or loading a federation manifest, then reuse it for each RTI
/// state instantiated from that manifest.
#[derive(Debug)]
pub struct CompiledTopology {
    original: FederatedTopology,
    federates: tinymap::TinyMap<FederateKey, CompiledFederate>,
    federate_keys: BTreeMap<FederateId, FederateKey>,
    endpoints: tinymap::TinyMap<EndpointKey, CompiledEndpoint>,
    endpoint_keys: BTreeMap<EndpointId, EndpointKey>,
    minimum_delays: BTreeMap<(FederateKey, FederateKey), WireDelay>,
}

impl Clone for CompiledTopology {
    fn clone(&self) -> Self {
        Self {
            original: self.original.clone(),
            federates: self.federates.values().cloned().collect(),
            federate_keys: self.federate_keys.clone(),
            endpoints: self.endpoints.values().cloned().collect(),
            endpoint_keys: self.endpoint_keys.clone(),
            minimum_delays: self.minimum_delays.clone(),
        }
    }
}

impl PartialEq for CompiledTopology {
    fn eq(&self, other: &Self) -> bool {
        self.original == other.original
            && self.federates.values().eq(other.federates.values())
            && self.federate_keys == other.federate_keys
            && self.endpoints.values().eq(other.endpoints.values())
            && self.endpoint_keys == other.endpoint_keys
            && self.minimum_delays == other.minimum_delays
    }
}

impl Eq for CompiledTopology {}

impl CompiledTopology {
    pub fn new(topology: FederatedTopology) -> Result<Self, RtiError> {
        let mut members = BTreeSet::new();
        for federate_id in &topology.federates {
            if !members.insert(federate_id.clone()) {
                return Err(RtiError::DuplicateFederate(federate_id.clone()));
            }
        }

        let mut endpoint_routes = BTreeMap::<EndpointId, TopologyEdge>::new();

        for edge in &topology.edges {
            if !members.contains(&edge.source) {
                return Err(RtiError::UndeclaredEdgeFederate {
                    endpoint: edge.endpoint.clone(),
                    federate_id: edge.source.clone(),
                });
            }
            if !members.contains(&edge.target) {
                return Err(RtiError::UndeclaredEdgeFederate {
                    endpoint: edge.endpoint.clone(),
                    federate_id: edge.target.clone(),
                });
            }
            if edge.endpoint.as_str().is_empty() {
                return Err(RtiError::MissingRouteEndpoint {
                    route_source: edge.source.clone(),
                    route_target: edge.target.clone(),
                });
            }

            if let Some(existing) = endpoint_routes.get(&edge.endpoint) {
                if existing == edge {
                    return Err(RtiError::DuplicateRoute {
                        route_source: edge.source.clone(),
                        route_target: edge.target.clone(),
                        endpoint: edge.endpoint.clone(),
                    });
                }
                return Err(RtiError::ConflictingRoute {
                    endpoint: edge.endpoint.clone(),
                });
            }
            endpoint_routes.insert(edge.endpoint.clone(), edge.clone());
        }

        let mut federates = tinymap::TinyMap::with_capacity(members.len());
        let mut federate_keys = BTreeMap::new();
        for federate_id in members {
            let key = federates.insert(CompiledFederate {
                id: federate_id.clone(),
                incoming: Vec::new(),
                downstream: Vec::new(),
                transitive_incoming: Vec::new(),
                transitive_downstream: Vec::new(),
                neighbors: NeighborStructure {
                    federate_id: federate_id.clone(),
                    upstream: Vec::new(),
                    downstream: Vec::new(),
                },
            });
            federate_keys.insert(federate_id, key);
        }

        let mut endpoints = tinymap::TinyMap::with_capacity(endpoint_routes.len());
        let mut endpoint_keys = BTreeMap::new();
        let mut minimum_delays = BTreeMap::<(FederateKey, FederateKey), WireDelay>::new();
        for (endpoint_id, edge) in endpoint_routes {
            let source = federate_keys[&edge.source];
            let target = federate_keys[&edge.target];
            let endpoint = endpoints.insert(CompiledEndpoint {
                id: endpoint_id.clone(),
                source,
                target,
                delay: edge.delay,
            });
            endpoint_keys.insert(endpoint_id, endpoint);

            federates[target].incoming.push(IncomingDependency {
                source,
                endpoint,
                delay: edge.delay,
            });
            federates[source].downstream.push(target);
            minimum_delays
                .entry((source, target))
                .and_modify(|delay| *delay = (*delay).min(edge.delay))
                .or_insert(edge.delay);
            federates[target].neighbors.upstream.push(edge.clone());
            federates[source].neighbors.downstream.push(edge);
        }

        for federate in federates.values_mut() {
            federate.incoming.sort();
            federate.downstream.sort();
            federate.downstream.dedup();
            federate.neighbors.upstream.sort();
            federate.neighbors.downstream.sort();
        }

        for _ in 0..federates.len() {
            let paths = minimum_delays
                .iter()
                .map(|((source, target), delay)| (source.clone(), target.clone(), *delay))
                .collect::<Vec<_>>();
            let mut updates = BTreeMap::new();

            for (source, intermediate, first) in &paths {
                for (next_intermediate, target, second) in &paths {
                    if intermediate != next_intermediate {
                        continue;
                    }
                    let nanos =
                        first
                            .as_nanos()
                            .checked_add(second.as_nanos())
                            .ok_or_else(|| RtiError::PathDelayOverflow {
                                path_source: federates[*source].id.clone(),
                                intermediate: federates[*intermediate].id.clone(),
                                target: federates[*target].id.clone(),
                                first_delay_ns: first.as_nanos(),
                                second_delay_ns: second.as_nanos(),
                            })?;
                    let candidate = WireDelay::from_nanos(nanos);
                    let key = (*source, *target);
                    let current = updates
                        .get(&key)
                        .copied()
                        .or_else(|| minimum_delays.get(&key).copied());
                    if current.is_none_or(|delay| candidate < delay) {
                        updates.insert(key, candidate);
                    }
                }
            }

            if updates.is_empty() {
                break;
            }
            minimum_delays.extend(updates);
        }

        for ((source, target), delay) in &minimum_delays {
            federates[*target].transitive_incoming.push(IncomingPath {
                source: *source,
                delay: *delay,
            });
            federates[*source].transitive_downstream.push(*target);
        }

        Ok(Self {
            original: topology,
            federates,
            federate_keys,
            endpoints,
            endpoint_keys,
            minimum_delays,
        })
    }

    pub fn topology(&self) -> &FederatedTopology {
        &self.original
    }

    pub(crate) fn federate_key(&self, id: &FederateId) -> Option<FederateKey> {
        self.federate_keys.get(id).copied()
    }

    pub(crate) fn federate_id(&self, key: FederateKey) -> &FederateId {
        &self.federates[key].id
    }

    pub(crate) fn federates(
        &self,
    ) -> impl ExactSizeIterator<Item = (FederateKey, &CompiledFederate)> {
        (0..self.federates.len()).map(|index| {
            let key = FederateKey::from(index);
            (key, &self.federates[key])
        })
    }

    pub(crate) fn endpoint_key(&self, id: &EndpointId) -> Option<EndpointKey> {
        self.endpoint_keys.get(id).copied()
    }

    pub(crate) fn endpoint(&self, key: EndpointKey) -> &CompiledEndpoint {
        &self.endpoints[key]
    }

    /// Return the precomputed incoming and outgoing edge view for one federate.
    pub fn neighbors_for(&self, federate_id: &FederateId) -> Option<&NeighborStructure> {
        self.federate_key(federate_id)
            .map(|key| &self.federates[key].neighbors)
    }

    fn incoming(&self, target: &FederateId) -> &[IncomingDependency] {
        self.federate_key(target)
            .map_or(&[], |key| self.federates[key].incoming.as_slice())
    }

    #[cfg(test)]
    fn downstream(&self, source: &FederateId) -> &[FederateKey] {
        self.federate_key(source)
            .map_or(&[], |key| self.federates[key].downstream.as_slice())
    }

    fn transitive_incoming(&self, target: &FederateId) -> &[IncomingPath] {
        self.federate_key(target).map_or(&[], |key| {
            self.federates[key].transitive_incoming.as_slice()
        })
    }

    fn transitive_downstream(&self, source: &FederateId) -> &[FederateKey] {
        self.federate_key(source).map_or(&[], |key| {
            self.federates[key].transitive_downstream.as_slice()
        })
    }

    #[cfg(test)]
    fn minimum_delay(&self, source: &FederateId, target: &FederateId) -> Option<WireDelay> {
        let source = self.federate_key(source)?;
        let target = self.federate_key(target)?;
        self.minimum_delays.get(&(source, target)).copied()
    }

    fn contains_route(
        &self,
        source: &FederateId,
        target: &FederateId,
        endpoint: &EndpointId,
    ) -> bool {
        let Some(source) = self.federate_key(source) else {
            return false;
        };
        let Some(target) = self.federate_key(target) else {
            return false;
        };
        let Some(endpoint) = self.endpoint_key(endpoint) else {
            return false;
        };
        let endpoint = self.endpoint(endpoint);
        endpoint.source == source && endpoint.target == target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextEvent {
    Unknown,
    Finite(WireTag),
    NoFuture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FederateLifecycle {
    Running { next_event: NextEvent },
    Stopped,
}

/// Per-federate control-plane state known by the RTI.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FederateCoordination {
    lifecycle: FederateLifecycle,
    last_completed: WireTag,
    last_granted: Option<WireTag>,
    in_transit: BTreeSet<WireTag>,
}

impl Default for FederateCoordination {
    fn default() -> Self {
        Self {
            lifecycle: FederateLifecycle::Running {
                next_event: NextEvent::Unknown,
            },
            last_completed: WireTag::Never,
            last_granted: None,
            in_transit: BTreeSet::new(),
        }
    }
}

impl FederateCoordination {
    fn advertised_next_event(&self) -> WireTag {
        match self.lifecycle {
            FederateLifecycle::Running {
                next_event: NextEvent::Unknown,
            } => WireTag::Never,
            FederateLifecycle::Running {
                next_event: NextEvent::Finite(tag),
            } => tag,
            FederateLifecycle::Running {
                next_event: NextEvent::NoFuture,
            }
            | FederateLifecycle::Stopped => WireTag::Forever,
        }
    }

    fn requested_tag(&self) -> Option<WireTag> {
        match self.lifecycle {
            FederateLifecycle::Running {
                next_event: NextEvent::Finite(tag),
            } => Some(tag),
            FederateLifecycle::Running {
                next_event: NextEvent::Unknown | NextEvent::NoFuture,
            }
            | FederateLifecycle::Stopped => None,
        }
    }

    fn effective_next_event(&self) -> WireTag {
        self.in_transit.iter().next().copied().map_or_else(
            || self.advertised_next_event(),
            |tag| tag.min(self.advertised_next_event()),
        )
    }

    fn request(&mut self, tag: WireTag) {
        let FederateLifecycle::Running { next_event } = &mut self.lifecycle else {
            return;
        };
        *next_event = if tag == WireTag::FOREVER {
            NextEvent::NoFuture
        } else {
            NextEvent::Finite(tag)
        };
    }

    fn stop(&mut self) {
        self.lifecycle = FederateLifecycle::Stopped;
    }

    fn is_stopped(&self) -> bool {
        matches!(self.lifecycle, FederateLifecycle::Stopped)
    }
}

/// Result of evaluating whether a pending NET request can receive a TAG.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GrantDecision {
    Granted {
        tag: WireTag,
    },
    AlreadyGranted {
        tag: WireTag,
    },
    Blocked {
        requested: WireTag,
        earliest_incoming: Option<WireTag>,
    },
}

/// A message the RTI should deliver to a specific federate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtiDelivery {
    pub federate_id: FederateId,
    pub message: RtiToFederate,
}

impl RtiDelivery {
    fn new(federate_id: FederateId, message: RtiToFederate) -> Self {
        Self {
            federate_id,
            message,
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RtiError {
    #[error("unknown federate `{0}`")]
    UnknownFederate(FederateId),

    #[error("duplicate federate `{0}` in RTI topology")]
    DuplicateFederate(FederateId),

    #[error("route endpoint `{endpoint}` refers to undeclared federate `{federate_id}`")]
    UndeclaredEdgeFederate {
        endpoint: EndpointId,
        federate_id: FederateId,
    },

    #[error("route {route_source} -> {route_target} has an empty endpoint identity")]
    MissingRouteEndpoint {
        route_source: FederateId,
        route_target: FederateId,
    },

    #[error("duplicate route {route_source} -> {route_target} endpoint `{endpoint}`")]
    DuplicateRoute {
        route_source: FederateId,
        route_target: FederateId,
        endpoint: EndpointId,
    },

    #[error("endpoint `{endpoint}` is assigned to conflicting routes")]
    ConflictingRoute { endpoint: EndpointId },

    #[error("delaying tag {tag} by {delay_ns}ns overflowed")]
    TagDelayOverflow { tag: WireTag, delay_ns: u64 },

    #[error("cannot calculate the latest tag strictly before {tag}")]
    TagPredecessorUnderflow { tag: WireTag },

    #[error(
        "minimum path delay {path_source} -> {intermediate} -> {target} overflowed while adding {first_delay_ns}ns and {second_delay_ns}ns"
    )]
    PathDelayOverflow {
        path_source: FederateId,
        intermediate: FederateId,
        target: FederateId,
        first_delay_ns: u64,
        second_delay_ns: u64,
    },

    #[error(
        "{event} identified federate `{claimed_federate}`, but authenticated endpoint is `{authenticated_federate}`"
    )]
    FederateIdentityMismatch {
        event: &'static str,
        authenticated_federate: FederateId,
        claimed_federate: FederateId,
    },

    #[error("{event} from federate `{federate_id}` used illegal tag {tag}")]
    InvalidTag {
        event: &'static str,
        federate_id: FederateId,
        tag: WireTag,
    },

    #[error("NET for federate `{federate_id}` regressed from {previous} to {requested}")]
    RegressingNet {
        federate_id: FederateId,
        previous: WireTag,
        requested: WireTag,
    },

    #[error("LTC for federate `{federate_id}` regressed from {previous} to {completed}")]
    RegressingLtc {
        federate_id: FederateId,
        previous: WireTag,
        completed: WireTag,
    },

    #[error("federate `{federate_id}` cannot process {event} while {lifecycle}")]
    InvalidLifecycleTransition {
        federate_id: FederateId,
        event: &'static str,
        lifecycle: &'static str,
    },

    #[error("MSG route {source_federate} -> {target_federate} endpoint `{endpoint}` is not in the RTI topology")]
    InvalidRoute {
        source_federate: FederateId,
        target_federate: FederateId,
        endpoint: EndpointId,
    },
}

/// Deterministic RTI state for static TAG/NET/LTC/MSG coordination.
#[derive(Debug, Clone)]
pub struct RtiState {
    topology: CompiledTopology,
    federates: BTreeMap<FederateId, FederateCoordination>,
}

impl RtiState {
    pub fn new(topology: FederatedTopology) -> Result<Self, RtiError> {
        Ok(Self::from_compiled(CompiledTopology::new(topology)?))
    }

    pub fn from_compiled(topology: CompiledTopology) -> Self {
        let federates = topology
            .federates()
            .map(|(_, federate)| (federate.id.clone(), FederateCoordination::default()))
            .collect();

        Self {
            topology,
            federates,
        }
    }

    pub fn topology(&self) -> &FederatedTopology {
        self.topology.topology()
    }

    pub(crate) fn neighbors_for(&self, federate_id: &FederateId) -> Option<&NeighborStructure> {
        self.topology.neighbors_for(federate_id)
    }

    fn contains_route(
        &self,
        source: &FederateId,
        target: &FederateId,
        endpoint: &EndpointId,
    ) -> bool {
        self.topology.contains_route(source, target, endpoint)
    }

    pub fn handle_from(
        &mut self,
        authenticated_federate: &FederateId,
        message: FederateToRti,
    ) -> Result<Vec<RtiDelivery>, RtiError> {
        self.validate_message(authenticated_federate, &message)?;
        self.handle_validated(message)
    }

    #[cfg(test)]
    fn handle(&mut self, message: FederateToRti) -> Result<Vec<RtiDelivery>, RtiError> {
        let authenticated_federate = match &message {
            FederateToRti::Hello { federate_id, .. }
            | FederateToRti::Net { federate_id, .. }
            | FederateToRti::Ltc { federate_id, .. }
            | FederateToRti::Stop { federate_id } => federate_id.clone(),
            FederateToRti::Msg { source, .. } => source.clone(),
        };
        self.handle_from(&authenticated_federate, message)
    }

    fn handle_validated(&mut self, message: FederateToRti) -> Result<Vec<RtiDelivery>, RtiError> {
        match message {
            FederateToRti::Hello { federate_id, .. } => {
                self.ensure_federate(&federate_id)?;
                Ok(Vec::new())
            }
            FederateToRti::Net { federate_id, tag } => {
                let mut staged = self.coordination(&federate_id)?.clone();
                staged.request(tag);
                let affected = self.net_affected_federates(&federate_id);
                let grants = self.evaluate_grants(&affected, Some((&federate_id, &staged)))?;
                Ok(self.commit_transition(federate_id, staged, grants))
            }
            FederateToRti::Ltc { federate_id, tag } => {
                let mut staged = self.coordination(&federate_id)?.clone();
                if tag > staged.last_completed {
                    staged.last_completed = tag;
                }
                staged.in_transit.retain(|in_transit| *in_transit > tag);
                let affected = self.ltc_affected_federates(&federate_id);
                let grants = self.evaluate_grants(&affected, Some((&federate_id, &staged)))?;
                Ok(self.commit_transition(federate_id, staged, grants))
            }
            FederateToRti::Msg {
                source,
                target,
                endpoint,
                tag,
                payload,
            } => {
                self.record_in_transit_message(&source, &target, tag)?;
                Ok(vec![RtiDelivery::new(
                    target,
                    RtiToFederate::Msg {
                        source,
                        endpoint,
                        tag,
                        payload,
                    },
                )])
            }
            FederateToRti::Stop { federate_id } => {
                let mut staged = self.coordination(&federate_id)?.clone();
                staged.stop();
                let affected = self
                    .topology
                    .transitive_downstream(&federate_id)
                    .iter()
                    .map(|key| self.topology.federate_id(*key).clone())
                    .collect::<Vec<_>>();
                let grants = self.evaluate_grants(&affected, Some((&federate_id, &staged)))?;
                Ok(self.commit_transition(federate_id, staged, grants))
            }
        }
    }

    fn validate_message(
        &self,
        authenticated_federate: &FederateId,
        message: &FederateToRti,
    ) -> Result<(), RtiError> {
        self.ensure_federate(authenticated_federate)?;
        match message {
            FederateToRti::Hello { federate_id, .. } => {
                Self::validate_identity(authenticated_federate, federate_id, "Hello")
            }
            FederateToRti::Net { federate_id, tag } => {
                Self::validate_identity(authenticated_federate, federate_id, "NET")?;
                if *tag == WireTag::NEVER || !is_nonnegative_wire_tag(*tag) {
                    return Err(RtiError::InvalidTag {
                        event: "NET",
                        federate_id: federate_id.clone(),
                        tag: *tag,
                    });
                }
                let state = self
                    .federates
                    .get(federate_id)
                    .expect("authenticated federate must exist");
                match state.lifecycle {
                    FederateLifecycle::Stopped => Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "NET",
                        lifecycle: "stopped",
                    }),
                    FederateLifecycle::Running {
                        next_event: NextEvent::NoFuture,
                    } => Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "NET",
                        lifecycle: "no-future",
                    }),
                    FederateLifecycle::Running { .. } if *tag < state.last_completed => {
                        Err(RtiError::RegressingNet {
                            federate_id: federate_id.clone(),
                            previous: state.last_completed,
                            requested: *tag,
                        })
                    }
                    FederateLifecycle::Running { .. } => Ok(()),
                }
            }
            FederateToRti::Ltc { federate_id, tag } => {
                Self::validate_identity(authenticated_federate, federate_id, "LTC")?;
                Self::validate_finite_tag(federate_id, "LTC", *tag)?;
                let state = self
                    .federates
                    .get(federate_id)
                    .expect("authenticated federate must exist");
                if state.is_stopped() {
                    return Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "LTC",
                        lifecycle: "stopped",
                    });
                }
                if *tag < state.last_completed {
                    return Err(RtiError::RegressingLtc {
                        federate_id: federate_id.clone(),
                        previous: state.last_completed,
                        completed: *tag,
                    });
                }
                Ok(())
            }
            FederateToRti::Msg {
                source,
                target,
                endpoint,
                tag,
                ..
            } => {
                Self::validate_identity(authenticated_federate, source, "MSG")?;
                self.ensure_federate(target)?;
                Self::validate_finite_tag(source, "MSG", *tag)?;
                let source_state = self
                    .federates
                    .get(source)
                    .expect("authenticated federate must exist");
                if source_state.is_stopped() {
                    return Err(RtiError::InvalidLifecycleTransition {
                        federate_id: source.clone(),
                        event: "MSG",
                        lifecycle: "stopped",
                    });
                }
                if !self.contains_route(source, target, endpoint) {
                    return Err(RtiError::InvalidRoute {
                        source_federate: source.clone(),
                        target_federate: target.clone(),
                        endpoint: endpoint.clone(),
                    });
                }
                Ok(())
            }
            FederateToRti::Stop { federate_id } => {
                Self::validate_identity(authenticated_federate, federate_id, "Stop")?;
                let state = self
                    .federates
                    .get(federate_id)
                    .expect("authenticated federate must exist");
                match state.lifecycle {
                    FederateLifecycle::Running {
                        next_event: NextEvent::NoFuture,
                    } => Ok(()),
                    FederateLifecycle::Running { .. } => {
                        Err(RtiError::InvalidLifecycleTransition {
                            federate_id: federate_id.clone(),
                            event: "Stop",
                            lifecycle: "running with future events",
                        })
                    }
                    FederateLifecycle::Stopped => Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "Stop",
                        lifecycle: "stopped",
                    }),
                }
            }
        }
    }

    fn validate_identity(
        authenticated_federate: &FederateId,
        claimed_federate: &FederateId,
        event: &'static str,
    ) -> Result<(), RtiError> {
        if authenticated_federate == claimed_federate {
            Ok(())
        } else {
            Err(RtiError::FederateIdentityMismatch {
                event,
                authenticated_federate: authenticated_federate.clone(),
                claimed_federate: claimed_federate.clone(),
            })
        }
    }

    fn validate_finite_tag(
        federate_id: &FederateId,
        event: &'static str,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        if is_nonnegative_finite_tag(tag) {
            Ok(())
        } else {
            Err(RtiError::InvalidTag {
                event,
                federate_id: federate_id.clone(),
                tag,
            })
        }
    }

    #[cfg(test)]
    fn request_tag(
        &mut self,
        federate_id: &FederateId,
        tag: WireTag,
    ) -> Result<GrantDecision, RtiError> {
        self.ensure_federate(federate_id)?;
        self.federates
            .get_mut(federate_id)
            .expect("federate existence was checked")
            .request(tag);
        self.try_grant_tag(federate_id)
    }

    fn coordination(&self, federate_id: &FederateId) -> Result<&FederateCoordination, RtiError> {
        self.federates
            .get(federate_id)
            .ok_or_else(|| RtiError::UnknownFederate(federate_id.clone()))
    }

    fn record_in_transit_message(
        &mut self,
        source: &FederateId,
        target: &FederateId,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        self.ensure_federate(source)?;
        self.ensure_federate(target)?;
        let state = self
            .federates
            .get_mut(target)
            .expect("federate existence was checked");
        if tag > state.last_completed {
            state.in_transit.insert(tag);
        }
        Ok(())
    }

    #[cfg(test)]
    fn earliest_incoming_message_tag(
        &self,
        federate_id: &FederateId,
    ) -> Result<Option<WireTag>, RtiError> {
        self.earliest_incoming_message_tag_with_override(federate_id, None)
    }

    fn earliest_incoming_message_tag_with_override<'a>(
        &'a self,
        federate_id: &FederateId,
        override_state: Option<(&'a FederateId, &'a FederateCoordination)>,
    ) -> Result<Option<WireTag>, RtiError> {
        let mut earliest = None;

        for dependency in self.topology.transitive_incoming(federate_id) {
            let source = self.topology.federate_id(dependency.source);
            let upstream_state = self.coordination_with_override(source, override_state)?;
            let candidate =
                apply_edge_delay(upstream_state.effective_next_event(), dependency.delay)?;

            if earliest.is_none_or(|current| candidate < current) {
                earliest = Some(candidate);
            }
        }

        Ok(earliest)
    }

    fn coordination_with_override<'a>(
        &'a self,
        federate_id: &FederateId,
        override_state: Option<(&'a FederateId, &'a FederateCoordination)>,
    ) -> Result<&'a FederateCoordination, RtiError> {
        if let Some((override_id, state)) = override_state {
            if override_id == federate_id {
                return Ok(state);
            }
        }
        self.coordination(federate_id)
    }

    fn evaluate_grant_tag<'a>(
        &'a self,
        federate_id: &FederateId,
        override_state: Option<(&'a FederateId, &'a FederateCoordination)>,
    ) -> Result<GrantDecision, RtiError> {
        let state = self.coordination_with_override(federate_id, override_state)?;
        let earliest =
            || self.earliest_incoming_message_tag_with_override(federate_id, override_state);
        if state.is_stopped() {
            return Ok(GrantDecision::Blocked {
                requested: WireTag::Forever,
                earliest_incoming: None,
            });
        }
        let requested = match state.requested_tag() {
            Some(tag) => tag,
            None => {
                return Ok(GrantDecision::Blocked {
                    requested: WireTag::Forever,
                    earliest_incoming: None,
                })
            }
        };

        if requested == WireTag::FOREVER {
            return Ok(GrantDecision::Blocked {
                requested,
                earliest_incoming: earliest()?,
            });
        }

        let requested = state.effective_next_event().min(requested);

        if state
            .last_granted
            .is_some_and(|last_granted| last_granted >= requested)
        {
            return Ok(GrantDecision::AlreadyGranted { tag: requested });
        }

        if self.topology.incoming(federate_id).is_empty() {
            return Ok(GrantDecision::Granted { tag: requested });
        }

        let last_granted = state.last_granted.unwrap_or(WireTag::NEVER);
        let mut minimum_upstream_completed = WireTag::FOREVER;
        for dependency in self.topology.incoming(federate_id) {
            let source = self.topology.federate_id(dependency.source);
            let upstream_state = self.coordination_with_override(source, override_state)?;
            if upstream_state.is_stopped() {
                continue;
            }
            let candidate = apply_edge_delay(upstream_state.last_completed, dependency.delay)?;
            minimum_upstream_completed = minimum_upstream_completed.min(candidate);
        }
        if minimum_upstream_completed > last_granted && minimum_upstream_completed >= requested {
            return Ok(GrantDecision::Granted {
                tag: minimum_upstream_completed,
            });
        }

        let earliest_incoming = earliest()?;
        if let Some(incoming) = earliest_incoming {
            if incoming > requested {
                let safe = latest_tag_strictly_before(incoming)
                    .ok_or(RtiError::TagPredecessorUnderflow { tag: incoming })?;
                if safe > last_granted {
                    return Ok(GrantDecision::Granted { tag: safe });
                }
            }
        }
        Ok(GrantDecision::Blocked {
            requested,
            earliest_incoming,
        })
    }

    #[cfg(test)]
    fn try_grant_tag(&mut self, federate_id: &FederateId) -> Result<GrantDecision, RtiError> {
        let decision = self.evaluate_grant_tag(federate_id, None)?;
        if let GrantDecision::Granted { tag } = decision {
            self.federates
                .get_mut(federate_id)
                .expect("federate existence was checked")
                .last_granted = Some(tag);
        }
        Ok(decision)
    }

    fn net_affected_federates(&self, source: &FederateId) -> Vec<FederateId> {
        let mut affected = vec![source.clone()];
        let source_key = self
            .topology
            .federate_key(source)
            .expect("validated event source must be a topology member");
        affected.extend(
            self.topology
                .transitive_downstream(source)
                .iter()
                .filter(|target| **target != source_key)
                .map(|target| self.topology.federate_id(*target).clone()),
        );
        affected
    }

    fn ltc_affected_federates(&self, source: &FederateId) -> Vec<FederateId> {
        let mut affected = vec![source.clone()];
        let source_key = self
            .topology
            .federate_key(source)
            .expect("validated event source must be a topology member");
        affected.extend(
            self.topology
                .transitive_downstream(source)
                .iter()
                .filter(|target| **target != source_key)
                .map(|target| self.topology.federate_id(*target).clone()),
        );
        affected
    }

    fn evaluate_grants<'a>(
        &'a self,
        affected: &[FederateId],
        override_state: Option<(&'a FederateId, &'a FederateCoordination)>,
    ) -> Result<Vec<(FederateId, GrantDecision)>, RtiError> {
        affected
            .iter()
            .map(|federate_id| {
                self.evaluate_grant_tag(federate_id, override_state)
                    .map(|decision| (federate_id.clone(), decision))
            })
            .collect()
    }

    fn commit_transition(
        &mut self,
        federate_id: FederateId,
        staged: FederateCoordination,
        grants: Vec<(FederateId, GrantDecision)>,
    ) -> Vec<RtiDelivery> {
        self.federates.insert(federate_id, staged);
        let mut deliveries = Vec::new();
        for (grantee, decision) in grants {
            if let GrantDecision::Granted { tag } = decision {
                self.federates
                    .get_mut(&grantee)
                    .expect("affected federate comes from compiled topology")
                    .last_granted = Some(tag);
                deliveries.push(RtiDelivery::new(grantee, RtiToFederate::Tag { tag }));
            }
        }
        deliveries
    }

    fn ensure_federate(&self, federate_id: &FederateId) -> Result<(), RtiError> {
        if self.federates.contains_key(federate_id) {
            Ok(())
        } else {
            Err(RtiError::UnknownFederate(federate_id.clone()))
        }
    }
}

fn apply_edge_delay(tag: WireTag, delay: WireDelay) -> Result<WireTag, RtiError> {
    tag.checked_delay(delay).ok_or(RtiError::TagDelayOverflow {
        tag,
        delay_ns: delay.as_nanos(),
    })
}

fn latest_tag_strictly_before(tag: WireTag) -> Option<WireTag> {
    match tag {
        WireTag::Never => Some(WireTag::Never),
        WireTag::Forever => Some(WireTag::Forever),
        WireTag::Finite {
            offset_ns,
            microstep,
        } => {
            if microstep > 0 {
                Some(WireTag::finite(offset_ns, microstep - 1))
            } else {
                offset_ns
                    .checked_sub(1)
                    .map(|offset_ns| WireTag::finite(offset_ns, u64::MAX))
            }
        }
    }
}

fn is_nonnegative_wire_tag(tag: WireTag) -> bool {
    tag == WireTag::FOREVER || is_nonnegative_finite_tag(tag)
}

fn is_nonnegative_finite_tag(tag: WireTag) -> bool {
    matches!(tag, WireTag::Finite { offset_ns, .. } if offset_ns >= 0)
}

#[cfg(test)]
mod tests;
