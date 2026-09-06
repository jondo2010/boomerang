//! Immutable tables consumed by a generated central RTI binary.
//!
//! Host compilation first performs the authoritative, backend-neutral federation
//! analysis. A central-backend adapter then maps its stable identities exactly once
//! into the dense keys stored here. [`RtiImage`] therefore contains dependency and
//! reachability results, but no graph representation or graph algorithms.
//!
//! A generated RTI launcher embeds these tables alongside the selected protocol and
//! transport implementation. Mutable session state remains separate: grants,
//! membership epochs, in-transit tags, queues, sockets, and secrets are never part of
//! this image.

use super::{FederateIndex, IdentityRange};
use tinymap::{TableRange, TinyMapView};

tinymap::key_type!(pub FlowIndex);
tinymap::key_type!(pub PhysicalBoundaryIndex);
tinymap::key_type!(pub RecoveryPolicyIndex);
tinymap::key_type!(pub BoundaryFailurePolicyIndex);
tinymap::key_type!(pub TransportPolicyIndex);
tinymap::key_type!(pub CodecPolicyIndex);
tinymap::key_type!(pub TimingPolicyIndex);
tinymap::key_type!(pub SecurityPolicyIndex);
tinymap::key_type!(pub TransportCapabilityIndex);
tinymap::key_type!(pub CodecCapabilityIndex);

/// One precomputed incoming dependency in dense Federate coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtiDependencyImage {
    /// Dense source Federate local to this compiled deployment.
    source: FederateIndex,
    /// Minimum logical delay from the source to the owning target.
    delay_nanos: u64,
}

impl RtiDependencyImage {
    /// Creates one precomputed dependency record.
    #[must_use]
    pub const fn new(source: FederateIndex, delay_nanos: u64) -> Self {
        Self {
            source,
            delay_nanos,
        }
    }

    /// Returns the dense source Federate.
    #[must_use]
    pub const fn source(self) -> FederateIndex {
        self.source
    }

    /// Returns the minimum logical delay in nanoseconds.
    #[must_use]
    pub const fn delay_nanos(self) -> u64 {
        self.delay_nanos
    }
}

/// Precomputed coordination ranges owned by one dense Federate entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtiMemberImage {
    /// Explicit recovery policy selected for this Federate.
    pub(super) recovery: RecoveryPolicyIndex,
    /// Direct incoming dependencies grouped by this target Federate.
    pub(super) direct_incoming: TableRange<RtiDependencyImage>,
    /// Transitive incoming dependencies grouped by this target Federate.
    pub(super) transitive_incoming: TableRange<RtiDependencyImage>,
    /// Reachable downstream Federates grouped by this source Federate.
    pub(super) affected_downstream: TableRange<FederateIndex>,
}

/// One preserved cross-Federate route in canonical stable-identity order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtiRouteImage {
    /// Stable boundary identity stored in the RTI identity blob.
    pub(super) boundary: IdentityRange,
    /// Dense stable flow shared by every route in the same end-to-end flow.
    pub(super) flow: FlowIndex,
    /// Optional dense physical input identity.
    pub(super) physical_input: Option<PhysicalBoundaryIndex>,
    /// Optional dense physical output identity.
    pub(super) physical_output: Option<PhysicalBoundaryIndex>,
    /// Dense boundary-failure policy reference.
    pub(super) failure_policy: BoundaryFailurePolicyIndex,
    /// Dense transport contract policy reference.
    pub(super) transport_policy: TransportPolicyIndex,
    /// Dense codec contract policy reference.
    pub(super) codec_policy: CodecPolicyIndex,
    /// Dense timing policy reference.
    pub(super) timing_policy: TimingPolicyIndex,
    /// Dense security policy reference.
    pub(super) security_policy: SecurityPolicyIndex,
    /// Dense selected transport implementation capability.
    pub(super) transport_capability: TransportCapabilityIndex,
    /// Dense selected codec implementation capability.
    pub(super) codec_capability: CodecCapabilityIndex,
    /// Dense source Federate.
    pub(super) source: FederateIndex,
    /// Dense target Federate.
    pub(super) target: FederateIndex,
    /// Direct logical or physical delay on this route.
    delay_nanos: u64,
}

impl RtiRouteImage {
    /// Creates one unchecked route record.
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "flat immutable image record")]
    #[rustfmt::skip]
    pub const fn new(
        boundary: IdentityRange, flow: FlowIndex,
        physical_input: Option<PhysicalBoundaryIndex>, physical_output: Option<PhysicalBoundaryIndex>,
        failure_policy: BoundaryFailurePolicyIndex, transport_policy: TransportPolicyIndex,
        codec_policy: CodecPolicyIndex, timing_policy: TimingPolicyIndex,
        security_policy: SecurityPolicyIndex, transport_capability: TransportCapabilityIndex,
        codec_capability: CodecCapabilityIndex, source: FederateIndex, target: FederateIndex,
        delay_nanos: u64,
    ) -> Self {
        Self {
            boundary, flow, physical_input, physical_output, failure_policy, transport_policy,
            codec_policy, timing_policy, security_policy, transport_capability, codec_capability,
            source, target, delay_nanos,
        }
    }

    /// Returns the dense source Federate.
    #[must_use]
    pub const fn source(self) -> FederateIndex {
        self.source
    }

    /// Returns the dense target Federate.
    #[must_use]
    pub const fn target(self) -> FederateIndex {
        self.target
    }

    /// Returns the route delay in nanoseconds.
    #[must_use]
    pub const fn delay_nanos(self) -> u64 {
        self.delay_nanos
    }
}

