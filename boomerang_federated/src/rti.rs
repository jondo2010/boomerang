use std::collections::BTreeMap;

use crate::protocol::{
    FederateId, FederateToRti, FederatedTopology, RtiToFederate, WireDelay, WireTag,
};

/// Per-federate control-plane state known by the RTI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederateState {
    pub last_completed: WireTag,
    pub last_granted: Option<WireTag>,
    pub next_event: Option<WireTag>,
    pub stopped: bool,
}

impl Default for FederateState {
    fn default() -> Self {
        Self {
            last_completed: WireTag::Never,
            last_granted: None,
            next_event: None,
            stopped: false,
        }
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

    #[error("delaying tag {tag} by {delay_ns}ns overflowed")]
    TagDelayOverflow { tag: WireTag, delay_ns: u64 },
}

/// Deterministic RTI state for static TAG/NET/LTC/MSG coordination.
#[derive(Debug, Clone)]
pub struct RtiState {
    topology: FederatedTopology,
    federates: BTreeMap<FederateId, FederateState>,
    in_transit: BTreeMap<FederateId, BTreeMap<WireTag, usize>>,
}

impl RtiState {
    pub fn new(mut topology: FederatedTopology) -> Self {
        for edge in topology.edges.clone() {
            topology.add_federate(edge.source);
            topology.add_federate(edge.target);
        }

        let federates = topology
            .federates
            .iter()
            .cloned()
            .map(|federate_id| (federate_id, FederateState::default()))
            .collect();

        Self {
            topology,
            federates,
            in_transit: BTreeMap::new(),
        }
    }

    pub fn topology(&self) -> &FederatedTopology {
        &self.topology
    }

