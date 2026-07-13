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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledTopology {
    original: FederatedTopology,
    incoming: BTreeMap<FederateId, Vec<IncomingDependency>>,
    downstream: BTreeMap<FederateId, Vec<FederateId>>,
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
        }

        for dependencies in incoming.values_mut() {
            dependencies.sort();
        }
        let downstream = downstream_sets
            .into_iter()
            .map(|(source, targets)| (source, targets.into_iter().collect()))
            .collect();

        Ok(Self {
            original: topology,
            incoming,
            downstream,
            routes,
        })
    }

    fn incoming(&self, target: &FederateId) -> &[IncomingDependency] {
        self.incoming.get(target).map_or(&[], Vec::as_slice)
    }

    #[allow(
        dead_code,
        reason = "consumed by the affected-federate reevaluation milestone"
    )]
    fn downstream(&self, source: &FederateId) -> &[FederateId] {
        self.downstream.get(source).map_or(&[], Vec::as_slice)
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
pub enum GrantDecision {
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

    pub(crate) fn contains_route(
        &self,
        source: &FederateId,
        target: &FederateId,
        endpoint: &EndpointId,
    ) -> bool {
        self.topology.contains_route(source, target, endpoint)
    }

    pub fn handle(&mut self, message: FederateToRti) -> Result<Vec<RtiDelivery>, RtiError> {
        match message {
            FederateToRti::Hello { federate_id, .. } => {
                self.ensure_federate(&federate_id)?;
                Ok(Vec::new())
            }
            FederateToRti::Net { federate_id, tag } => {
                let decision = self.request_tag(&federate_id, tag)?;
                let mut deliveries = Self::grant_delivery(federate_id, decision)
                    .into_iter()
                    .collect::<Vec<_>>();
                deliveries.extend(self.try_grants_for_all()?);
                Ok(deliveries)
            }
            FederateToRti::Ltc { federate_id, tag } => {
                self.complete_tag(&federate_id, tag)?;
                self.try_grants_for_all()
            }
            FederateToRti::MsgAck { federate_id, tag } => {
                self.acknowledge_message(&federate_id, tag)?;
                self.try_grants_for_all()
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
                self.ensure_federate(&federate_id)?;
                let state = self
                    .federates
                    .get_mut(&federate_id)
                    .expect("federate existence was checked");
                state.stop();
                self.try_grants_for_all()
            }
        }
    }

    pub fn request_tag(
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

    pub fn complete_tag(&mut self, federate_id: &FederateId, tag: WireTag) -> Result<(), RtiError> {
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
    pub fn acknowledge_message(
        &mut self,
        federate_id: &FederateId,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        self.ensure_federate(federate_id)?;
        let state = self
            .federates
            .get_mut(federate_id)
            .expect("federate existence was checked");
        let count =
            state
                .in_transit
                .get_mut(&tag)
                .ok_or_else(|| RtiError::UnmatchedMessageAck {
                    federate_id: federate_id.clone(),
                    tag,
                })?;
        match NonZeroUsize::new(count.get() - 1) {
            Some(remaining) => *count = remaining,
            None => {
                state.in_transit.remove(&tag);
            }
        }
        Ok(())
    }

    pub fn record_in_transit_message(
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

    pub fn can_grant_tag(
        &self,
        federate_id: &FederateId,
        requested: WireTag,
    ) -> Result<bool, RtiError> {
        self.ensure_federate(federate_id)?;
        Ok(match self.earliest_incoming_message_tag(federate_id)? {
            Some(earliest) => earliest > requested,
            None => true,
        })
    }

    pub fn earliest_incoming_message_tag(
        &self,
        federate_id: &FederateId,
    ) -> Result<Option<WireTag>, RtiError> {
        self.ensure_federate(federate_id)?;
        let mut earliest = self.earliest_in_transit_message_tag(federate_id);

        for dependency in self.topology.incoming(federate_id) {
            let upstream_state = self
                .federates
                .get(&dependency.source)
                .ok_or_else(|| RtiError::UnknownFederate(dependency.source.clone()))?;
            let candidate =
                apply_edge_delay(upstream_state.advertised_next_event(), dependency.delay)?;

            if earliest.is_none_or(|current| candidate < current) {
                earliest = Some(candidate);
            }
        }

        Ok(earliest)
    }

    fn try_grant_tag(&mut self, federate_id: &FederateId) -> Result<GrantDecision, RtiError> {
        let state = self
            .federates
            .get(federate_id)
            .ok_or_else(|| RtiError::UnknownFederate(federate_id.clone()))?;
        if state.is_stopped() {
            return Ok(GrantDecision::Blocked {
                requested: WireTag::Forever,
                earliest_incoming: self.earliest_incoming_message_tag(federate_id)?,
            });
        }
        let requested = match state.requested_tag() {
            Some(tag) => tag,
            None => {
                return Ok(GrantDecision::Blocked {
                    requested: WireTag::Forever,
                    earliest_incoming: self.earliest_incoming_message_tag(federate_id)?,
                })
            }
        };

        if requested == WireTag::FOREVER {
            return Ok(GrantDecision::Blocked {
                requested,
                earliest_incoming: self.earliest_incoming_message_tag(federate_id)?,
            });
        }

        if state
            .last_granted
            .is_some_and(|last_granted| last_granted >= requested)
        {
            return Ok(GrantDecision::AlreadyGranted { tag: requested });
        }

        let earliest_incoming = self.earliest_incoming_message_tag(federate_id)?;
        if earliest_incoming.is_none_or(|earliest| earliest > requested) {
            self.federates
                .get_mut(federate_id)
                .expect("federate existence was checked")
                .last_granted = Some(requested);
            Ok(GrantDecision::Granted { tag: requested })
        } else {
            Ok(GrantDecision::Blocked {
                requested,
                earliest_incoming,
            })
        }
    }

    fn try_grants_for_all(&mut self) -> Result<Vec<RtiDelivery>, RtiError> {
        let federate_ids: Vec<_> = self.federates.keys().cloned().collect();
        let mut deliveries = Vec::new();

        for federate_id in federate_ids {
            let decision = self.try_grant_tag(&federate_id)?;
            if let Some(delivery) = Self::grant_delivery(federate_id, decision) {
                deliveries.push(delivery);
            }
        }

        Ok(deliveries)
    }

    fn earliest_in_transit_message_tag(&self, federate_id: &FederateId) -> Option<WireTag> {
        self.federates
            .get(federate_id)
            .and_then(|state| state.in_transit.keys().next().copied())
    }

    fn ensure_federate(&self, federate_id: &FederateId) -> Result<(), RtiError> {
        if self.federates.contains_key(federate_id) {
            Ok(())
        } else {
            Err(RtiError::UnknownFederate(federate_id.clone()))
        }
    }

    fn grant_delivery(federate_id: FederateId, decision: GrantDecision) -> Option<RtiDelivery> {
        match decision {
            GrantDecision::Granted { tag } => {
                Some(RtiDelivery::new(federate_id, RtiToFederate::Tag { tag }))
            }
            GrantDecision::AlreadyGranted { .. } | GrantDecision::Blocked { .. } => None,
        }
    }
}

fn apply_edge_delay(tag: WireTag, delay: WireDelay) -> Result<WireTag, RtiError> {
    tag.checked_delay(delay).ok_or(RtiError::TagDelayOverflow {
        tag,
        delay_ns: delay.as_nanos(),
    })
}

#[cfg(test)]
mod tests;
