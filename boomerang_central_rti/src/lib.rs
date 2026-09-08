#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

/// Federate-side protocol clients and their scheduler bridge.
pub mod client;
/// Checked conversions between runtime values and protocol values.
pub mod runtime_bridge;
/// Central RTI session coordination over connected transports.
pub mod session;
/// Static federation runners that own Tokio execution.
pub mod static_runner;
#[cfg(test)]
mod test_trace;
/// In-memory and TCP protocol transports for the central RTI backend.
pub mod transport;

/// Shared pure federation protocol used by the backend transports.
pub use boomerang_federated::protocol;
/// Shared pure RTI state used by the backend session.
pub use boomerang_federated::rti;
pub use boomerang_federated::{
    CompiledTopology, EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure,
    ProtocolFrame, RtiToFederate, TopologyEdge, WireDelay, WireTag,
};
pub use client::{
    FederateClientError, FederateClientMailbox, FederateClientRoute, FederateProtocolClient,
    FederateProtocolSender, RtiFederatedTimeBarrier,
};
pub use runtime_bridge::{
    runtime_tag_from_wire, wire_delay_from_runtime, wire_tag_from_runtime,
    FederatedRuntimeConnections, RuntimeBridgeError,
};
pub use session::{RtiSessionEndpoint, SessionError, StaticRtiSession};
pub use static_runner::{
    FederatePlacementError, StaticFederationRunnerError, StaticFederationRuntime,
    TcpStaticFederationConfig,
};
pub use transport::{
    in_memory_transport_pair, json_protocol_frame_transport, run_tcp_static_rti_session,
    InMemoryFrameSink, InMemoryFrameStream, InMemoryTransport, JsonProtocolFrameSink,
    JsonProtocolFrameStream, JsonProtocolFrameTransport, TransportError,
};
