//! Build-time analysis and lowering for static Federations.

mod bindings;
mod codec;
mod lowering;

pub(crate) use bindings::FederatedInboundEndpointFactory;
pub(crate) use codec::FederatedCodecRegistry;
pub(crate) use lowering::{lower_federation, FederatedBoundaryIndex, FederationLoweringArtifacts};
