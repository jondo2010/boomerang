#[cfg(test)]
mod tests {
    use super::{
        ConstructionError, Failure, Member, Outcome, ReferenceCoordinator, Route, RouteTopology,
        Topology, VectorStep,
    };
    use crate::WireTag;

    #[test]
    fn completion_releases_only_the_accounted_frontier() {
        let source = Member::new(0);
        let destination = Member::new(1);
        let route = Route::new(0);
        let earlier = WireTag::finite(0, 5);
        let later = WireTag::finite(0, 10);
        let mut oracle = ReferenceCoordinator::new(
            [source, destination],
            Topology::new([(route, RouteTopology::new(source, destination))]).unwrap(),
            1,
        )
        .unwrap();

        assert_eq!(
            oracle.apply(VectorStep::publish(source, 1, Some(earlier))),
            vec![Outcome::grant(source, 1, WireTag::FOREVER)]
        );
        assert_eq!(
            oracle.apply(VectorStep::payload(source, route, earlier)),
            vec![Outcome::payload(destination, route, earlier)]
        );
        assert!(oracle
            .apply(VectorStep::complete(source, earlier))
            .is_empty());

        // A later NET does not erase B's delivered, still-accounted earlier tag.
        assert_eq!(
            oracle.apply(VectorStep::publish(source, 2, Some(later))),
            vec![Outcome::grant(source, 2, WireTag::FOREVER)]
        );
        assert!(oracle
            .apply(VectorStep::publish(
                destination,
                1,
                Some(WireTag::finite(0, 9))
            ))
            .is_empty());
        assert!(oracle
            .apply(VectorStep::complete(destination, WireTag::finite(0, 4)))
            .is_empty());

        assert_eq!(
            oracle.apply(VectorStep::complete(destination, earlier)),
            vec![Outcome::grant(destination, 1, WireTag::finite(0, 9))]
        );
    }

    #[test]
    fn construction_rejects_route_endpoints_outside_the_member_table() {
        let member = Member::new(0);
        let missing = Member::new(1);
        let route = Route::new(0);
        let topology = Topology::new([(route, RouteTopology::new(member, missing))]).unwrap();

        assert!(matches!(
            ReferenceCoordinator::new([member], topology, 1),
            Err(ConstructionError::RouteEndpoint {
                route: actual_route,
                member: actual_member,
            })
            if actual_route == route && actual_member == missing
        ));
    }

    #[test]
    fn terminal_failure_retains_the_first_distinct_invalid_step() {
        let member = Member::new(0);
        let mut oracle = ReferenceCoordinator::new(
            [member],
            Topology::new([] as [(Route, RouteTopology); 0]).unwrap(),
            1,
        )
        .unwrap();

        assert_eq!(
            oracle.apply(VectorStep::payload(member, Route::new(0), WireTag::ZERO)),
            vec![Outcome::Failed {
                member,
                failure: Failure::UnknownRoute,
            }]
        );
        assert_eq!(
            oracle.apply(VectorStep::complete(member, WireTag::NEVER)),
            vec![Outcome::Failed {
                member,
                failure: Failure::UnknownRoute,
            }]
        );
    }
}

use crate::WireTag;
use tinymap::TinyMap;

tinymap::key_type!(
    /// Identifies one member in a dense, trace-local topology table.
    pub Member
);
tinymap::key_type!(
    /// Identifies one route in a dense, trace-local topology table.
    pub Route
);

/// One directed portable route between two members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteTopology {
    /// Member that may submit payloads on this route.
    pub source: Member,
    /// Member that receives payloads submitted on this route.
    pub target: Member,
}

impl RouteTopology {
    /// Creates a directed route without deriving any compiled routing decision.
    pub const fn new(source: Member, target: Member) -> Self {
        Self { source, target }
    }
}

/// Immutable, dense topology used by a conformance trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topology {
    routes: TinyMap<Route, RouteTopology>,
}

impl Topology {
    /// Builds a dense route table whose supplied keys must match its table positions.
    pub fn new(
        routes: impl IntoIterator<Item = (Route, RouteTopology)>,
    ) -> Result<Self, ConstructionError> {
        let mut table = TinyMap::new();
        for (route, topology) in routes {
            if table
                .try_insert(topology)
                .map_err(|_| ConstructionError::RouteCapacity)?
                != route
            {
                return Err(ConstructionError::RouteKey { route });
            }
        }
        Ok(Self { routes: table })
    }
}

/// Construction failed because a supplied topology key was not its dense table position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstructionError {
    /// A member key did not match the generated complete member-table key.
    MemberKey {
        /// Supplied key that was not the next dense member key.
        member: Member,
    },
    /// A route key did not match the generated complete route-table key.
    RouteKey {
        /// Supplied key that was not the next dense route key.
        route: Route,
    },
    /// The route key domain cannot represent another complete route-table entry.
    RouteCapacity,
    /// A route endpoint was absent from the fixed complete member table.
    RouteEndpoint {
        /// Route whose endpoint cannot participate in this coordinator.
        route: Route,
        /// Missing source or target member endpoint.
        member: Member,
    },
}

