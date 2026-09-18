use super::*;
use boomerang_runtime::image::{
    BoundaryFailurePolicy, RecoveryPolicy, RtiImageView, SecurityPolicy, TimingPolicy,
};
use tinymap::TinySecondaryMap;

/// A compiled coordination resource budget could not be allocated or was exhausted.
#[derive(Clone, Debug, thiserror::Error)]
pub enum RtiResourceError {
    /// Startup could not reserve the compiled incoming-tag storage.
    #[error("could not reserve {capacity} in-transit tags for {member:?}: {source}")]
    Allocation {
        /// Destination whose storage could not be reserved.
        member: FederateIndex,
        /// Compiled number of distinct incoming tags.
        capacity: u32,
        /// Original allocation or address-space failure.
        #[source]
        source: std::collections::TryReserveError,
    },
    /// A new destination tag would exceed its compiled accounting budget.
    #[error("in-transit tag capacity {capacity} exhausted for {member:?}")]
    InTransitCapacity {
        /// Destination whose accounting budget was exhausted.
        member: FederateIndex,
        /// Compiled number of distinct incoming tags.
        capacity: u32,
    },
}

/// Mutable state for one canonical compiled member, allocated only at RTI startup.
#[derive(Default)]
struct MemberState {
    /// Whether this member passed admission.
    joined: bool,
    /// Latest reversible local publication; absence means no information yet.
    publication: Option<(u64, Option<WireTag>)>,
    /// Revision of the last issued grant.
    granted_revision: Option<u64>,
    /// Last issued logical horizon.
    granted: Option<WireTag>,
    /// Last downstream bound sent to this member.
    dnet: Option<WireTag>,
    /// Greatest completed tag.
    completed: Option<WireTag>,
    /// Tags delivered but not yet covered by destination completion.
    in_transit: Vec<WireTag>,
    /// Current revision explicitly participating in terminal quiescence.
    idle_request: Option<u64>,
    /// Revision authorized to commit terminal idle stop.
    idle: Option<u64>,
    /// Whether the member committed terminal stop.
    stopped: bool,
}
impl MemberState {
    /// Earliest possible work, including payloads not yet reflected in local publications.
    fn earliest(&self) -> WireTag {
        let published = match self.publication {
            None => WireTag::NEVER,
            Some((_, next)) => next.unwrap_or(WireTag::FOREVER),
        };
        self.in_transit
            .first()
            .copied()
            .map_or(published, |tag| tag.min(published))
    }
}

/// Central coordination state owning a validated immutable RTI projection descriptor.
///
/// Construction accepts a validated RTI-only projection and its canonical member names.
/// Only stable member identities and the RTI projection are retained; no Enclave scheduler
/// image is stored or analyzed. The I/O owner binds each stable member identity once,
/// dispatches ordered requests, and sends returned deliveries in order. Transport failure
/// must call `abort`.
/// The projection's backing tables and member names remain borrowed.
pub struct CompiledRti<'a> {
    /// Checked projection and stable member names in the original typed key domain.
    view: RtiImageView<'a>,
    /// Mutable data for every existing member key; the image owns the key domain.
    states: TinySecondaryMap<FederateIndex, MemberState>,
    /// Shared immutable coordination identity.
    identity: CoordinationIdentity,
    /// First terminal session failure.
    failure: Option<String>,
}
impl<'a> CompiledRti<'a> {
    /// Consumes a validated RTI-only projection to create coordination state without Enclave images.
    pub fn from_image(
        view: RtiImageView<'a>,
        identity: CoordinationIdentity,
    ) -> Result<Self, CentralRtiError> {
        let image = view.image();
        let mut states = TinySecondaryMap::with_capacity(image.members().len());
        for (key, member) in image.members().iter() {
            if image.member_recovery_policy(key) != RecoveryPolicy::FailStop {
                return Err(CentralRtiError::new("unsupported member recovery policy"));
            }
            let mut state = MemberState::default();
            state
                .in_transit
                .try_reserve_exact(member.in_transit_capacity() as usize)
                .map_err(|source| RtiResourceError::Allocation {
                    member: key,
                    capacity: member.in_transit_capacity(),
                    source,
                })?;
            states.insert(key, state);
        }
        for (key, _) in image.routes().iter() {
            if image.route_failure_policy(key) != BoundaryFailurePolicy::PropagateStop
                || image.route_timing_policy(key) != TimingPolicy::BestEffort
                || image.route_security_policy(key) != SecurityPolicy::None
            {
                return Err(CentralRtiError::new("unsupported compiled boundary policy"));
            }
        }
        Ok(Self {
            view,
            states,
            identity,
            failure: None,
        })
    }

