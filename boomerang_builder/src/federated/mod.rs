//! Build-time analysis and lowering for static Federations.

mod lowering;

pub(crate) use lowering::{FederatedBoundaryIndex, FederationLowering};