/// One input event in a portable coordination vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorStep {
    /// Publishes a reversible Next Event Tag for one member revision.
    Publish {
        /// Member publishing the candidate.
        member: Member,
        /// Member-owned revision for this publication.
        revision: u64,
        /// Earliest local tag, or `None` for local idle.
        next_event: Option<WireTag>,
    },
    /// Declares completion through the supplied local tag.
    Complete {
        /// Member advancing its completed frontier.
        member: Member,
        /// Greatest local tag completed by the member.
        tag: WireTag,
    },
    /// Submits a route payload at its already-derived destination tag.
    Payload {
        /// Member submitting the payload.
        member: Member,
        /// Portable route selected by the topology.
        route: Route,
        /// Destination tag carried by the payload.
        tag: WireTag,
    },
    /// Commits the member's terminal lifecycle transition.
    Stop {
        /// Member that is stopping.
        member: Member,
    },
}

impl VectorStep {
    /// Creates a NET publication step.
    pub const fn publish(member: Member, revision: u64, next_event: Option<WireTag>) -> Self {
        Self::Publish {
            member,
            revision,
            next_event,
        }
    }

    /// Creates an LTC completion step.
    pub const fn complete(member: Member, tag: WireTag) -> Self {
        Self::Complete { member, tag }
    }

    /// Creates a payload submission step.
    pub const fn payload(member: Member, route: Route, tag: WireTag) -> Self {
        Self::Payload { member, route, tag }
    }
}

/// Observable result emitted by the reference coordinator in causal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Delivers a payload before any consequent grant is emitted.
    Payload {
        /// Destination member receiving the payload.
        member: Member,
        /// Route that selected the delivery.
        route: Route,
        /// Delivered tag.
        tag: WireTag,
    },
    /// Conservatively authorizes one member through a tag.
    Grant {
        /// Member receiving the authorization.
        member: Member,
        /// Revision answered by this grant.
        revision: u64,
        /// Inclusive permitted frontier.
        tag: WireTag,
    },
    /// A member entered its terminal lifecycle state.
    Stopped {
        /// Member that stopped.
        member: Member,
    },
    /// Reports the first retained semantic failure.
    Failed {
        /// Member whose step observed the retained failure.
        member: Member,
        /// First failure retained by the coordinator.
        failure: Failure,
    },
}

impl Outcome {
    /// Creates a literal payload delivery expectation.
    pub const fn payload(member: Member, route: Route, tag: WireTag) -> Self {
        Self::Payload { member, route, tag }
    }

    /// Creates a literal grant expectation.
    pub const fn grant(member: Member, revision: u64, tag: WireTag) -> Self {
        Self::Grant {
            member,
            revision,
            tag,
        }
    }
}

/// A semantic violation retained by the coordinator without text conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The step addressed a member outside the fixed topology.
    UnknownMember,
    /// The step addressed a route outside the fixed topology.
    UnknownRoute,
    /// A payload was submitted by a member other than its route source.
    RouteSource,
    /// A step used a sentinel or negative tag where an event tag is required.
    InvalidTag,
    /// A publication regressed its revision or changed a known revision.
    Publication,
    /// A completion regressed the member's completed frontier.
    Completion,
    /// A payload would exceed the destination's fixed in-transit tag budget.
    InTransitCapacity,
    /// A stopped member attempted another active lifecycle step.
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MemberState {
    publication: Option<(u64, Option<WireTag>)>,
    completion: Option<WireTag>,
    granted: Option<(u64, WireTag)>,
    stopped: bool,
    in_transit: Vec<WireTag>,
}

impl MemberState {
    fn earliest(&self) -> WireTag {
        let published = self
            .publication
            .and_then(|(_, tag)| tag)
            .unwrap_or(WireTag::FOREVER);
        self.in_transit
            .first()
            .copied()
            .map_or(published, |tag| tag.min(published))
    }

    fn earliest_in_transit(&self) -> WireTag {
        self.in_transit.first().copied().unwrap_or(WireTag::FOREVER)
    }
}

/// A portable semantic coordinator that owns trace-local coordination state.
pub struct ReferenceCoordinator {
    members: TinyMap<Member, MemberState>,
    topology: Topology,
    in_transit_capacity: usize,
    failure: Option<Failure>,
}

impl ReferenceCoordinator {
    /// Creates a coordinator over a fixed dense member table and portable topology.
    pub fn new(
        members: impl IntoIterator<Item = Member>,
        topology: Topology,
        in_transit_capacity: usize,
    ) -> Result<Self, ConstructionError> {
        let mut states = TinyMap::new();
        for member in members {
            if states.try_insert(MemberState::default()).is_err()
                || states.keys().last() != Some(member)
            {
                return Err(ConstructionError::MemberKey { member });
            }
        }
        for (route, route_topology) in topology.routes.iter() {
            for member in [route_topology.source, route_topology.target] {
                if states.get(member).is_none() {
                    return Err(ConstructionError::RouteEndpoint { route, member });
                }
            }
        }
        Ok(Self {
            members: states,
            topology,
            in_transit_capacity,
            failure: None,
        })
    }

