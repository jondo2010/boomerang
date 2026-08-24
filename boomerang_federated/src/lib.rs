#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod client;
pub mod codec;
#[cfg(feature = "runtime")]
mod federate_coordination;
#[cfg(feature = "runtime")]
mod hierarchy;
/// Stable protocol identities, frames, tags, and delays.
///
/// Declarative topology manifests are intentionally not part of the public API;
/// assembly lowering is the only supported producer of an RTI graph.
pub mod protocol;
/// Runtime RTI graph and live coordination state.
///
/// Static topology failures belong to assembly lowering rather than the live
/// RTI error boundary.
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
    EndpointId, FederateId, FederateToRti, ProtocolFrame, RtiToFederate, WireDelay, WireTag,
};
pub use rti::{RtiDelivery, RtiError, RtiGraph, RtiState};
#[cfg(feature = "runtime")]
pub use runtime::{
    FederatedEndpointError, FederatedFaultState, FederatedInboundEndpoint,
    FederatedOutboundCommand, FederatedOutboundMessage, FederatedOutboundSink,
    SerializedInterPartitionEventSink,
};
#[cfg(feature = "runtime")]
pub use runtime_bridge::{FederateRuntimeBridge, FederatedRuntimeConnections, RuntimeBridgeError};
pub use session::{RtiSessionEndpoint, SessionError, StaticRtiSession};
#[cfg(feature = "runtime")]
pub use static_runner::StaticFederationRunnerError;
#[cfg(all(feature = "runtime", feature = "serde-json-codec"))]
pub use static_runner::TcpStaticFederationConfig;
pub use transport::{
    in_memory_transport_pair, InMemoryFrameSink, InMemoryFrameStream, InMemoryTransport,
    TransportError,
};
#[cfg(feature = "serde-json-codec")]
pub use transport::{
    json_protocol_frame_transport, run_tcp_static_rti_session, JsonProtocolFrameSink,
    JsonProtocolFrameStream, JsonProtocolFrameTransport,
};
