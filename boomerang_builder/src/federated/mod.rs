//! Build-time analysis and lowering for static Federations.

mod bindings;
mod codec;
mod graph;
mod lowering;

pub(crate) use bindings::FederatedInboundEndpointFactory;
pub(crate) use codec::FederatedCodecRegistry;
pub(crate) use graph::{analyze_federation_graph, FederationEndpoint};
pub(crate) use lowering::{lower_federation, FederatedBoundaryIndex, FederationLoweringArtifacts};
