use crate::protocol::{EndpointId, FederateId, WireDelay};

tinymap::key_type! {
    /// Dense identity of one Federate within an RTI graph.
    pub(crate) FederateKey
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RtiFederate {
    pub(super) id: FederateId,
    pub(super) incoming: Vec<IncomingDependency>,
    pub(super) transitive_incoming: Vec<IncomingPath>,
    pub(super) affected_downstream: Vec<FederateKey>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RtiEndpoint {
    pub(super) id: EndpointId,
    pub(super) source: FederateKey,
    pub(super) target: FederateKey,
    pub(super) delay: WireDelay,
}

tinymap::key_type! {
    /// Dense identity of one endpoint within an RTI graph.
    pub(crate) EndpointKey
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
