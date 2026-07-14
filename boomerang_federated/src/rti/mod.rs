use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;

use crate::protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, RtiToFederate, TopologyEdge,
    WireDelay, WireTag,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RouteKey {
    source: FederateId,
    target: FederateId,
    endpoint: EndpointId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IncomingDependency {
    source: FederateId,
    endpoint: EndpointId,
    delay: WireDelay,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct IncomingPath {
    source: FederateId,
    delay: WireDelay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledTopology {
    original: FederatedTopology,
    incoming: BTreeMap<FederateId, Vec<IncomingDependency>>,
    downstream: BTreeMap<FederateId, Vec<FederateId>>,
    transitive_incoming: BTreeMap<FederateId, Vec<IncomingPath>>,
    transitive_downstream: BTreeMap<FederateId, Vec<FederateId>>,
    minimum_delays: BTreeMap<(FederateId, FederateId), WireDelay>,
    routes: BTreeSet<RouteKey>,
}

impl CompiledTopology {
    fn new(topology: FederatedTopology) -> Result<Self, RtiError> {
        let mut members = BTreeSet::new();
        for federate_id in &topology.federates {
            if !members.insert(federate_id.clone()) {
                return Err(RtiError::DuplicateFederate(federate_id.clone()));
            }
        }

        let mut incoming = members
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        let mut downstream_sets = members
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut routes = BTreeSet::new();
        let mut endpoint_routes = BTreeMap::<EndpointId, TopologyEdge>::new();
        let mut minimum_delays = BTreeMap::<(FederateId, FederateId), WireDelay>::new();

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

            routes.insert(RouteKey {
                source: edge.source.clone(),
                target: edge.target.clone(),
                endpoint: edge.endpoint.clone(),
            });
            incoming
                .get_mut(&edge.target)
                .expect("validated topology target must have an incoming index")
                .push(IncomingDependency {
                    source: edge.source.clone(),
                    endpoint: edge.endpoint.clone(),
                    delay: edge.delay,
                });
            downstream_sets
                .get_mut(&edge.source)
                .expect("validated topology source must have a downstream index")
                .insert(edge.target.clone());
            minimum_delays
                .entry((edge.source.clone(), edge.target.clone()))
                .and_modify(|delay| *delay = (*delay).min(edge.delay))
                .or_insert(edge.delay);
        }

        for dependencies in incoming.values_mut() {
            dependencies.sort();
        }
        let downstream = downstream_sets
            .into_iter()
            .map(|(source, targets)| (source, targets.into_iter().collect()))
            .collect::<BTreeMap<_, _>>();

        for _ in 0..members.len() {
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
                                path_source: source.clone(),
                                intermediate: intermediate.clone(),
                                target: target.clone(),
                                first_delay_ns: first.as_nanos(),
                                second_delay_ns: second.as_nanos(),
                            })?;
                    let candidate = WireDelay::from_nanos(nanos);
                    let key = (source.clone(), target.clone());
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

        let mut transitive_incoming = members
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        let mut transitive_downstream_sets = members
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for ((source, target), delay) in &minimum_delays {
            transitive_incoming
                .get_mut(target)
                .expect("minimum-delay target must be a topology member")
                .push(IncomingPath {
                    source: source.clone(),
                    delay: *delay,
                });
            transitive_downstream_sets
                .get_mut(source)
                .expect("minimum-delay source must be a topology member")
                .insert(target.clone());
        }
        let transitive_downstream = transitive_downstream_sets
            .into_iter()
            .map(|(source, targets)| (source, targets.into_iter().collect()))
            .collect();

        Ok(Self {
            original: topology,
            incoming,
            downstream,
            transitive_incoming,
            transitive_downstream,
            minimum_delays,
            routes,
        })
    }

    fn incoming(&self, target: &FederateId) -> &[IncomingDependency] {
        self.incoming.get(target).map_or(&[], Vec::as_slice)
    }

    fn downstream(&self, source: &FederateId) -> &[FederateId] {
        self.downstream.get(source).map_or(&[], Vec::as_slice)
    }

    fn transitive_incoming(&self, target: &FederateId) -> &[IncomingPath] {
        self.transitive_incoming
            .get(target)
            .map_or(&[], Vec::as_slice)
    }

    fn transitive_downstream(&self, source: &FederateId) -> &[FederateId] {
        self.transitive_downstream
            .get(source)
            .map_or(&[], Vec::as_slice)
    }

    fn minimum_delay(&self, source: &FederateId, target: &FederateId) -> Option<WireDelay> {
        self.minimum_delays
            .get(&(source.clone(), target.clone()))
            .copied()
    }

    fn contains_route(
        &self,
        source: &FederateId,
        target: &FederateId,
        endpoint: &EndpointId,
    ) -> bool {
        self.routes.contains(&RouteKey {
            source: source.clone(),
            target: target.clone(),
            endpoint: endpoint.clone(),
        })
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
    in_transit: BTreeMap<WireTag, NonZeroUsize>,
}

impl Default for FederateCoordination {
    fn default() -> Self {
        Self {
            lifecycle: FederateLifecycle::Running {
                next_event: NextEvent::Unknown,
            },
            last_completed: WireTag::Never,
            last_granted: None,
            in_transit: BTreeMap::new(),
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

    #[error("federate `{federate_id}` acknowledged no in-transit message at {tag}")]
    UnmatchedMessageAck {
        federate_id: FederateId,
        tag: WireTag,
    },

    #[error("in-transit message count overflow for federate `{federate_id}` at {tag}")]
    MessageCountOverflow {
        federate_id: FederateId,
        tag: WireTag,
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
        let topology = CompiledTopology::new(topology)?;
        let federates = topology
            .original
            .federates
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, FederateCoordination::default()))
            .collect();

        Ok(Self {
            topology,
            federates,
        })
    }

    pub fn topology(&self) -> &FederatedTopology {
        &self.topology.original
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
            | FederateToRti::MsgAck { federate_id, .. }
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
                self.complete_tag(&federate_id, tag)?;
                Ok(Vec::new())
            }
            FederateToRti::MsgAck { federate_id, tag } => {
                let mut staged = self.coordination(&federate_id)?.clone();
                Self::acknowledge_coordination(&mut staged, tag);
                let affected = [federate_id.clone()];
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
                let affected = self.topology.downstream(&federate_id);
                let grants = self.evaluate_grants(affected, Some((&federate_id, &staged)))?;
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
            FederateToRti::MsgAck { federate_id, tag } => {
                Self::validate_identity(authenticated_federate, federate_id, "MsgAck")?;
                Self::validate_finite_tag(federate_id, "MsgAck", *tag)?;
                let state = self
                    .federates
                    .get(federate_id)
                    .expect("authenticated federate must exist");
                if state.is_stopped() {
                    return Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "MsgAck",
                        lifecycle: "stopped",
                    });
                }
                if !state.in_transit.contains_key(tag) {
                    return Err(RtiError::UnmatchedMessageAck {
                        federate_id: federate_id.clone(),
                        tag: *tag,
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
                let target_state = self
                    .federates
                    .get(target)
                    .expect("validated target federate must exist");
                if !self.contains_route(source, target, endpoint) {
                    return Err(RtiError::InvalidRoute {
                        source_federate: source.clone(),
                        target_federate: target.clone(),
                        endpoint: endpoint.clone(),
                    });
                }
                if target_state
                    .in_transit
                    .get(tag)
                    .is_some_and(|count| count.get() == usize::MAX)
                {
                    return Err(RtiError::MessageCountOverflow {
                        federate_id: target.clone(),
                        tag: *tag,
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

    fn complete_tag(&mut self, federate_id: &FederateId, tag: WireTag) -> Result<(), RtiError> {
        self.ensure_federate(federate_id)?;
        let state = self
            .federates
            .get_mut(federate_id)
            .expect("federate existence was checked");
        if tag > state.last_completed {
            state.last_completed = tag;
        }

        Ok(())
    }

    /// Acknowledge delivery of exactly one message into a federate's scheduler queue.
    #[cfg(test)]
    fn acknowledge_message(
        &mut self,
        federate_id: &FederateId,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        self.ensure_federate(federate_id)?;
        let state = self
            .federates
            .get_mut(federate_id)
            .expect("federate existence was checked");
        if !state.in_transit.contains_key(&tag) {
            return Err(RtiError::UnmatchedMessageAck {
                federate_id: federate_id.clone(),
                tag,
            });
        }
        Self::acknowledge_coordination(state, tag);
        Ok(())
    }

    fn acknowledge_coordination(state: &mut FederateCoordination, tag: WireTag) {
        let count = state
            .in_transit
            .get_mut(&tag)
            .expect("acknowledgment was validated against staged coordination");
        match NonZeroUsize::new(count.get() - 1) {
            Some(remaining) => *count = remaining,
            None => {
                state.in_transit.remove(&tag);
            }
        }
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
        match state.in_transit.get_mut(&tag) {
            Some(count) => {
                let next =
                    count
                        .get()
                        .checked_add(1)
                        .ok_or_else(|| RtiError::MessageCountOverflow {
                            federate_id: target.clone(),
                            tag,
                        })?;
                *count = NonZeroUsize::new(next).expect("positive count remains nonzero");
            }
            None => {
                state
                    .in_transit
                    .insert(tag, NonZeroUsize::new(1).expect("one is nonzero"));
            }
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
        let state = self.coordination_with_override(federate_id, override_state)?;
        let mut earliest = state.in_transit.keys().next().copied();

        for dependency in self.topology.incoming(federate_id) {
            let upstream_state =
                self.coordination_with_override(&dependency.source, override_state)?;
            let candidate =
                apply_edge_delay(upstream_state.advertised_next_event(), dependency.delay)?;

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
                earliest_incoming: earliest()?,
            });
        }
        let requested = match state.requested_tag() {
            Some(tag) => tag,
            None => {
                return Ok(GrantDecision::Blocked {
                    requested: WireTag::Forever,
                    earliest_incoming: earliest()?,
                })
            }
        };

        if requested == WireTag::FOREVER {
            return Ok(GrantDecision::Blocked {
                requested,
                earliest_incoming: earliest()?,
            });
        }

        if state
            .last_granted
            .is_some_and(|last_granted| last_granted >= requested)
        {
            return Ok(GrantDecision::AlreadyGranted { tag: requested });
        }

        let earliest_incoming = earliest()?;
        if earliest_incoming.is_none_or(|incoming| incoming > requested) {
            Ok(GrantDecision::Granted { tag: requested })
        } else {
            Ok(GrantDecision::Blocked {
                requested,
                earliest_incoming,
            })
        }
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
        affected.extend(
            self.topology
                .downstream(source)
                .iter()
                .filter(|target| *target != source)
                .cloned(),
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

fn is_nonnegative_wire_tag(tag: WireTag) -> bool {
    tag == WireTag::FOREVER || is_nonnegative_finite_tag(tag)
}

fn is_nonnegative_finite_tag(tag: WireTag) -> bool {
    matches!(tag, WireTag::Finite { offset_ns, .. } if offset_ns >= 0)
}

#[cfg(test)]
mod tests;
