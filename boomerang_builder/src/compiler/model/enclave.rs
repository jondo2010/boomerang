use crate::compiler::{ReactorId, StableEnclaveId};

/// Scheduler and logical-time domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Enclave {
    /// Stable enclave identity.
    pub(super) id: StableEnclaveId,
    /// Stable identity of the Enclave root reactor.
    pub(super) root: ReactorId,
}

impl Enclave {
    /// Returns the stable enclave identity.
    pub fn id(&self) -> &StableEnclaveId {
        &self.id
    }

    /// Returns the root reactor identity.
    pub fn root(&self) -> &ReactorId {
        &self.root
    }
}
