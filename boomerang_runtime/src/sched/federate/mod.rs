//! Crate-private scheduler coordination boundaries for owned Federates.

pub(crate) mod backend;
mod dependencies;
mod quiescence;
pub(crate) mod state;

pub(crate) use backend::LocalFederateCoordinationBackend;
pub use backend::{
    CoordinationRevision, FederateAcquisition, FederateCompletion, FederateCoordinationBackend,
    FederateCoordinationError, FederatePublication,
};
pub(crate) use dependencies::EnclaveDependencies;
#[allow(unused_imports)]
pub(crate) use quiescence::{
    FederateCoordination, FederateCoordinationHandle, FederateCoordinationParticipant,
    FederateCoordinator, FederateSchedulerCoordination,
};
pub(crate) use quiescence::{
    FederateQuiescence, FederateQuiescenceHandle, QuiescenceControl, QuiescenceParticipant,
};
pub(crate) use state::LifecyclePolicy;
