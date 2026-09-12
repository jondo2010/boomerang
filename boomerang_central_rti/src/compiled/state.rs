use super::*;
use boomerang_runtime::image::{
    BoundaryFailurePolicy, CompiledDeploymentView, CoordinationProjection, FederateImage,
    RecoveryPolicy, RtiImage, RtiRouteIndex, SecurityPolicy, TimingPolicy, TinyMapView,
};
use std::collections::{BTreeMap, BTreeSet};
use tinymap::TinyMap;

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
    /// Greatest completed tag.
    completed: Option<WireTag>,
    /// Tags delivered but not yet covered by destination completion.
    in_transit: BTreeSet<WireTag>,
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

/// Central coordination state borrowing the canonical immutable RTI projection.
///
/// Construction accepts a validated complete deployment for this slice. Only the member
/// descriptors and RtiImage are retained; no Enclave scheduler image is stored or analyzed.
/// An I/O owner resolves each authenticated stable member identity once, dispatches ordered
/// requests, and sends returned deliveries in order. Transport failure must call `abort`.
pub struct CompiledRti<'a> {
    /// Mechanical precomputed dependency and route projection.
    image: RtiImage<'a>,
    /// Stable member descriptors in the same typed domain as the RTI image.
    members: TinyMapView<'a, FederateIndex, FederateImage<'a>>,
    /// Startup identity lookup; never used to infer dense ordinals.
    identities: BTreeMap<&'a str, FederateIndex>,
    /// Startup route lookup into the distinct deployment-wide RTI route domain.
    routes: BTreeMap<&'a str, RtiRouteIndex>,
    /// Mutable member state materialized once at the runtime boundary.
    states: TinyMap<FederateIndex, MemberState>,
    /// Shared immutable coordination identity.
    identity: CoordinationIdentity,
    /// First terminal session failure.
    failure: Option<String>,
}
impl<'a> CompiledRti<'a> {
    /// Materializes runtime state without rebuilding reachability or delay analysis.
    pub fn new(
        view: &CompiledDeploymentView<'a>,
        identity: CoordinationIdentity,
    ) -> Result<Self, CentralRtiError> {
        let CoordinationProjection::CentralRti(image) = view.coordination() else {
            return Err(CentralRtiError::new(
                "compiled deployment does not select central-rti",
            ));
        };
        let members = view.federates();
        let mut states = TinyMap::new();
        let mut identities = BTreeMap::new();
        for (key, member) in members.iter() {
            if image.member_recovery_policy(key) != RecoveryPolicy::FailStop {
                return Err(CentralRtiError::new("unsupported member recovery policy"));
            }
            let allocated = states.insert(MemberState::default());
            assert_eq!(
                allocated, key,
                "materialization preserves the Federate key domain"
            );
            identities.insert(member.id().as_str(), key);
        }
        let mut routes = BTreeMap::new();
        for (key, _) in image.routes().iter() {
            if image.route_failure_policy(key) != BoundaryFailurePolicy::PropagateStop
                || image.route_timing_policy(key) != TimingPolicy::BestEffort
                || image.route_security_policy(key) != SecurityPolicy::None
            {
                return Err(CentralRtiError::new("unsupported compiled boundary policy"));
            }
            routes.insert(image.route_boundary(key).as_str(), key);
        }
        Ok(Self {
            image,
            members,
            identities,
            routes,
            states,
            identity,
            failure: None,
        })
    }

    /// Resolves a stable connection identity once, before dispatching runtime requests.
    pub fn resolve_member(&self, identity: &str) -> Result<FederateIndex, CentralRtiError> {
        self.identities
            .get(identity)
            .copied()
            .ok_or_else(|| CentralRtiError::new("unknown compiled Federate identity"))
    }
    /// Returns the stable identity used by transport admission and diagnostics.
    pub fn member_identity(&self, member: FederateIndex) -> &str {
        self.members[member].id().as_str()
    }
    /// Reports terminal global stop or failure to the owning I/O loop.
    pub fn is_finished(&self) -> bool {
        self.failure.is_some() || self.states.values().all(|state| state.stopped)
    }
    /// Fails the session and releases every peer, preserving the first cause.
    pub fn abort(&mut self, message: impl Into<String>) -> Vec<RtiDelivery> {
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
        match self.apply(member, request) {
            Ok(deliveries) => deliveries,
            Err(error) => self.abort(error.to_string()),
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
            }
            RtiRequest::Payload {
                boundary,
                tag,
                payload,
            } => {
                let route_key = self
                    .routes
                    .get(boundary.as_str())
                    .copied()
                    .ok_or_else(|| CentralRtiError::new("unknown boundary"))?;
                let route = self.image.routes()[route_key];
                let lower = delay(
                    state.completed.unwrap_or(WireTag::ZERO),
                    route.delay_nanos(),
                )?;
                if route.source() != member
                    || !finite(tag)
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
                self.states[target].in_transit.insert(tag);
                deliveries.push(RtiDelivery {
                    member: target,
                    reply: RtiReply::Payload {
                        boundary,
                        tag,
                        payload,
                    },
                });
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
            self.image
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
            }
        }
        if self.states.values().all(|state| {
            state.publication.is_some_and(|(revision, next)| {
                next.is_none() && state.idle_request == Some(revision)
            }) && state.in_transit.is_empty()
        }) {
            for member in self.members.keys() {
                let state = &mut self.states[member];
                let revision = state.publication.expect("all members published").0;
                if !state.stopped && state.idle != Some(revision) {
                    state.idle = Some(revision);
                    deliveries.push(RtiDelivery {
                        member,
                        reply: RtiReply::Idle { revision },
                    });
                }
            }
        }
        Ok(deliveries)
    }
    /// Applies precomputed direct completion and transitive next-event lower bounds.
    fn grant(&self, member: FederateIndex) -> Result<Option<(u64, WireTag)>, CentralRtiError> {
        let state = &self.states[member];
        let Some((revision, Some(requested))) = state.publication else {
            return Ok(None);
        };
        if state.stopped || state.granted_revision == Some(revision) {
            return Ok(None);
        }
        let mut complete = true;
        for dependency in self.image.direct_incoming(member) {
            let bound = delay(
                self.states[dependency.source()]
                    .completed
                    .unwrap_or(WireTag::NEVER),
                dependency.delay_nanos(),
            )?;
            // Positive delay collapses later source microsteps onto this same destination tag.
            if bound < requested || (dependency.delay_nanos() != 0 && bound == requested) {
                complete = false;
                break;
            }
        }
        if !complete {
            for dependency in self.image.transitive_incoming(member) {
                if delay(
                    self.states[dependency.source()].earliest(),
                    dependency.delay_nanos(),
                )? <= requested
                {
                    return Ok(None);
                }
            }
        }
        Ok(Some((revision, requested)))
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