impl RtiMemberImage {
    /// Creates one unchecked member record from flattened table ranges.
    #[must_use]
    pub const fn new(
        recovery: RecoveryPolicyIndex,
        direct_incoming: TableRange<RtiDependencyImage>,
        transitive_incoming: TableRange<RtiDependencyImage>,
        affected_downstream: TableRange<FederateIndex>,
    ) -> Self {
        Self {
            recovery,
            direct_incoming,
            transitive_incoming,
            affected_downstream,
        }
    }
}

/// Dense immutable coordination image embedded by a central RTI launcher.
///
/// Member table positions are the canonical [`FederateIndex`] values shared with
/// the surrounding compiled deployment. The flattened ranges are produced directly
/// from the already analyzed federation; constructing this image must never rerun
/// reachability, SCC, shortest-path, or equivalent graph analysis.
#[derive(Clone, Copy, Debug)]
pub struct RtiImage<'a> {
    /// Concatenated stable identities referenced by RTI records.
    pub(super) identity_data: &'a str,
    /// Per-Federate ranges in canonical dense-key order.
    pub(super) members: TinyMapView<'a, FederateIndex, RtiMemberImage>,
    /// Flattened direct and transitive dependency records.
    pub(super) dependencies: &'a [RtiDependencyImage],
    /// Flattened affected-downstream Federate keys.
    pub(super) affected_downstream: &'a [FederateIndex],
    /// Canonically ordered cross-Federate routes, including parallel routes.
    pub(super) routes: &'a [RtiRouteImage],
    /// Canonically ordered stable flow identities.
    pub(super) flows: TinyMapView<'a, FlowIndex, IdentityRange>,
    /// Canonically ordered stable physical input/output identities.
    pub(super) physical_boundaries: TinyMapView<'a, PhysicalBoundaryIndex, IdentityRange>,
    /// Canonically ordered recovery policy identities.
    pub(super) recovery_policies: TinyMapView<'a, RecoveryPolicyIndex, IdentityRange>,
    /// Canonically ordered boundary-failure policy identities.
    pub(super) failure_policies: TinyMapView<'a, BoundaryFailurePolicyIndex, IdentityRange>,
    /// Canonically ordered transport policy identities.
    pub(super) transport_policies: TinyMapView<'a, TransportPolicyIndex, IdentityRange>,
    /// Canonically ordered codec policy identities.
    pub(super) codec_policies: TinyMapView<'a, CodecPolicyIndex, IdentityRange>,
    /// Canonically ordered timing policy identities.
    pub(super) timing_policies: TinyMapView<'a, TimingPolicyIndex, IdentityRange>,
    /// Canonically ordered security policy identities.
    pub(super) security_policies: TinyMapView<'a, SecurityPolicyIndex, IdentityRange>,
    /// Canonically ordered transport implementation identities.
    pub(super) transport_capabilities: TinyMapView<'a, TransportCapabilityIndex, IdentityRange>,
    /// Canonically ordered codec implementation identities.
    pub(super) codec_capabilities: TinyMapView<'a, CodecCapabilityIndex, IdentityRange>,
}

impl PartialEq for RtiImage<'_> {
    fn eq(&self, other: &Self) -> bool {
        macro_rules! views_equal {
            ($($field:ident),+ $(,)?) => {
                true $(&& self.$field.values().eq(other.$field.values()))+
            };
        }
        self.identity_data == other.identity_data
            && self.dependencies == other.dependencies
            && self.affected_downstream == other.affected_downstream
            && self.routes == other.routes
            && views_equal!(
                members,
                flows,
                physical_boundaries,
                recovery_policies,
                failure_policies,
                transport_policies,
                codec_policies,
                timing_policies,
                security_policies,
                transport_capabilities,
                codec_capabilities,
            )
    }
}

impl Eq for RtiImage<'_> {}

macro_rules! route_identity_accessor {
    ($name:ident, $doc:literal, $table:ident, $field:ident) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(self, route: RtiRouteImage) -> &'a str {
            self.resolve(self.$table[route.$field])
        }
    };
}

macro_rules! member_slice_accessor {
    ($name:ident, $doc:literal, $field:ident, $values:ident, $item:ty) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(self, member: FederateIndex) -> &'a [$item] {
            self.members[member]
                .$field
                .get(self.$values)
                .expect("validated RTI member range")
        }
    };
}

