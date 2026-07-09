#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

#[cfg(feature = "runtime")]
pub mod client;
pub mod codec;
pub mod protocol;
pub mod rti;
#[cfg(feature = "runtime")]
pub mod runtime_bridge;
pub mod session;
#[cfg(feature = "runtime")]
pub mod static_runner;
pub mod transport;

#[cfg(feature = "runtime")]
pub use client::{
    FederateClientError, FederateClientRoute, FederateProtocolClient, RtiFederatedTimeBarrier,
};
#[cfg(feature = "serde-json-codec")]
pub use codec::SerdeJsonCodec;
pub use codec::{CodecError, PayloadCodec, PayloadDecoder, PayloadEncoder};
pub use protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure, ProtocolFrame,
    RtiToFederate, TopologyEdge, WireDelay, WireTag,
};
pub use rti::{FederateState, GrantDecision, RtiDelivery, RtiError, RtiState};
#[cfg(feature = "runtime")]
pub use runtime_bridge::RuntimeBridgeError;
pub use session::{RtiSessionEndpoint, SessionError, StaticRtiSession};
#[cfg(feature = "runtime")]
pub use static_runner::{StaticFederationRunnerError, StaticFederationRuntimeParts};
pub use transport::{
    in_memory_transport_pair, InMemoryFrameSink, InMemoryFrameStream, InMemoryTransport,
    TransportError,
};
#[cfg(feature = "serde-json-codec")]
pub use transport::{json_protocol_frame_transport, JsonProtocolFrameTransport};
