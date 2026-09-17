//! Deterministic coordination conformance support.
//!
//! Boomerang federations coordinate logical time through a central RTI. A Federate publishes its
//! next event tag (NET), reports completion (LTC), and submits tagged payloads on compiled routes.
//! The RTI may optimize when it sends grants or suppresses reports, but it must never permit an
//! outcome that changes the federation's logical-time semantics.
//!
//! This module provides a small, unoptimized oracle for testing that boundary.
//! [`crate::conformance::Topology`] and [`crate::conformance::VectorStep`] describe a compiled
//! coordination scenario with portable typed keys; [`crate::conformance::ReferenceCoordinator`]
//! applies the scenario and emits its permitted [`crate::conformance::Outcome`]s. It does not
//! call the production RTI or duplicate its implementation. A projection test runs the same
//! vector through its production coordinator, converts keys once at its test boundary, and checks
//! the complete observed outcome trace against this oracle.
//!
//! [`crate::conformance::OrderedFaultScheduler`] drives vector steps through virtual
//! reliable-ordered lanes. [`crate::conformance::FaultScript`] injects deterministic loss,
//! duplication, delay, corruption, and explicit
//! failure reports. A fault may be recovered before delivery or become the first terminal
//! [`crate::conformance::Failure`], but it cannot expose a later frame before an earlier frame on
//! the same lane.
//!
//! The API is test support, enabled with the `conformance` feature. It is deliberately absent
//! from normal portable execution and contains no hosted runtime, transport I/O, payload bytes,
//! or wire-format behavior. It covers the Phase 6 baseline only; PTAG, ABS, membership/rejoin,
//! and constructive zero-delay coordination are not modeled here.

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
tinymap::key_type!(
    /// Identifies one dense FIFO delivery lane in a portable fault trace.
    pub Lane
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
    /// A lane key did not match the generated complete lane-table key.
    LaneKey {
        /// Supplied key that was not the next dense lane key.
        lane: Lane,
    },
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
    /// A scripted lost lower-layer frame could not be delivered reliably.
    Lost,
    /// A scripted corrupted lower-layer frame entered a terminal failure state.
    Corrupted,
    /// A submitted frame addressed a lane outside the fixed lane table.
    UnknownLane,
    /// A submission or advance attempted to move the scheduler clock backwards.
    TickRegression,
    /// The generated ordinal could not represent another submitted frame.
    OrdinalExhausted,
    /// Delaying a frame would exceed the representable scheduler tick range.
    DelayOverflow,
}

/// A deterministic lower-layer event selected by submission ordinal and logical tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultEffect {
    /// Declares an unrecoverable loss for the selected frame.
    Drop { ordinal: u64, tick: u64 },
    /// Duplicates the selected lower-layer frame, which the ordered lane deduplicates.
    Duplicate { ordinal: u64, tick: u64 },
    /// Holds the selected lower-layer frame for a finite number of logical ticks.
    Delay { ordinal: u64, tick: u64, ticks: u64 },
    /// Declares the selected lower-layer frame corrupted.
    Corrupt { ordinal: u64, tick: u64 },
    /// Injects an explicit terminal failure report for the selected frame.
    Fail {
        ordinal: u64,
        tick: u64,
        failure: Failure,
    },
}

impl FaultEffect {
    /// Scripts loss for one submission ordinal at one logical tick.
    pub const fn drop(ordinal: u64, tick: u64) -> Self {
        Self::Drop { ordinal, tick }
    }

    /// Scripts a lower-layer duplicate that must not surface to the application lane.
    pub const fn duplicate(ordinal: u64, tick: u64) -> Self {
        Self::Duplicate { ordinal, tick }
    }

    /// Scripts a finite delay for one submission ordinal at one logical tick.
    pub const fn delay(ordinal: u64, tick: u64, ticks: u64) -> Self {
        Self::Delay {
            ordinal,
            tick,
            ticks,
        }
    }

    /// Scripts corruption for one submission ordinal at one logical tick.
    pub const fn corrupt(ordinal: u64, tick: u64) -> Self {
        Self::Corrupt { ordinal, tick }
    }

    /// Scripts an explicit retained failure report for one submission ordinal and tick.
    pub const fn fail(ordinal: u64, tick: u64, failure: Failure) -> Self {
        Self::Fail {
            ordinal,
            tick,
            failure,
        }
    }

    const fn key(self) -> (u64, u64) {
        match self {
            Self::Drop { ordinal, tick }
            | Self::Duplicate { ordinal, tick }
            | Self::Delay { ordinal, tick, .. }
            | Self::Corrupt { ordinal, tick }
            | Self::Fail { ordinal, tick, .. } => (ordinal, tick),
        }
    }
}

/// A validated deterministic sequence of lower-layer fault effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultScript {
    effects: Vec<FaultEffect>,
}

impl FaultScript {
    /// Builds a script ordered strictly by `(submission ordinal, logical tick)`.
    pub fn new(effects: impl IntoIterator<Item = FaultEffect>) -> Result<Self, FaultScriptError> {
        let effects = effects.into_iter().collect::<Vec<_>>();
        for effect in &effects {
            if let FaultEffect::Delay {
                ordinal,
                tick,
                ticks: 0,
            } = effect
            {
                return Err(FaultScriptError::ZeroDelay {
                    ordinal: *ordinal,
                    tick: *tick,
                });
            }
        }
        for pair in effects.windows(2) {
            let previous = pair[0].key();
            let current = pair[1].key();
            if current == previous {
                return Err(FaultScriptError::Duplicate {
                    ordinal: current.0,
                    tick: current.1,
                });
            }
            if current < previous {
                return Err(FaultScriptError::Order { previous, current });
            }
        }
        Ok(Self { effects })
    }

