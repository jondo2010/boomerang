//! Crate-private scheduler coordination boundaries for owned Federates.

pub(crate) mod backend;
mod dependencies;
mod orchestration;
pub(crate) mod state;

pub(crate) use backend::LocalFederateCoordinationBackend;
pub use backend::{
    CoordinationRevision, FederateAcquisition, FederateCompletion, FederateCoordinationBackend,
    FederateCoordinationError, FederatePublication,
};
pub(crate) use dependencies::EnclaveDependencies;
#[allow(unused_imports)]
pub(crate) use orchestration::{
    EnclaveCoordinationPort, FederateAbortHandle, FederateControlAuthorization,
    FederateCoordinationParts, FederateCoordinator, FederateIdleWait,
    FederateSchedulerCoordination, FederateTagAcquisition, FederateTermination,
};
pub(crate) use state::LifecyclePolicy;
