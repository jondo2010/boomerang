//! Observation payloads, admission receipts, and scheduler batch events.

use super::InputSourceHandle;
use crate::{
    clock::{ExecutionEpoch, PhysicalClockDomainId, PhysicalInstant},
    image::{ActionIndex, EnclaveIndex},
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
/// An owned batch with delivery and receipt storage prepared by the caller.
///
/// Construction allocates buffers but does not validate or publish input.
/// Admission consumes the batch. It transfers each delivery buffer to its scheduler
/// and returns the tag buffer in [`InputReceipt`]. Payloads are not cloned.
/// This preparation does not make the complete hosted runtime allocation-free.
pub struct InputBatch {
    pub(super) observations: Vec<InputObservation>,
    pub(super) envelopes: tinymap::TinySecondaryMap<EnclaveIndex, InputBatchEvent>,
    tags: Vec<Tag>,
}

impl InputBatch {
    /// Prepare delivery buffers for the supplied observations before admission takes its locks.
    /// An empty batch needs no buffers and can be used to report progress only.
    pub fn new(observations: Vec<InputObservation>) -> Self {
        let mut counts = tinymap::TinySecondaryMap::<EnclaveIndex, usize>::new();
        let mut values = 0;
        for observation in &observations {
            for value in &observation.values {
                let enclave = value.action.reference.enclave;
                if !counts.contains_key(enclave) {
                    counts.insert(enclave, 0);
                }
                counts[enclave] += 1;
                values += 1;
            }
        }
        let envelopes = counts
            .into_iter()
            .map(|(enclave, count)| {
                (
                    enclave,
                    InputBatchEvent {
                        values: Vec::with_capacity(count),
                    },
                )
            })
            .collect();
        Self {
            observations,
            envelopes,
            tags: Vec::with_capacity(values),
        }
    }

    /// Move validated payloads into their prepared destination buffers.
    pub(super) fn route_values(&mut self, tags: impl Iterator<Item = Tag>) {
        self.tags.extend(tags);
        let mut tags = self.tags.iter();
        for observation in self.observations.drain(..) {
            for value in observation.values {
                let target = value.action.reference;
                self.envelopes[target.enclave].values.push((
                    *tags.next().expect("one mapped tag per validated value"),
                    target.action,
                    value.value,
                ));
            }
        }
    }

    /// Return the caller-prepared tag storage after publication succeeds.
    pub(super) fn receipt(self, arrival: Instant) -> InputReceipt {
        InputReceipt {
            arrival,
            tags: self.tags,
        }
    }
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
