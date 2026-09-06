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
use tinymap::{Key, TableRange, TinyMapView};

tinymap::key_type!(pub FlowIndex);
tinymap::key_type!(pub PhysicalBoundaryIndex);
tinymap::key_type!(pub TransportCapabilityIndex);
tinymap::key_type!(pub CodecCapabilityIndex);

/// A densely keyed table of UTF-8 identities stored as ranges into one backing string.
///
/// Unlike `TinyMapView<K, IdentityRange>`, this type makes the range payload and its
/// backing storage one inseparable abstraction. It is used only for open stable
/// identities; closed operational vocabularies are represented by enums.
#[derive(Clone, Copy, Debug)]
pub struct IdentityTable<'a, K: Key> {
    pub(super) identity_data: &'a str,
    ranges: TinyMapView<'a, K, IdentityRange>,
}

impl<'a, K: Key> IdentityTable<'a, K> {
    /// Creates an unchecked identity table over `identity_data` and its byte ranges.
    #[must_use]
    pub const fn new(identity_data: &'a str, ranges: TinyMapView<'a, K, IdentityRange>) -> Self {
        Self {
            identity_data,
            ranges,
        }
    }

    /// Returns the number of densely keyed identities.
    #[must_use]
    pub const fn len(self) -> usize {
        self.ranges.len()
    }

    /// Returns whether the table contains no identities.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.ranges.is_empty()
    }

    /// Resolves one key to its UTF-8 identity, or returns `None` for an invalid key or range.
    #[must_use]
    pub fn get(self, key: K) -> Option<&'a str> {
        self.ranges.get(key)?.get(self.identity_data)
    }

    pub(super) const fn ranges(self) -> TinyMapView<'a, K, IdentityRange> {
        self.ranges
    }
}

impl<K: Key> PartialEq for IdentityTable<'_, K> {
    fn eq(&self, other: &Self) -> bool {
        self.identity_data == other.identity_data && self.ranges.values().eq(other.ranges.values())
    }
}

impl<K: Key> Eq for IdentityTable<'_, K> {}

