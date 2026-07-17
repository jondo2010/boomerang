#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod client;
pub mod codec;
#[cfg(feature = "runtime")]
mod hierarchy;
pub mod protocol;
pub mod rti;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "runtime")]
pub mod runtime_bridge;
pub mod session;
#[cfg(feature = "runtime")]
pub mod static_runner;
#[cfg(test)]
mod test_trace;
pub mod transport;

pub use client::{
    FederateClientError, FederateClientMailbox, FederateProtocolClient, FederateProtocolSender,
};
#[cfg(feature = "runtime")]
pub use client::{FederateClientRoute, RtiLogicalTimeCoordinator};
#[cfg(feature = "serde-json-codec")]
pub use codec::SerdeJsonCodec;
pub use codec::{CodecError, PayloadCodec, PayloadDecoder, PayloadEncoder};
#[cfg(feature = "runtime")]
pub use hierarchy::{RuntimeFederate, RuntimeFederation, RuntimeFederationError};
pub use protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure, ProtocolFrame,
    RtiToFederate, TopologyEdge, WireDelay, WireTag,
};
pub use rti::{CompiledTopology, RtiDelivery, RtiError, RtiState};
#[cfg(feature = "runtime")]
pub use runtime::{
    FederatedEndpointError, FederatedFaultState, FederatedInboundEndpoint,
    FederatedOutboundCommand, FederatedOutboundMessage, FederatedOutboundSink,
    SerializedInterPartitionEventSink,
};
#[cfg(feature = "runtime")]
pub use runtime_bridge::{FederateRuntimeBridge, FederatedRuntimeConnections, RuntimeBridgeError};
pub use session::{RtiSessionEndpoint, SessionError, StaticRtiSession};
#[cfg(all(feature = "runtime", feature = "serde-json-codec"))]
pub use static_runner::TcpStaticFederationConfig;
#[cfg(feature = "runtime")]
pub use static_runner::{StaticFederationRunnerError, StaticFederationRuntime};
pub use transport::{
    in_memory_transport_pair, InMemoryFrameSink, InMemoryFrameStream, InMemoryTransport,
    TransportError,
};
#[cfg(feature = "serde-json-codec")]
pub use transport::{
    json_protocol_frame_transport, run_tcp_static_rti_session, JsonProtocolFrameSink,
    JsonProtocolFrameStream, JsonProtocolFrameTransport,
};
