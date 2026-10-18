//! Observation payloads, admission receipts, and scheduler batch events.

use super::InputSourceHandle;
use crate::{
    clock::{ExecutionEpoch, PhysicalClockDomainId, PhysicalInstant},
    image::ActionIndex,
    ReactorData, Tag,
};
use std::time::Instant;

/// An owned action value. Admission does not clone the payload.
pub struct InputValue {
    pub(super) action: super::action::AdmittedAction,
    pub(super) value: Box<dyn ReactorData>,
}
impl InputValue {
    /// Create an owned value for a source action without cloning the payload.
    pub fn new<T: ReactorData>(action: &super::SourceAction<T>, value: T) -> Self {
        Self {
            action: action.admission(),
            value: Box::new(value),
        }
    }
}
/// An acquisition whose values are submitted together in one batch.
pub struct InputObservation {
    /// The source handle for this execution.
    pub source: InputSourceHandle,
    /// The source sequence number. Each new observation uses a greater number.
    pub sequence: u64,
    /// The clock domain in which the acquisition occurred.
    pub domain: PhysicalClockDomainId,
    /// The execution epoch supplied by the selected clock.
    pub epoch: ExecutionEpoch,
    /// The acquisition time within that epoch.
    pub acquired: PhysicalInstant,
    /// The owned action values.
    pub values: Vec<InputValue>,
}
/// The result of a successful batch submission.
#[derive(Debug)]
pub struct InputReceipt {
    /// The host monotonic arrival time for this batch.
    pub arrival: Instant,
    /// The mapped tags in observation and value order. Arrival time does not affect these tags.
    pub tags: Vec<Tag>,
}

/// A batch of values for one Enclave. Only admission can construct this event.
pub struct InputBatchEvent {
    /// Scheduled tags, compiled action rows, and owned payloads for this Enclave.
    pub(crate) values: Vec<(Tag, ActionIndex, Box<dyn ReactorData>)>,
}

impl std::fmt::Debug for InputBatchEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputBatchEvent")
            .field("values", &self.values.len())
            .finish()
    }
}
