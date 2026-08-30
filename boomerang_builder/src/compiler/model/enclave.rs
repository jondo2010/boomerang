use crate::compiler::{ReactorId, StableEnclaveId};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Scheduler and logical-time domain.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
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
