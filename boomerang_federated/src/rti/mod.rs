use std::collections::BTreeSet;

use crate::protocol::{EndpointId, FederateId, FederateToRti, RtiToFederate, WireDelay, WireTag};

mod graph;
mod index;

pub use graph::RtiGraph;
#[doc(hidden)]
pub use graph::{RtiEndpointParts, RtiFederateParts, RtiGraphParts};
use index::EndpointKey;
pub(crate) use index::FederateKey;

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

    #[error("delaying tag {tag} by {delay_ns}ns overflowed")]
    TagDelayOverflow { tag: WireTag, delay_ns: u64 },

    #[error("cannot calculate the latest tag strictly before {tag}")]
    TagPredecessorUnderflow { tag: WireTag },

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

/// Fully validated RTI input expressed only in process-local dense identities.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedRtiEvent {
    Hello {
        federate: FederateKey,
    },
    Net {
        federate: FederateKey,
        tag: WireTag,
    },
    Ltc {
        federate: FederateKey,
        tag: WireTag,
    },
    Msg {
        source: FederateKey,
        target: FederateKey,
        endpoint: EndpointKey,
        tag: WireTag,
        payload: Vec<u8>,
    },
    Stop {
        federate: FederateKey,
    },
}

#[derive(Debug, Clone)]
struct RtiRuntimeState {
    federates: tinymap::TinySecondaryMap<FederateKey, FederateCoordination>,
}

/// Deterministic RTI state for static TAG/NET/LTC/MSG coordination.
#[derive(Debug)]
pub struct RtiState {
    graph: RtiGraph,
    runtime: RtiRuntimeState,
}

impl RtiState {
    pub fn from_graph(graph: RtiGraph) -> Self {
        let federates = graph
            .federates()
            .map(|(key, _)| (key, FederateCoordination::default()))
            .collect();

        Self {
            graph,
            runtime: RtiRuntimeState { federates },
        }
    }

    pub fn graph(&self) -> &RtiGraph {
        &self.graph
    }

    pub(crate) fn federate_key(&self, federate_id: &FederateId) -> Option<FederateKey> {
        self.graph.federate_key(federate_id)
    }

    pub fn handle_from(
        &mut self,
        authenticated_federate: &FederateId,
        message: FederateToRti,
    ) -> Result<Vec<RtiDelivery>, RtiError> {
        let authenticated_federate = self.resolve_federate(authenticated_federate)?;
        self.handle_from_key(authenticated_federate, message)
    }

    pub(crate) fn handle_from_key(
        &mut self,
        authenticated_federate: FederateKey,
        message: FederateToRti,
    ) -> Result<Vec<RtiDelivery>, RtiError> {
        let event = self.validate_message(authenticated_federate, message)?;
        self.handle_validated(event)
    }

    #[cfg(test)]
    fn handle(&mut self, message: FederateToRti) -> Result<Vec<RtiDelivery>, RtiError> {
        let authenticated_federate = match &message {
            FederateToRti::Hello { federate_id }
            | FederateToRti::Net { federate_id, .. }
            | FederateToRti::Ltc { federate_id, .. }
            | FederateToRti::Stop { federate_id } => federate_id.clone(),
            FederateToRti::Msg { source, .. } => source.clone(),
        };
        self.handle_from(&authenticated_federate, message)
    }

    fn handle_validated(&mut self, event: ResolvedRtiEvent) -> Result<Vec<RtiDelivery>, RtiError> {
        match event {
            ResolvedRtiEvent::Hello { federate } => {
                debug_assert!(self.runtime.federates.contains_key(federate));
                Ok(Vec::new())
            }
            ResolvedRtiEvent::Net { federate, tag } => {
                let mut staged = self.coordination(federate).clone();
                staged.request(tag);
                let affected = self.net_affected_federates(federate);
                let grants = self.evaluate_grants(&affected, Some((federate, &staged)))?;
                Ok(self.commit_transition(federate, staged, grants))
            }
            ResolvedRtiEvent::Ltc { federate, tag } => {
                let mut staged = self.coordination(federate).clone();
                if tag > staged.last_completed {
                    staged.last_completed = tag;
                }
                staged.in_transit.retain(|in_transit| *in_transit > tag);
                let affected = self.ltc_affected_federates(federate);
                let grants = self.evaluate_grants(&affected, Some((federate, &staged)))?;
                Ok(self.commit_transition(federate, staged, grants))
            }
            ResolvedRtiEvent::Msg {
                source,
                target,
                endpoint,
                tag,
                payload,
            } => {
                self.record_in_transit_message_key(target, tag);
                let source_id = self.graph.federate_id(source).clone();
                let target_id = self.graph.federate_id(target).clone();
                let endpoint_id = self.graph.endpoint_id(endpoint).clone();
                Ok(vec![RtiDelivery::new(
                    target_id,
                    RtiToFederate::Msg {
                        source: source_id,
                        endpoint: endpoint_id,
                        tag,
                        payload,
                    },
                )])
            }
            ResolvedRtiEvent::Stop { federate } => {
                let mut staged = self.coordination(federate).clone();
                staged.stop();
                let affected = self.graph.affected_downstream(federate).to_vec();
                let grants = self.evaluate_grants(&affected, Some((federate, &staged)))?;
                Ok(self.commit_transition(federate, staged, grants))
            }
        }
    }

