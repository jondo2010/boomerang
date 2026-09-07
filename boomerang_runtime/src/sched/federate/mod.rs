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
    FederateControlAuthorization, FederateCoordination, FederateCoordinationHandle,
    FederateCoordinationParticipant, FederateCoordinator, FederateIdleWait,
    FederateSchedulerCoordination, FederateTagAcquisition,
};
pub(crate) use quiescence::{FederateQuiescence, FederateQuiescenceHandle};
pub(crate) use state::LifecyclePolicy;
