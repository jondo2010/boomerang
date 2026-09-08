#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod codec;
pub mod protocol;
pub mod rti;

#[cfg(feature = "serde-json-codec")]
pub use codec::SerdeJsonCodec;
pub use codec::{CodecError, PayloadCodec, PayloadDecoder, PayloadEncoder};
pub use protocol::{
    EndpointId, FederateId, FederateToRti, FederatedTopology, NeighborStructure, ProtocolFrame,
    RtiToFederate, TopologyEdge, WireDelay, WireTag,
};
pub use rti::{CompiledTopology, RtiDelivery, RtiError, RtiState};