    /// Applies one vector step and emits deliveries before consequent grants.
    pub fn apply(&mut self, step: VectorStep) -> Vec<Outcome> {
        let member = match step {
            VectorStep::Publish { member, .. }
            | VectorStep::Complete { member, .. }
            | VectorStep::Payload { member, .. }
            | VectorStep::Stop { member } => member,
        };
        if let Some(failure) = self.failure {
            return vec![Outcome::Failed { member, failure }];
        }
        if self.members.get(member).is_none() {
            return self.fail(member, Failure::UnknownMember);
        }
        let mut outcomes = match step {
            VectorStep::Publish {
                member,
                revision,
                next_event,
            } => self.publish(member, revision, next_event),
            VectorStep::Complete { member, tag } => self.complete(member, tag),
            VectorStep::Payload { member, route, tag } => self.payload(member, route, tag),
            VectorStep::Stop { member } => self.stop(member),
        };
        if self.failure.is_none() {
            outcomes.extend(self.grants());
        }
        outcomes
    }

    fn publish(
        &mut self,
        member: Member,
        revision: u64,
        next_event: Option<WireTag>,
    ) -> Vec<Outcome> {
        let state = &self.members[member];
        if state.stopped {
            return self.fail(member, Failure::Stopped);
        }
        if next_event.is_some_and(|tag| !event_tag(tag))
            || state
                .publication
                .is_some_and(|(old, tag)| revision < old || (revision == old && tag != next_event))
        {
            return self.fail(
                member,
                if next_event.is_some_and(|tag| !event_tag(tag)) {
                    Failure::InvalidTag
                } else {
                    Failure::Publication
                },
            );
        }
        self.members[member].publication = Some((revision, next_event));
        Vec::new()
    }

    fn complete(&mut self, member: Member, tag: WireTag) -> Vec<Outcome> {
        let state = &self.members[member];
        if state.stopped {
            return self.fail(member, Failure::Stopped);
        }
        if !event_tag(tag) {
            return self.fail(member, Failure::InvalidTag);
        }
        if state.completion.is_some_and(|old| tag < old) {
            return self.fail(member, Failure::Completion);
        }
        let state = &mut self.members[member];
        state.completion = Some(tag);
        state.in_transit.retain(|pending| *pending > tag);
        Vec::new()
    }

    fn payload(&mut self, member: Member, route: Route, tag: WireTag) -> Vec<Outcome> {
        if self.members[member].stopped {
            return self.fail(member, Failure::Stopped);
        }
        if !event_tag(tag) {
            return self.fail(member, Failure::InvalidTag);
        }
        let Some(route_topology) = self.topology.routes.get(route).copied() else {
            return self.fail(member, Failure::UnknownRoute);
        };
        if route_topology.source != member {
            return self.fail(member, Failure::RouteSource);
        }
        let Some(target) = self.members.get(route_topology.target) else {
            return self.fail(member, Failure::UnknownMember);
        };
        if target.stopped {
            return self.fail(member, Failure::Stopped);
        }
        let pending = &mut self.members[route_topology.target].in_transit;
        if let Err(position) = pending.binary_search(&tag) {
            if pending.len() == self.in_transit_capacity {
                return self.fail(member, Failure::InTransitCapacity);
            }
            pending.insert(position, tag);
        }
        vec![Outcome::payload(route_topology.target, route, tag)]
    }

    fn stop(&mut self, member: Member) -> Vec<Outcome> {
        if self.members[member].stopped {
            return self.fail(member, Failure::Stopped);
        }
        self.members[member].stopped = true;
        vec![Outcome::Stopped { member }]
    }

    fn grants(&mut self) -> Vec<Outcome> {
        let mut grants = Vec::new();
        for member in self.members.keys().collect::<Vec<_>>() {
            let Some((revision, requested)) = self.members[member].publication else {
                continue;
            };
            let Some(requested) = requested else {
                continue;
            };
            if self.members[member].stopped {
                continue;
            }
            let horizon = self.permitted(member);
            if horizon < requested
                || self.members[member]
                    .granted
                    .is_some_and(|(old_revision, old)| old_revision == revision && old >= horizon)
            {
                continue;
            }
            self.members[member].granted = Some((revision, horizon));
            grants.push(Outcome::grant(member, revision, horizon));
        }
        grants
    }

    fn permitted(&self, member: Member) -> WireTag {
        let mut frontier = self.members[member].earliest_in_transit();
        for (_, route) in self
            .topology
            .routes
            .iter()
            .filter(|(_, route)| route.target == member)
        {
            frontier = frontier.min(self.members[route.source].earliest());
        }
        frontier.checked_predecessor().unwrap_or(WireTag::NEVER)
    }

    fn fail(&mut self, member: Member, failure: Failure) -> Vec<Outcome> {
        let failure = *self.failure.get_or_insert(failure);
        vec![Outcome::Failed { member, failure }]
    }
}

fn event_tag(tag: WireTag) -> bool {
    tag.is_finite() && tag >= WireTag::ZERO
}
