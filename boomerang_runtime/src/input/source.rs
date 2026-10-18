//! Source handles for observations, progress, and clock readings.

use super::{InputAdmission, InputBatch, InputError, InputReceipt, SourceKey};
use crate::clock::{PhysicalInstant, SelectedClock};

/// A source handle for one execution. Generated initializers receive this handle.
#[derive(Clone, Debug)]
pub struct InputSourceHandle {
    /// The compiled source index.
    pub(super) key: SourceKey,
    /// Shared admission and time-reading capability.
    pub(super) admission: InputAdmission,
}
impl InputSourceHandle {
    /// Get a read-only handle for the selected clock and its identity.
    pub fn clock(&self) -> SelectedClock {
        self.admission.0.clock.clone()
    }
    /// Submit a caller-prepared batch and source progress.
    /// The batch can include other sources from the same admission service and execution.
    pub fn submit(
        &self,
        batch: InputBatch,
        frontiers: &[(InputSourceHandle, PhysicalInstant)],
    ) -> Result<InputReceipt, InputError> {
        self.admission.submit(batch, frontiers)
    }
    /// Advance this source's exclusive acquisition frontier without advancing the clock.
    pub fn advance(&self, frontier: PhysicalInstant) -> Result<(), InputError> {
        self.admission.advance(self, frontier)
    }
    /// Disconnect this source according to its required or optional policy.
    pub fn disconnect(&self) -> InputError {
        self.admission.disconnect(self)
    }
    /// Get the shared admission handle for batches that contain multiple sources.
    pub fn admission(&self) -> InputAdmission {
        self.admission.clone()
    }
}