    fn effect(&self, ordinal: u64, tick: u64) -> Option<FaultEffect> {
        self.effects
            .iter()
            .copied()
            .find(|effect| effect.key() == (ordinal, tick))
    }
}

/// A fault script was ambiguous or non-deterministically ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultScriptError {
    /// A delay effect used zero ticks, which is excluded from fault injection.
    ZeroDelay { ordinal: u64, tick: u64 },
    /// Two effects selected the same submission ordinal and logical tick.
    Duplicate { ordinal: u64, tick: u64 },
    /// An effect appeared before a lower ordinal/tick script position.
    Order {
        previous: (u64, u64),
        current: (u64, u64),
    },
}

/// An observable ordered-lane delivery or retained scheduler failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduledOutcome {
    /// One vector step was delivered once to its lane's application boundary.
    Delivered { lane: Lane, step: VectorStep },
    /// The first scheduler failure was retained and reported.
    Failed { failure: Failure },
}

impl ScheduledOutcome {
    /// Creates a literal expected delivery.
    pub const fn delivered(lane: Lane, step: VectorStep) -> Self {
        Self::Delivered { lane, step }
    }

    /// Creates a literal expected retained failure.
    pub const fn failed(failure: Failure) -> Self {
        Self::Failed { failure }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingFrame {
    release_tick: u64,
    step: VectorStep,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LaneState {
    pending: Vec<PendingFrame>,
}

/// Deterministically applies a fault script while preserving each dense lane's FIFO boundary.
#[derive(Clone, Debug)]
pub struct OrderedFaultScheduler {
    lanes: TinyMap<Lane, LaneState>,
    script: FaultScript,
    tick: u64,
    next_ordinal: u64,
    failure: Option<Failure>,
}

impl OrderedFaultScheduler {
    /// Creates a scheduler over a fixed complete dense lane table.
    pub fn new(
        lanes: impl IntoIterator<Item = Lane>,
        script: FaultScript,
    ) -> Result<Self, ConstructionError> {
        let mut states = TinyMap::new();
        for lane in lanes {
            if states.try_insert(LaneState::default()).is_err()
                || states.keys().last() != Some(lane)
            {
                return Err(ConstructionError::LaneKey { lane });
            }
        }
        Ok(Self {
            lanes: states,
            script,
            tick: 0,
            next_ordinal: 0,
            failure: None,
        })
    }

    /// Submits one frame at `tick`, applying its scripted lower-layer fault if selected.
    pub fn submit(&mut self, tick: u64, lane: Lane, step: VectorStep) -> Vec<ScheduledOutcome> {
        if let Some(failure) = self.failure {
            return vec![ScheduledOutcome::failed(failure)];
        }
        if tick < self.tick {
            return self.fail(Failure::TickRegression);
        }
        self.tick = tick;
        if self.lanes.get(lane).is_none() {
            return self.fail(Failure::UnknownLane);
        }
        let ordinal = self.next_ordinal;
        let Some(next_ordinal) = ordinal.checked_add(1) else {
            return self.fail(Failure::OrdinalExhausted);
        };
        self.next_ordinal = next_ordinal;
        match self.script.effect(ordinal, tick) {
            Some(FaultEffect::Drop { .. }) => self.fail(Failure::Lost),
            Some(FaultEffect::Corrupt { .. }) => self.fail(Failure::Corrupted),
            Some(FaultEffect::Fail { failure, .. }) => self.fail(failure),
            Some(FaultEffect::Delay { ticks, .. }) => {
                let Some(release_tick) = tick.checked_add(ticks) else {
                    return self.fail(Failure::DelayOverflow);
                };
                self.lanes[lane]
                    .pending
                    .push(PendingFrame { release_tick, step });
                self.advance(tick)
            }
            Some(FaultEffect::Duplicate { .. }) | None => {
                self.lanes[lane].pending.push(PendingFrame {
                    release_tick: tick,
                    step,
                });
                self.advance(tick)
            }
        }
    }

    /// Advances logical time and releases only a FIFO prefix from each lane.
    pub fn advance(&mut self, tick: u64) -> Vec<ScheduledOutcome> {
        if let Some(failure) = self.failure {
            return vec![ScheduledOutcome::failed(failure)];
        }
        if tick < self.tick {
            return self.fail(Failure::TickRegression);
        }
        self.tick = tick;
        let mut outcomes = Vec::new();
        for lane in self.lanes.keys().collect::<Vec<_>>() {
            let pending = &mut self.lanes[lane].pending;
            while pending
                .first()
                .is_some_and(|frame| frame.release_tick <= tick)
            {
                let frame = pending.remove(0);
                outcomes.push(ScheduledOutcome::delivered(lane, frame.step));
            }
        }
        outcomes
    }

    fn fail(&mut self, failure: Failure) -> Vec<ScheduledOutcome> {
        let failure = *self.failure.get_or_insert(failure);
        vec![ScheduledOutcome::failed(failure)]
    }
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
#[cfg(test)]
mod tests;
