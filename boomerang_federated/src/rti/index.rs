use crate::protocol::{EndpointId, FederateId, NeighborStructure, WireDelay};

tinymap::key_type! {
    /// Dense identity of one Federate within a compiled topology.
    pub(crate) FederateKey
}

tinymap::key_type! {
    /// Dense identity of one endpoint within a compiled topology.
    pub(crate) EndpointKey
}

/// Immutable compiled state for one Federate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledFederate {
    /// Stable protocol identity of this Federate.
    pub(super) id: FederateId,
    /// Direct incoming dependencies in deterministic source and endpoint order.
    pub(super) incoming: Vec<IncomingDependency>,
    /// Direct downstream Federates in lexical stable-identity order.
    pub(super) downstream: Vec<FederateKey>,
    /// Transitive incoming paths in lexical stable-identity order.
    pub(super) transitive_incoming: Vec<IncomingPath>,
    /// Transitive downstream Federates in lexical stable-identity order.
    pub(super) transitive_downstream: Vec<FederateKey>,
    /// Cached stable protocol view used during participant admission.
    pub(super) neighbors: NeighborStructure,
}

/// Immutable compiled state for one serialized endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledEndpoint {
    /// Stable protocol identity of this endpoint.
    pub(super) id: EndpointId,
    /// Dense key of the source Federate.
    pub(super) source: FederateKey,
    /// Dense key of the target Federate.
    pub(super) target: FederateKey,
    /// Minimum logical delay on this endpoint.
    pub(super) delay: WireDelay,
}

/// One direct incoming endpoint dependency.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct IncomingDependency {
    /// Dense key of the source Federate.
    pub(super) source: FederateKey,
    /// Dense key of the serialized endpoint.
    pub(super) endpoint: EndpointKey,
    /// Minimum logical delay on the endpoint.
    pub(super) delay: WireDelay,
}

/// One transitive incoming path and its minimum accumulated delay.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct IncomingPath {
    /// Dense key of the path's source Federate.
    pub(super) source: FederateKey,
    /// Minimum accumulated logical delay from source to target.
    pub(super) delay: WireDelay,
}