#[rustfmt::skip]
impl<'a> RtiImage<'a> {
    /// Creates an unchecked central RTI image over immutable tables.
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "flat immutable image schema")]
    #[rustfmt::skip]
    pub const fn new(
        identity_data: &'a str, members: TinyMapView<'a, FederateIndex, RtiMemberImage>,
        dependencies: &'a [RtiDependencyImage], affected_downstream: &'a [FederateIndex],
        routes: &'a [RtiRouteImage], flows: TinyMapView<'a, FlowIndex, IdentityRange>,
        physical_boundaries: TinyMapView<'a, PhysicalBoundaryIndex, IdentityRange>,
        recovery_policies: TinyMapView<'a, RecoveryPolicyIndex, IdentityRange>,
        failure_policies: TinyMapView<'a, BoundaryFailurePolicyIndex, IdentityRange>,
        transport_policies: TinyMapView<'a, TransportPolicyIndex, IdentityRange>,
        codec_policies: TinyMapView<'a, CodecPolicyIndex, IdentityRange>,
        timing_policies: TinyMapView<'a, TimingPolicyIndex, IdentityRange>,
        security_policies: TinyMapView<'a, SecurityPolicyIndex, IdentityRange>,
        transport_capabilities: TinyMapView<'a, TransportCapabilityIndex, IdentityRange>,
        codec_capabilities: TinyMapView<'a, CodecCapabilityIndex, IdentityRange>,
    ) -> Self {
        Self {
            identity_data, members, dependencies, affected_downstream, routes, flows,
            physical_boundaries, recovery_policies, failure_policies, transport_policies,
            codec_policies, timing_policies, security_policies, transport_capabilities,
            codec_capabilities,
        }
    }

    /// Returns all cross-Federate routes in canonical order.
    #[must_use]
    pub const fn routes(self) -> &'a [RtiRouteImage] {
        self.routes
    }

    /// Returns the number of distinct stable flows referenced by all routes.
    #[must_use]
    pub const fn flow_count(self) -> usize {
        self.flows.len()
    }

    /// Resolves the stable boundary identity carried by one route.
    #[must_use]
    pub fn route_boundary(self, route: RtiRouteImage) -> &'a str {
        route
            .boundary
            .get(self.identity_data)
            .expect("validated RTI route boundary range")
    }

    /// Resolves the stable end-to-end flow identity carried by one route.
    #[must_use]
    pub fn route_flow(self, route: RtiRouteImage) -> &'a str {
        self.flows[route.flow]
            .get(self.identity_data)
            .expect("validated RTI flow identity range")
    }

    /// Resolves the route's optional physical input identity.
    #[must_use]
    pub fn route_physical_input(self, route: RtiRouteImage) -> Option<&'a str> {
        route.physical_input.map(|index| {
            self.physical_boundaries[index]
                .get(self.identity_data)
                .expect("validated RTI physical-input identity range")
        })
    }

    /// Resolves the route's optional physical output identity.
    #[must_use]
    pub fn route_physical_output(self, route: RtiRouteImage) -> Option<&'a str> {
        route.physical_output.map(|index| {
            self.physical_boundaries[index]
                .get(self.identity_data)
                .expect("validated RTI physical-output identity range")
        })
    }

    /// Resolves one Federate's explicit recovery policy identity.
    #[must_use]
    pub fn member_recovery_policy(self, member: FederateIndex) -> &'a str {
        self.resolve(self.recovery_policies[self.members[member].recovery])
    }

    route_identity_accessor!(route_failure_policy, "Resolves one route's boundary-failure policy identity.", failure_policies, failure_policy);
    route_identity_accessor!(route_transport_policy, "Resolves one route's transport policy identity.", transport_policies, transport_policy);
    route_identity_accessor!(route_codec_policy, "Resolves one route's codec policy identity.", codec_policies, codec_policy);
    route_identity_accessor!(route_timing_policy, "Resolves one route's timing policy identity.", timing_policies, timing_policy);
    route_identity_accessor!(route_security_policy, "Resolves one route's security policy identity.", security_policies, security_policy);
    route_identity_accessor!(route_transport_capability, "Resolves one route's selected transport implementation capability.", transport_capabilities, transport_capability);
    route_identity_accessor!(route_codec_capability, "Resolves one route's selected codec implementation capability.", codec_capabilities, codec_capability);

    /// Resolves one checked RTI identity range.
    fn resolve(self, range: IdentityRange) -> &'a str {
        range
            .get(self.identity_data)
            .expect("validated RTI identity range")
    }

    member_slice_accessor!(direct_incoming, "Returns direct incoming dependencies for one target Federate.", direct_incoming, dependencies, RtiDependencyImage);
    member_slice_accessor!(transitive_incoming, "Returns transitive incoming dependencies for one target Federate.", transitive_incoming, dependencies, RtiDependencyImage);
    member_slice_accessor!(affected_downstream, "Returns Federates affected by loss or progress of one source Federate.", affected_downstream, affected_downstream, FederateIndex);
}
