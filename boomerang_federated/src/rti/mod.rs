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

    #[error("federate `{federate_id}` acknowledged no in-transit message at {tag}")]
    UnmatchedMessageAck {
        federate_id: FederateId,
        tag: WireTag,
    },
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

        Ok(())
    }

    /// Acknowledge delivery of exactly one message into a federate's scheduler queue.
    pub fn acknowledge_message(
        &mut self,
        federate_id: &FederateId,
        tag: WireTag,
    ) -> Result<(), RtiError> {
        self.ensure_federate(federate_id)?;
        let remove_federate_entry = {
            let messages = self.in_transit.get_mut(federate_id).ok_or_else(|| {
                RtiError::UnmatchedMessageAck {
                    federate_id: federate_id.clone(),
                    tag,
                }
            })?;
            let count = messages
                .get_mut(&tag)
                .ok_or_else(|| RtiError::UnmatchedMessageAck {
                    federate_id: federate_id.clone(),
                    tag,
                })?;
            *count -= 1;
            if *count == 0 {
                messages.remove(&tag);
            }
            messages.is_empty()
        };
        if remove_federate_entry {
            self.in_transit.remove(federate_id);
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
mod tests;