macro_rules! policy_enum {
    ($name:ident, $doc:literal, {$($(#[$meta:meta])* $variant:ident = $value:literal => $text:literal),+ $(,)?}) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
        #[repr(u8)]
        pub enum $name {
            $($(#[$meta])* $variant = $value),+
        }

        impl $name {
            /// Returns the canonical manifest spelling of this policy.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

policy_enum!(RecoveryPolicy, "Closed Federate recovery behavior compiled into an RTI image.", {
    /// Isolate the member and apply downstream failure policies.
    FailStop = 0 => "fail-stop",
    /// Restart the selected artifact from its compiled initial image.
    RestartReset = 1 => "restart-reset",
    /// Retain local state across transport loss and rejoin with a new incarnation.
    TransientRejoin = 2 => "transient-rejoin",
    /// Activate a predefined hot or warm standby.
    RedundantFailover = 3 => "redundant-failover",
    /// Transfer explicitly declared bounded semantic state.
    ApplicationStateTransfer = 4 => "application-state-transfer",
    /// Restore an optional hosted process checkpoint.
    CheckpointRestore = 5 => "checkpoint-restore",
});

policy_enum!(BoundaryFailurePolicy, "Boundary behavior when its source Federate is lost.", {
    /// Stop affected downstream execution.
    PropagateStop = 0 => "propagate-stop",
    /// Produce explicit absence.
    ProduceAbsence = 1 => "produce-absence",
    /// Produce a declared bounded safe value.
    BoundedSafeValue = 2 => "bounded-safe-value",
    /// Enter a declared degraded mode.
    EnterDegradedMode = 3 => "enter-degraded-mode",
    /// Switch to a predefined standby.
    SwitchToStandby = 4 => "switch-to-standby",
});

policy_enum!(TransportPolicy, "Closed transport contract compiled into an RTI route.", {
    /// Reliable, ordered, framed delivery within one membership epoch.
    ReliableOrderedFramed = 0 => "reliable-ordered-framed",
});

policy_enum!(CodecPolicy, "Closed codec contract compiled into an RTI route.", {
    /// Canonical architecture-independent encoding into bounded storage.
    CanonicalBounded = 0 => "canonical-bounded",
});

policy_enum!(TimingPolicy, "Closed physical-time contract compiled into an RTI route.", {
    /// Worst-case contract requiring complete qualification evidence.
    HardBound = 0 => "hard-bound",
    /// Target-window objective with explicit miss behavior.
    SoftTarget = 1 => "soft-target",
    /// Bounded resource use without a response-time guarantee.
    BestEffort = 2 => "best-effort",
});

policy_enum!(SecurityPolicy, "Closed communication security profile compiled into an RTI route.", {
    /// No channel security, accepted only by the deployment threat model.
    None = 0 => "none",
    /// Integrity protection on an otherwise protected link.
    IntegrityOnly = 1 => "integrity-only",
    /// Federate authentication without payload confidentiality.
    Authenticated = 2 => "authenticated",
    /// Federate authentication, integrity, and encryption.
    AuthenticatedEncrypted = 3 => "authenticated-encrypted",
});

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
    pub(super) recovery: RecoveryPolicy,
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
    /// Boundary-failure behavior selected for this route.
    pub(super) failure_policy: BoundaryFailurePolicy,
    /// Transport contract selected for this route.
    pub(super) transport_policy: TransportPolicy,
    /// Codec contract selected for this route.
    pub(super) codec_policy: CodecPolicy,
    /// Physical-time contract selected for this route.
    pub(super) timing_policy: TimingPolicy,
    /// Communication security profile selected for this route.
    pub(super) security_policy: SecurityPolicy,
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
    pub const fn new(
        boundary: IdentityRange,
        flow: FlowIndex,
        physical_input: Option<PhysicalBoundaryIndex>,
        physical_output: Option<PhysicalBoundaryIndex>,
        failure_policy: BoundaryFailurePolicy,
        transport_policy: TransportPolicy,
        codec_policy: CodecPolicy,
        timing_policy: TimingPolicy,
        security_policy: SecurityPolicy,
        transport_capability: TransportCapabilityIndex,
        codec_capability: CodecCapabilityIndex,
        source: FederateIndex,
        target: FederateIndex,
        delay_nanos: u64,
    ) -> Self {
        Self {
            boundary,
            flow,
            physical_input,
            physical_output,
            failure_policy,
            transport_policy,
            codec_policy,
            timing_policy,
            security_policy,
            transport_capability,
            codec_capability,
            source,
            target,
            delay_nanos,
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
        recovery: RecoveryPolicy,
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
    pub(super) flows: IdentityTable<'a, FlowIndex>,
    /// Canonically ordered stable physical input/output identities.
    pub(super) physical_boundaries: IdentityTable<'a, PhysicalBoundaryIndex>,
    /// Canonically ordered transport implementation identities.
    pub(super) transport_capabilities: IdentityTable<'a, TransportCapabilityIndex>,
    /// Canonically ordered codec implementation identities.
    pub(super) codec_capabilities: IdentityTable<'a, CodecCapabilityIndex>,
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
            && views_equal!(members)
            && self.flows == other.flows
            && self.physical_boundaries == other.physical_boundaries
            && self.transport_capabilities == other.transport_capabilities
            && self.codec_capabilities == other.codec_capabilities
    }
}

impl Eq for RtiImage<'_> {}

macro_rules! route_identity_accessor {
    ($name:ident, $doc:literal, $table:ident, $field:ident) => {
        #[doc = $doc]
        #[must_use]
        pub fn $name(self, route: RtiRouteImage) -> &'a str {
            self.$table
                .get(route.$field)
                .expect("validated RTI identity reference")
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

impl<'a> RtiImage<'a> {
    /// Creates an unchecked central RTI image over immutable tables.
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "flat immutable image schema")]
    pub const fn new(
        identity_data: &'a str,
        members: TinyMapView<'a, FederateIndex, RtiMemberImage>,
        dependencies: &'a [RtiDependencyImage],
        affected_downstream: &'a [FederateIndex],
        routes: &'a [RtiRouteImage],
        flows: IdentityTable<'a, FlowIndex>,
        physical_boundaries: IdentityTable<'a, PhysicalBoundaryIndex>,
        transport_capabilities: IdentityTable<'a, TransportCapabilityIndex>,
        codec_capabilities: IdentityTable<'a, CodecCapabilityIndex>,
    ) -> Self {
        Self {
            identity_data,
            members,
            dependencies,
            affected_downstream,
            routes,
            flows,
            physical_boundaries,
            transport_capabilities,
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
        self.flows
            .get(route.flow)
            .expect("validated RTI flow identity range")
    }

    /// Resolves the route's optional physical input identity.
    #[must_use]
    pub fn route_physical_input(self, route: RtiRouteImage) -> Option<&'a str> {
        route.physical_input.map(|index| {
            self.physical_boundaries
                .get(index)
                .expect("validated RTI physical-input identity range")
        })
    }

    /// Resolves the route's optional physical output identity.
    #[must_use]
    pub fn route_physical_output(self, route: RtiRouteImage) -> Option<&'a str> {
        route.physical_output.map(|index| {
            self.physical_boundaries
                .get(index)
                .expect("validated RTI physical-output identity range")
        })
    }

    /// Returns one Federate's explicit recovery behavior.
    #[must_use]
    pub fn member_recovery_policy(self, member: FederateIndex) -> RecoveryPolicy {
        self.members[member].recovery
    }

    /// Returns one route's boundary-failure behavior.
    #[must_use]
    pub const fn route_failure_policy(self, route: RtiRouteImage) -> BoundaryFailurePolicy {
        route.failure_policy
    }

    /// Returns one route's transport contract.
    #[must_use]
    pub const fn route_transport_policy(self, route: RtiRouteImage) -> TransportPolicy {
        route.transport_policy
    }

    /// Returns one route's codec contract.
    #[must_use]
    pub const fn route_codec_policy(self, route: RtiRouteImage) -> CodecPolicy {
        route.codec_policy
    }

    /// Returns one route's physical-time contract.
    #[must_use]
    pub const fn route_timing_policy(self, route: RtiRouteImage) -> TimingPolicy {
        route.timing_policy
    }

    /// Returns one route's communication security profile.
    #[must_use]
    pub const fn route_security_policy(self, route: RtiRouteImage) -> SecurityPolicy {
        route.security_policy
    }
    route_identity_accessor!(
        route_transport_capability,
        "Resolves one route's selected transport implementation capability.",
        transport_capabilities,
        transport_capability
    );
    route_identity_accessor!(
        route_codec_capability,
        "Resolves one route's selected codec implementation capability.",
        codec_capabilities,
        codec_capability
    );

    member_slice_accessor!(
        direct_incoming,
        "Returns direct incoming dependencies for one target Federate.",
        direct_incoming,
        dependencies,
        RtiDependencyImage
    );
    member_slice_accessor!(
        transitive_incoming,
        "Returns transitive incoming dependencies for one target Federate.",
        transitive_incoming,
        dependencies,
        RtiDependencyImage
    );
    member_slice_accessor!(
        affected_downstream,
        "Returns Federates affected by loss or progress of one source Federate.",
        affected_downstream,
        affected_downstream,
        FederateIndex
    );
}