    pub fn federate_state(&self, federate_id: &FederateId) -> Option<&FederateState> {
        self.federates.get(federate_id)
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
                state.next_event = Some(WireTag::FOREVER);
                state.stopped = true;
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
            .next_event = Some(tag);
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

        if let Some(messages) = self.in_transit.get_mut(federate_id) {
            let completed_tags: Vec<_> = messages.range(..=tag).map(|(&tag, _)| tag).collect();
            for completed_tag in completed_tags {
                messages.remove(&completed_tag);
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
        *self
            .in_transit
            .entry(target.clone())
            .or_default()
            .entry(tag)
            .or_default() += 1;
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

        for edge in self.topology.incoming_edges(federate_id) {
            let upstream_state = self
                .federates
                .get(&edge.source)
                .ok_or_else(|| RtiError::UnknownFederate(edge.source.clone()))?;
            let candidate = match upstream_state.next_event {
                Some(tag) => apply_edge_delay(tag, edge.delay)?,
                None => WireTag::Never,
            };

            if earliest.map_or(true, |current| candidate < current) {
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
        if state.stopped {
            return Ok(GrantDecision::Blocked {
                requested: state.next_event.unwrap_or(WireTag::FOREVER),
                earliest_incoming: self.earliest_incoming_message_tag(federate_id)?,
            });
        }
        let requested = match state.next_event {
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
        if earliest_incoming.map_or(true, |earliest| earliest > requested) {
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
        self.in_transit
            .get(federate_id)
            .and_then(|messages| messages.keys().next().copied())
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
mod tests {
    use super::*;
    use crate::protocol::{EndpointId, FederatedTopology, TopologyEdge};

    fn fed(id: &str) -> FederateId {
        FederateId::new(id)
    }

    fn endpoint(id: &str) -> EndpointId {
        EndpointId::new(id)
    }

    fn topology_with_edge(delay: WireDelay) -> FederatedTopology {
        FederatedTopology::with_edges(
            [fed("source"), fed("target")],
            [TopologyEdge::new(
                fed("source"),
                fed("target"),
                endpoint("source.out->target.in"),
                delay,
            )],
        )
    }

    #[test]
    fn grants_tag_when_federate_has_no_upstream_or_in_transit_messages() {
        let mut rti = RtiState::new(FederatedTopology::new([fed("solo")]));

        let decision = rti.request_tag(&fed("solo"), WireTag::ZERO).unwrap();

        assert_eq!(decision, GrantDecision::Granted { tag: WireTag::ZERO });
    }

    #[test]
    fn upstream_net_at_requested_tag_blocks_tag_grant() {
        let mut rti = RtiState::new(topology_with_edge(WireDelay::ZERO));

        assert!(matches!(
            rti.request_tag(&fed("source"), WireTag::ZERO).unwrap(),
            GrantDecision::Granted { .. }
        ));

        let blocked = rti.request_tag(&fed("target"), WireTag::ZERO).unwrap();
        assert_eq!(
            blocked,
            GrantDecision::Blocked {
                requested: WireTag::ZERO,
                earliest_incoming: Some(WireTag::ZERO),
            }
        );

        assert!(matches!(
            rti.request_tag(&fed("source"), WireTag::finite(0, 1))
                .unwrap(),
            GrantDecision::Granted { .. }
        ));
        assert_eq!(
            rti.request_tag(&fed("target"), WireTag::ZERO).unwrap(),
            GrantDecision::Granted { tag: WireTag::ZERO }
        );
    }

    #[test]
    fn in_transit_message_blocks_grant_until_target_ltc_acknowledges_the_message_tag() {
        let mut rti = RtiState::new(FederatedTopology::new([fed("source"), fed("target")]));

        rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::finite(5, 0))
            .unwrap();

        assert_eq!(
            rti.request_tag(&fed("target"), WireTag::finite(10, 0))
                .unwrap(),
            GrantDecision::Blocked {
                requested: WireTag::finite(10, 0),
                earliest_incoming: Some(WireTag::finite(5, 0)),
            }
        );

        rti.complete_tag(&fed("target"), WireTag::finite(5, 0))
            .unwrap();

        assert_eq!(
            rti.request_tag(&fed("target"), WireTag::finite(10, 0))
                .unwrap(),
            GrantDecision::Granted {
                tag: WireTag::finite(10, 0),
            }
        );
    }

    #[test]
    fn ltc_can_trigger_pending_grant_after_in_transit_message_is_acknowledged() {
        let mut rti = RtiState::new(FederatedTopology::new([fed("source"), fed("target")]));
        rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::finite(5, 0))
            .unwrap();
        assert!(matches!(
            rti.request_tag(&fed("target"), WireTag::finite(10, 0))
                .unwrap(),
            GrantDecision::Blocked { .. }
        ));

        let deliveries = rti
            .handle(FederateToRti::Ltc {
                federate_id: fed("target"),
                tag: WireTag::finite(5, 0),
            })
            .unwrap();

        assert_eq!(
            deliveries,
            vec![RtiDelivery {
                federate_id: fed("target"),
                message: RtiToFederate::Tag {
                    tag: WireTag::finite(10, 0),
                },
            }]
        );
    }

    #[test]
    fn net_forever_unblocks_pending_downstream_without_granting_forever() {
        let mut rti = RtiState::new(topology_with_edge(WireDelay::ZERO));

        assert_eq!(
            rti.handle(FederateToRti::Net {
                federate_id: fed("target"),
                tag: WireTag::finite(10, 0),
            })
            .unwrap(),
            Vec::new()
        );

        let deliveries = rti
            .handle(FederateToRti::Net {
                federate_id: fed("source"),
                tag: WireTag::FOREVER,
            })
            .unwrap();

        assert_eq!(
            deliveries,
            vec![RtiDelivery {
                federate_id: fed("target"),
                message: RtiToFederate::Tag {
                    tag: WireTag::finite(10, 0),
                },
            }]
        );
        assert_eq!(
            rti.federate_state(&fed("source")).unwrap().next_event,
            Some(WireTag::FOREVER)
        );
        assert_eq!(
            rti.federate_state(&fed("source")).unwrap().last_granted,
            None
        );
    }

    #[test]
    fn stop_marks_federate_no_future_and_unblocks_pending_downstream() {
        let mut rti = RtiState::new(topology_with_edge(WireDelay::ZERO));

        assert_eq!(
            rti.handle(FederateToRti::Net {
                federate_id: fed("target"),
                tag: WireTag::finite(10, 0),
            })
            .unwrap(),
            Vec::new()
        );

        let deliveries = rti
            .handle(FederateToRti::Stop {
                federate_id: fed("source"),
            })
            .unwrap();

        assert_eq!(
            deliveries,
            vec![RtiDelivery {
                federate_id: fed("target"),
                message: RtiToFederate::Tag {
                    tag: WireTag::finite(10, 0),
                },
            }]
        );
        let source_state = rti.federate_state(&fed("source")).unwrap();
        assert!(source_state.stopped);
        assert_eq!(source_state.next_event, Some(WireTag::FOREVER));
        assert_eq!(source_state.last_granted, None);
    }

    #[test]
    fn topology_delays_shift_earliest_incoming_message_tags() {
        let mut rti = RtiState::new(topology_with_edge(WireDelay::from_nanos(10)));

        rti.request_tag(&fed("source"), WireTag::ZERO).unwrap();

        assert_eq!(
            rti.earliest_incoming_message_tag(&fed("target")).unwrap(),
            Some(WireTag::finite(10, 0))
        );
        assert_eq!(
            rti.request_tag(&fed("target"), WireTag::finite(9, 0))
                .unwrap(),
            GrantDecision::Granted {
                tag: WireTag::finite(9, 0),
            }
        );
        assert_eq!(
            rti.request_tag(&fed("target"), WireTag::finite(10, 0))
                .unwrap(),
            GrantDecision::Blocked {
                requested: WireTag::finite(10, 0),
                earliest_incoming: Some(WireTag::finite(10, 0)),
            }
        );
    }

    #[test]
    fn msg_frames_are_recorded_as_in_transit_and_forwarded_to_the_target() {
        let mut rti = RtiState::new(FederatedTopology::new([fed("source"), fed("target")]));

        let deliveries = rti
            .handle(FederateToRti::Msg {
                source: fed("source"),
                target: fed("target"),
                endpoint: endpoint("source.out->target.in"),
                tag: WireTag::finite(3, 0),
                payload: vec![1, 2, 3],
            })
            .unwrap();

        assert_eq!(
            rti.earliest_incoming_message_tag(&fed("target")).unwrap(),
            Some(WireTag::finite(3, 0))
        );
        assert_eq!(
            deliveries,
            vec![RtiDelivery {
                federate_id: fed("target"),
                message: RtiToFederate::Msg {
                    source: fed("source"),
                    endpoint: endpoint("source.out->target.in"),
                    tag: WireTag::finite(3, 0),
                    payload: vec![1, 2, 3],
                },
            }]
        );
    }
}
