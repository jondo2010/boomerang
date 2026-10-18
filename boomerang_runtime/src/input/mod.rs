//! Timestamped input observations and source progress.
//!
//! Use [`SourceAction`] with [`InputSourceHandle`] to preserve acquisition times.
//! Admission checks timestamps and sequence numbers. It rejects late observations.
//! Each batch publishes its observations before its associated progress.
//! Required sources limit logical execution until their frontiers permit progress.
//!
//! Source admission works with the host monotonic clock or a manual clock.
//! It requires the `hosted-source-input` feature with either clock.
//! For ordinary keyboard events and callbacks, use [`crate::context::SendContext`]
//! with [`crate::AsyncActionRef`]. Reactor initialization uses [`crate::initialization`].

mod action;
mod admission;
mod batch;
mod config;
mod retention;
mod source;

pub use action::SourceAction;
pub use admission::InputAdmission;
pub use batch::{InputBatchEvent, InputObservation, InputReceipt, InputValue};
pub use config::{InputConfig, InputSource};
pub use source::InputSourceHandle;

use crate::image::{ActionIndex, EnclaveIndex};
use std::time::Instant;

tinymap::key_type! {
    /// A source index in the compiled Federate source table.
    pub SourceKey
}

/// A reference to a physical action in a compiled Enclave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompiledPhysicalAction {
    /// The Enclave index in the deployment.
    pub enclave: EnclaveIndex,
    /// The action index within the Enclave.
    pub action: ActionIndex,
}
impl CompiledPhysicalAction {
    /// Create an action reference. Runtime validation checks the referenced image row.
    pub const fn new(enclave: EnclaveIndex, action: ActionIndex) -> Self {
        Self { enclave, action }
    }
}

/// An admission or source protocol error category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputErrorKind {
    /// A declaration, identity, payload, or time mapping is invalid.
    Malformed,
    /// Acquisition time exceeds the selected clock time. A later submission can succeed.
    Future,
    /// A destination has completed or reserved the mapped tag for execution.
    Late,
    /// The sequence matches the last accepted or checked sequence for this source.
    Duplicate,
    /// The sequence or acquisition time is less than the preceding accepted or checked value.
    OutOfOrder,
    /// An observation contradicts a frontier, or a frontier decreases.
    Protocol,
    /// A staging, batch, mailbox, or action/tag capacity limit has been reached.
    Overflow,
    /// Source, destination, clock, or execution is disconnected.
    Disconnected,
}
/// Diagnostic with stable identity and a separately captured host monotonic arrival time.
#[derive(Clone, Debug, thiserror::Error)]
#[error("physical input {source_id:?}: {kind:?}")]
pub struct InputError {
    /// Stable source name, when the dense identity was valid.
    pub source_id: Option<String>,
    /// The error category.
    pub kind: InputErrorKind,
    /// The host monotonic diagnostic timestamp. It does not determine the logical tag.
    pub arrival: Instant,
}

impl InputError {
    /// Create an error without a source identity, using the current host timestamp.
    fn new(kind: InputErrorKind) -> InputError {
        InputError {
            source_id: None,
            kind,
            arrival: Instant::now(),
        }
    }

    /// Create a configuration error without a resolved source.
    pub(crate) fn configuration_error() -> Self {
        Self::new(InputErrorKind::Malformed)
    }
}