    /// Resolves a stable connection identity once, before dispatching runtime requests.
    pub fn resolve_member(&self, identity: &str) -> Result<FederateIndex, CentralRtiError> {
        self.states
            .keys()
            .find(|key| self.view.members()[*key] == identity)
            .ok_or_else(|| CentralRtiError::new("unknown compiled Federate identity"))
    }

    /// Returns the exact compiled membership count for bounded transport admission.
    pub fn member_count(&self) -> usize {
        self.states.len()
    }

    /// Reports terminal global stop or failure to the owning I/O loop.
    pub fn is_finished(&self) -> bool {
        self.failure.is_some() || self.states.values().all(|state| state.stopped)
    }
    /// Fails the session and releases every peer, preserving the first cause.
    pub fn abort(&mut self, message: impl Into<String>) -> Vec<RtiDelivery> {
        if self.failure.is_none() {
            tracing::error!(target: "boomerang::coordination",
                event = "coordination.rti.failure.first", coordination = ?self.identity);
        }
        let message = self.failure.get_or_insert_with(|| message.into()).clone();
        self.states
            .keys()
            .map(|member| RtiDelivery {
                member,
                reply: RtiReply::Failed {
                    message: message.clone(),
                },
            })
            .collect()
    }
    /// Handles one ordered request; any protocol violation terminates the entire session.
    pub fn handle(&mut self, member: FederateIndex, request: RtiRequest) -> Vec<RtiDelivery> {
        if let Some(message) = &self.failure {
            return vec![RtiDelivery {
                member,
                reply: RtiReply::Failed {
                    message: message.clone(),
                },
            }];
        }
        let request_kind = match &request {
            RtiRequest::Hello { .. } => "hello",
            RtiRequest::Publish { .. } => "publication",
            RtiRequest::Complete { .. } => "completion",
            RtiRequest::Payload { .. } => "payload",
            RtiRequest::ConfirmIdle { .. } => "confirm-idle",
            RtiRequest::Stop => "stop",
            RtiRequest::Abort { .. } => "abort",
        };
        match self.apply(member, request) {
            Ok(deliveries) => {
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.rti.decision",
                    coordination = ?self.identity,
                    federate = ?member,
                    request = request_kind,
                    deliveries = deliveries.len(),
                    "RTI coordination decision"
                );
                deliveries
            }
            Err(CentralRtiError::Coordination(message)) => {
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::ERROR,
                    event = "coordination.rti.request.rejected",
                    coordination = ?self.identity,
                    federate = ?member,
                    request = request_kind,
                    "RTI rejected a coordination request"
                );
                self.abort(message)
            }
            Err(error) => {
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::ERROR,
                    event = "coordination.rti.request.rejected",
                    coordination = ?self.identity,
                    federate = ?member,
                    request = request_kind,
                    "RTI rejected a coordination request"
                );
                self.abort(error.to_string())
            }
        }
    }
    /// Validates transitions and orders payload delivery before consequent control replies.
    fn apply(
        &mut self,
        member: FederateIndex,
        request: RtiRequest,
    ) -> Result<Vec<RtiDelivery>, CentralRtiError> {
        if let RtiRequest::Abort { message } = request {
            return Err(CentralRtiError::new(message));
        }
        let state = self
            .states
            .get(member)
            .ok_or_else(|| CentralRtiError::new("unknown member binding"))?;
        if let RtiRequest::Hello { identity } = request {
            if state.joined || identity != self.identity {
                return Err(CentralRtiError::new(
                    "coordination identity mismatch or duplicate admission",
                ));
            }
            self.states[member].joined = true;
            return Ok(if self.states.values().all(|state| state.joined) {
                self.states
                    .keys()
                    .map(|member| RtiDelivery {
                        member,
                        reply: RtiReply::Started,
                    })
                    .collect()
            } else {
                Vec::new()
            });
        }
        if !self.states.values().all(|state| state.joined) || state.stopped {
            return Err(CentralRtiError::new(
                "request outside admitted member lifecycle",
            ));
        }
        let mut deliveries = Vec::new();
        match request {
            RtiRequest::Publish {
                revision,
                next_event,
            } => {
                if next_event.is_some_and(|tag| !finite(tag))
                    || state.publication.is_some_and(|(old, next)| {
                        revision < old || (revision == old && next != next_event)
                    })
                    || next_event.is_some_and(|tag| {
                        state.completed.is_some_and(|completed| tag <= completed)
                    })
                    || (state.idle.is_some() && next_event.is_some())
                {
                    return Err(CentralRtiError::new("invalid compiled publication"));
                }
                let changed = state.publication != Some((revision, next_event));
                self.states[member].publication = Some((revision, next_event));
                if changed {
                    self.states[member].idle_request = None;
                }
            }
            RtiRequest::Complete { tag } => {
                if !finite(tag)
                    || state.granted.is_none_or(|granted| tag > granted)
                    || state.completed.is_some_and(|completed| tag < completed)
                {
                    return Err(CentralRtiError::new("invalid compiled completion"));
                }
                self.states[member].completed = Some(tag);
                self.states[member]
                    .in_transit
                    .retain(|pending| *pending > tag);
                tracing::debug!(target: "boomerang::coordination",
                    event = "coordination.rti.accounting.completed",
                    coordination = ?self.identity, federate = ?member, ?tag,
                    pending_tags = self.states[member].in_transit.len());
            }
            RtiRequest::Payload {
                route: route_key,
                tag,
                payload,
            } => {
                let routes = self.view.image().routes();
                let route = routes
                    .get(route_key)
                    .ok_or_else(|| CentralRtiError::new("unknown RTI route key"))?;
                if route.source() != member {
                    return Err(CentralRtiError::new(
                        "payload route belongs to another source",
                    ));
                }
                let lower = delay(
                    state.completed.unwrap_or(WireTag::ZERO),
                    route.delay_nanos(),
                )?;
                if !finite(tag)
                    || state.idle.is_some()
                    || tag < lower
                    || (state.completed.is_some() && route.delay_nanos() == 0 && tag == lower)
                    || (route.delay_nanos() != 0
                        && !matches!(tag, WireTag::Finite { microstep: 0, .. }))
                    || state.completed.is_some_and(|completed| {
                        state.granted.is_some_and(|granted| completed >= granted)
                    })
                    || state.granted.is_none_or(|granted| {
                        delay(granted, route.delay_nanos()).map_or(true, |upper| tag > upper)
                    })
                {
                    return Err(CentralRtiError::new(
                        "payload violates compiled source route or grant",
                    ));
                }
                let target = route.target();
                if self.states[target].stopped
                    || self.states[target].idle.is_some()
                    || self.states[target]
                        .completed
                        .is_some_and(|completed| tag <= completed)
                {
                    return Err(CentralRtiError::new(
                        "payload reached a completed or stopped destination",
                    ));
                }
                if let Err(position) = self.states[target].in_transit.binary_search(&tag) {
                    let capacity = self.view.image().members()[target].in_transit_capacity();
                    if self.states[target].in_transit.len() >= capacity as usize {
                        return Err(RtiResourceError::InTransitCapacity {
                            member: target,
                            capacity,
                        }
                        .into());
                    }
                    self.states[target].in_transit.insert(position, tag);
                }
                deliveries.push(RtiDelivery {
                    member: target,
                    reply: RtiReply::Payload {
                        route: route_key,
                        tag,
                        payload,
                    },
                });
                tracing::debug!(target: "boomerang::coordination",
                    event = "coordination.rti.payload.forwarded",
                    coordination = ?self.identity, federate = ?member, destination = ?target,
                    route = ?route_key, ?tag, pending_tags = self.states[target].in_transit.len());
            }
            RtiRequest::ConfirmIdle { revision } => {
                if state.publication != Some((revision, None)) {
                    return Err(CentralRtiError::new(
                        "idle confirmation does not match publication",
                    ));
                }
                self.states[member].idle_request = Some(revision);
            }
            RtiRequest::Stop => {
                if state.idle.is_none() {
                    return Err(CentralRtiError::new("stop without global idle authority"));
                }
                self.states[member].stopped = true;
                deliveries.push(RtiDelivery {
                    member,
                    reply: RtiReply::Stopped,
                });
            }
            RtiRequest::Hello { .. } | RtiRequest::Abort { .. } => unreachable!(),
        }
        for candidate in std::iter::once(member).chain(
            self.view
                .image()
                .affected_downstream(member)
                .iter()
                .copied()
                .filter(|candidate| *candidate != member),
        ) {
            if let Some((revision, tag)) = self.grant(candidate)? {
                self.states[candidate].granted_revision = Some(revision);
                self.states[candidate].granted = Some(
                    self.states[candidate]
                        .granted
                        .map_or(tag, |old| old.max(tag)),
                );
                deliveries.push(RtiDelivery {
                    member: candidate,
                    reply: RtiReply::Grant { revision, tag },
                });
                tracing::debug!(target: "boomerang::coordination",
                    event = "coordination.rti.grant.issued",
                    coordination = ?self.identity, federate = ?candidate, revision, ?tag);
            }
        }
        if self.states.values().all(|state| {
            state.publication.is_some_and(|(revision, next)| {
                next.is_none() && state.idle_request == Some(revision)
            }) && state.in_transit.is_empty()
        }) {
            for member in self.view.image().members().keys() {
                let state = &mut self.states[member];
                let revision = state.publication.expect("all members published").0;
                if !state.stopped && state.idle != Some(revision) {
                    state.idle = Some(revision);
                    tracing::debug!(target: "boomerang::coordination",
                        event = "coordination.rti.idle.issued", coordination = ?self.identity,
                        federate = ?member, revision);
                    deliveries.push(RtiDelivery {
                        member,
                        reply: RtiReply::Idle { revision },
                    });
                }
            }
        }
        for member in self.view.image().members().keys() {
            let state = &self.states[member];
            let Some((_, next)) = state.publication else {
                continue;
            };
            if state.stopped || state.idle.is_some() {
                continue;
            }
            let mut tag = WireTag::FOREVER;
            for downstream in self.view.image().affected_downstream(member) {
                if *downstream == member {
                    continue;
                }
                for dependency in self.view.image().transitive_incoming(*downstream) {
                    if dependency.source() == member {
                        tag = tag.min(subtract_delay(
                            self.states[*downstream].earliest(),
                            dependency.delay_nanos(),
                        )?);
                    }
                }
            }
            let old = state.dnet.unwrap_or(WireTag::NEVER);
            // Idle members can wake using old advice, including advice still in flight.
            // Always send tightenings; only useful increases need transmission.
            if tag != old && (tag < old || next.is_some_and(|next| next <= tag)) {
                self.states[member].dnet = Some(tag);
                tracing::debug!(target: "boomerang::coordination",
                    event = "coordination.rti.dnet.issued", coordination = ?self.identity,
                    federate = ?member, ?tag);
                deliveries.push(RtiDelivery {
                    member,
                    reply: RtiReply::SuppressPublication { tag },
                });
            }
        }
        Ok(deliveries)
    }
    /// Extends authority using independent completion and earliest-input proofs.
    ///
    /// Completion can tighten a conservative incoming bound whose NET is stale. The
    /// retained horizon is irrevocable; a new revision is answered even if it is covered,
    /// since the client may have discarded an earlier reply for an obsolete revision.
    fn grant(&self, member: FederateIndex) -> Result<Option<(u64, WireTag)>, CentralRtiError> {
        let state = &self.states[member];
        let Some((revision, Some(requested))) = state.publication else {
            return Ok(None);
        };
        if state.stopped || state.idle.is_some() {
            return Ok(None);
        }
        let mut completed_horizon = WireTag::FOREVER;
        for dependency in self.view.image().direct_incoming(member) {
            let bound = delay(
                self.states[dependency.source()]
                    .completed
                    .unwrap_or(WireTag::NEVER),
                dependency.delay_nanos(),
            )?;
            // Positive delay collapses later source microsteps onto the same destination tag.
            completed_horizon = completed_horizon.min(if dependency.delay_nanos() == 0 {
                bound
            } else {
                predecessor(bound)?
            });
        }
        let mut incoming = WireTag::FOREVER;
        for dependency in self.view.image().transitive_incoming(member) {
            incoming = incoming.min(delay(
                self.states[dependency.source()].earliest(),
                dependency.delay_nanos(),
            )?);
        }
        let horizon = predecessor(incoming)?
            .max(completed_horizon)
            .max(state.granted.unwrap_or(WireTag::NEVER));
        if horizon < requested
            || (state.granted_revision == Some(revision)
                && state.granted.is_some_and(|old| horizon <= old))
        {
            return Ok(None);
        }
        Ok(Some((revision, horizon)))
    }
}
/// Checks the finite nonnegative event-tag domain at the wire boundary.
fn finite(tag: WireTag) -> bool {
    tag.is_finite() && tag >= WireTag::ZERO
}
/// Applies a compiled dependency delay without narrowing or unchecked arithmetic.
fn delay(tag: WireTag, nanos: u64) -> Result<WireTag, CentralRtiError> {
    tag.checked_delay(crate::WireDelay::from_nanos(nanos))
        .ok_or_else(|| CentralRtiError::new("compiled dependency tag overflow"))
}

/// Computes a strict safe horizon without wrapping finite tag arithmetic.
fn predecessor(tag: WireTag) -> Result<WireTag, CentralRtiError> {
    tag.checked_predecessor()
        .ok_or_else(|| CentralRtiError::new("compiled predecessor tag overflow"))
}

/// Latest source tag whose delayed value is at or before the downstream bound.
fn subtract_delay(tag: WireTag, nanos: u64) -> Result<WireTag, CentralRtiError> {
    if nanos == 0 {
        return Ok(tag);
    }
    match tag {
        WireTag::Finite { offset_ns, .. } => {
            let offset = offset_ns
                .checked_sub(i128::from(nanos))
                .ok_or_else(|| CentralRtiError::new("downstream delay subtraction overflowed"))?;
            Ok(if offset < 0 {
                WireTag::NEVER
            } else {
                WireTag::finite(offset, u64::MAX)
            })
        }
        _ => Ok(tag),
    }
}

#[cfg(test)]
mod tests;
