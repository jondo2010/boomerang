//! Crate-private scheduler coordination boundaries for owned Federates.

#[cfg(feature = "federated")]
pub(crate) mod backend;
mod dependencies;
mod quiescence;
#[cfg(feature = "federated")]
pub(crate) mod state;

#[cfg(feature = "federated")]
pub use backend::{
    CoordinationRevision, FederateAcquisition, FederateCompletion, FederateCoordinationBackend,
    FederateCoordinationError, FederatePublication,
};
pub(crate) use dependencies::EnclaveDependencies;
pub(crate) use quiescence::{
    FederateQuiescence, FederateQuiescenceCoordinator, FederateQuiescenceHandle, QuiescenceControl,
    QuiescenceParticipant,
};