    fn validate_message(
        &self,
        authenticated_federate: FederateKey,
        message: FederateToRti,
    ) -> Result<ResolvedRtiEvent, RtiError> {
        debug_assert!(self.runtime.federates.contains_key(authenticated_federate));
        match message {
            FederateToRti::Hello { federate_id } => {
                self.validate_identity(authenticated_federate, &federate_id, "Hello")?;
                Ok(ResolvedRtiEvent::Hello {
                    federate: authenticated_federate,
                })
            }
            FederateToRti::Net { federate_id, tag } => {
                self.validate_identity(authenticated_federate, &federate_id, "NET")?;
                if tag == WireTag::NEVER || !is_nonnegative_wire_tag(tag) {
                    return Err(RtiError::InvalidTag {
                        event: "NET",
                        federate_id: federate_id.clone(),
                        tag,
                    });
                }
                let state = self.coordination(authenticated_federate);
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
                    FederateLifecycle::Running { .. } if tag < state.last_completed => {
                        Err(RtiError::RegressingNet {
                            federate_id: federate_id.clone(),
                            previous: state.last_completed,
                            requested: tag,
                        })
                    }
                    FederateLifecycle::Running { .. } => Ok(ResolvedRtiEvent::Net {
                        federate: authenticated_federate,
                        tag,
                    }),
                }
            }
            FederateToRti::Ltc { federate_id, tag } => {
                self.validate_identity(authenticated_federate, &federate_id, "LTC")?;
                Self::validate_finite_tag(&federate_id, "LTC", tag)?;
                let state = self.coordination(authenticated_federate);
                if state.is_stopped() {
                    return Err(RtiError::InvalidLifecycleTransition {
                        federate_id: federate_id.clone(),
                        event: "LTC",
                        lifecycle: "stopped",
                    });
                }
                if tag < state.last_completed {
                    return Err(RtiError::RegressingLtc {
                        federate_id: federate_id.clone(),
                        previous: state.last_completed,
                        completed: tag,
                    });
                }
                Ok(ResolvedRtiEvent::Ltc {
                    federate: authenticated_federate,
                    tag,
                })
            }
            FederateToRti::Msg {
                source,
                target,
                endpoint,
                tag,
                payload,
            } => {
                self.validate_identity(authenticated_federate, &source, "MSG")?;
                let target_key = self.resolve_federate(&target)?;
                Self::validate_finite_tag(&source, "MSG", tag)?;
                let source_state = self.coordination(authenticated_federate);
                if source_state.is_stopped() {
                    return Err(RtiError::InvalidLifecycleTransition {
                        federate_id: source.clone(),
                        event: "MSG",
                        lifecycle: "stopped",
                    });
                }
                let Some(endpoint_key) = self.graph.endpoint_key(&endpoint) else {
                    return Err(RtiError::InvalidRoute {
                        source_federate: source.clone(),
                        target_federate: target.clone(),
                        endpoint: endpoint.clone(),
                    });
                };
                let compiled_endpoint = self.graph.endpoint(endpoint_key);
                if compiled_endpoint.source != authenticated_federate
                    || compiled_endpoint.target != target_key
                {
                    return Err(RtiError::InvalidRoute {
                        source_federate: source,
                        target_federate: target,
                        endpoint,
                    });
                }
                Ok(ResolvedRtiEvent::Msg {
                    source: authenticated_federate,
                    target: target_key,
                    endpoint: endpoint_key,
                    tag,
                    payload,
                })
            }
            FederateToRti::Stop { federate_id } => {
                self.validate_identity(authenticated_federate, &federate_id, "Stop")?;
                let state = self.coordination(authenticated_federate);
                match state.lifecycle {
                    FederateLifecycle::Running {
                        next_event: NextEvent::NoFuture,
                    } => Ok(ResolvedRtiEvent::Stop {
                        federate: authenticated_federate,
                    }),
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
        &self,
        authenticated_federate: FederateKey,
        claimed_federate: &FederateId,
        event: &'static str,
    ) -> Result<(), RtiError> {
        let authenticated_id = self.graph.federate_id(authenticated_federate);
        if authenticated_id == claimed_federate {
            Ok(())
        } else {
            Err(RtiError::FederateIdentityMismatch {
                event,
                authenticated_federate: authenticated_id.clone(),
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
        let federate = self.resolve_federate(federate_id)?;
        self.runtime
            .federates
            .get_mut(federate)
            .expect("resolved federate key must have coordination state")
            .request(tag);
        self.try_grant_tag(federate)
    }

    fn coordination(&self, federate: FederateKey) -> &FederateCoordination {
        self.runtime
            .federates
            .get(federate)
            .expect("compiled federate key must have coordination state")
    }

    fn record_in_transit_message_key(&mut self, target: FederateKey, tag: WireTag) {
        let state = self
            .runtime
            .federates
            .get_mut(target)
            .expect("resolved target key must have coordination state");
        if tag > state.last_completed {
            state.in_transit.insert(tag);
        }
    }

    #[cfg(test)]
    fn record_in_transit_message(
        &mut self,
        source: &FederateId,
        target: &FederateId,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        self.resolve_federate(source)?;
        let target = self.resolve_federate(target)?;
        self.record_in_transit_message_key(target, tag);
        Ok(())
    }

    #[cfg(test)]
    fn earliest_incoming_message_tag(
        &self,
        federate_id: &FederateId,
    ) -> Result<Option<WireTag>, RtiError> {
        let federate = self.resolve_federate(federate_id)?;
        self.earliest_incoming_message_tag_with_override(federate, None)
    }

    fn earliest_incoming_message_tag_with_override<'a>(
        &'a self,
        federate: FederateKey,
        override_state: Option<(FederateKey, &'a FederateCoordination)>,
    ) -> Result<Option<WireTag>, RtiError> {
        let mut earliest = None;

        for dependency in self.graph.transitive_incoming(federate) {
            let upstream_state = self.coordination_with_override(dependency.source, override_state);
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
        federate: FederateKey,
        override_state: Option<(FederateKey, &'a FederateCoordination)>,
    ) -> &'a FederateCoordination {
        if let Some((override_key, state)) = override_state {
            if override_key == federate {
                return state;
            }
        }
        self.coordination(federate)
    }

    fn evaluate_grant_tag<'a>(
        &'a self,
        federate: FederateKey,
        override_state: Option<(FederateKey, &'a FederateCoordination)>,
    ) -> Result<GrantDecision, RtiError> {
        let state = self.coordination_with_override(federate, override_state);
        let earliest =
            || self.earliest_incoming_message_tag_with_override(federate, override_state);
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

        if self.graph.incoming(federate).is_empty() {
            return Ok(GrantDecision::Granted { tag: requested });
        }

        let last_granted = state.last_granted.unwrap_or(WireTag::NEVER);
        let mut minimum_upstream_completed = WireTag::FOREVER;
        for dependency in self.graph.incoming(federate) {
            let upstream_state = self.coordination_with_override(dependency.source, override_state);
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
    fn try_grant_tag(&mut self, federate: FederateKey) -> Result<GrantDecision, RtiError> {
        let decision = self.evaluate_grant_tag(federate, None)?;
        if let GrantDecision::Granted { tag } = decision {
            self.runtime
                .federates
                .get_mut(federate)
                .expect("resolved federate key must have coordination state")
                .last_granted = Some(tag);
        }
        Ok(decision)
    }

    fn net_affected_federates(&self, source: FederateKey) -> Vec<FederateKey> {
        let mut affected = vec![source];
        affected.extend(
            self.graph
                .affected_downstream(source)
                .iter()
                .filter(|target| **target != source)
                .copied(),
        );
        affected
    }

    fn ltc_affected_federates(&self, source: FederateKey) -> Vec<FederateKey> {
        self.net_affected_federates(source)
    }

    fn evaluate_grants<'a>(
        &'a self,
        affected: &[FederateKey],
        override_state: Option<(FederateKey, &'a FederateCoordination)>,
    ) -> Result<Vec<(FederateKey, GrantDecision)>, RtiError> {
        affected
            .iter()
            .map(|federate| {
                self.evaluate_grant_tag(*federate, override_state)
                    .map(|decision| (*federate, decision))
            })
            .collect()
    }

    fn commit_transition(
        &mut self,
        federate: FederateKey,
        staged: FederateCoordination,
        grants: Vec<(FederateKey, GrantDecision)>,
    ) -> Vec<RtiDelivery> {
        self.runtime.federates.insert(federate, staged);
        let mut deliveries = Vec::new();
        for (grantee, decision) in grants {
            if let GrantDecision::Granted { tag } = decision {
                self.runtime
                    .federates
                    .get_mut(grantee)
                    .expect("affected federate comes from the RTI graph")
                    .last_granted = Some(tag);
                deliveries.push(RtiDelivery::new(
                    self.graph.federate_id(grantee).clone(),
                    RtiToFederate::Tag { tag },
                ));
            }
        }
        deliveries
    }

    fn resolve_federate(&self, federate_id: &FederateId) -> Result<FederateKey, RtiError> {
        self.graph
            .federate_key(federate_id)
            .ok_or_else(|| RtiError::UnknownFederate(federate_id.clone()))
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
pub(crate) fn test_graph(
    federates: impl IntoIterator<Item = RtiFederateParts>,
    endpoints: impl IntoIterator<Item = RtiEndpointParts>,
) -> RtiGraph {
    RtiGraph::from_lowered(RtiGraphParts {
        federates: federates.into_iter().collect(),
        endpoints: endpoints.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests;
