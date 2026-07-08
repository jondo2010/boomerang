#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod codec;
pub mod protocol;
pub mod rti;
pub mod transport;

#[cfg(feature = "serde-json-codec")]
pub use codec::SerdeJsonCodec;
pub use codec::{CodecError, PayloadCodec, PayloadDecoder, PayloadEncoder};
pub use protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure, ProtocolFrame,
    RtiToFederate, TopologyEdge, WireDelay, WireTag,
};
pub use rti::{FederateState, GrantDecision, RtiDelivery, RtiError, RtiState};
pub use transport::{
    in_memory_transport_pair, FrameSink, FrameStream, InMemoryTransport, TransportError,
    TransportFuture,
};
