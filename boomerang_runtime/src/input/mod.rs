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
//!
//! # Internal data flow
//!
//! One Federate coordinator owns source admission for the compiled execution.
//! Each Enclave has its own scheduler and event mailbox.
//! A mailbox is the channel through which a scheduler receives external events.
//! A batch can contain values for more than one Enclave.
//!
//! ```text
//! Producer
//!   | InputBatch through the admission request channel
//!   v
//! Federate coordinator / AdmissionProcessor
//!   | validate, map tags, then publish one event per destination
//!   +-- PhysicalBatch --> Enclave A mailbox --> action storage and event queue
//!   +-- PhysicalBatch --> Enclave B mailbox --> action storage and event queue
//!   |
//!   +-- InputReceipt --> producer through the admission reply channel
//!
//! Enclave scheduler -- BeginTag --> coordinator -- permit reply --> scheduler
//! Enclave scheduler -- tag completion --> coordinator
//! ```
//!
//! The data path transfers owned payloads. The control path carries source
//! progress, scheduler reports, and execution permits. The coordinator processes
//! admission requests and scheduler reports on the same thread.
//!
//! ## Prepare and submit a batch
//!
//! A [`SourceAction`] identifies a source and a destination action.
//! [`SourceKey`] selects the source protocol state.
//! [`CompiledPhysicalAction`] selects an Enclave and an action row within that Enclave.
//! These identities have separate purposes.
//!
//! The producer puts owned values in an [`InputObservation`]. Each observation
//! includes a sequence number, an acquisition time, a clock domain, and an epoch.
//! [`InputBatch::new`] prepares a delivery buffer for each destination Enclave.
//! It also prepares storage for the receipt tags.
//! [`InputBatch::with_progress`] attaches source frontiers to the batch.
//!
//! [`InputSourceHandle::submit`] calls [`InputAdmission::submit`]. The caller waits
//! for the shared request channel, transfers the batch, and reports `InputReady`.
//! The coordinator processes the request and sends a reply to the waiting caller.
//! Only one caller can use the request and reply channels at a time.
//! The arrival timestamp records host monotonic time before this wait.
//! It does not determine the logical tags.
//!
//! ## Validate and publish
//!
//! `AdmissionProcessor`, in `admission.rs`, owns the source protocol state.
//! It first reclaims reservations for completed action tags.
//! It then checks the whole batch and its progress updates in reusable temporary storage.
//! These checks include identities, payload types, clock metadata, source order,
//! frontiers, capacity limits, and action/tag conflicts.
//!
//! Each value receives a logical tag from its acquisition time and action minimum delay.
//! Acquisition times cannot exceed the selected clock time.
//! A value is late if its destination has a final execution permit for that tag
//! or a later tag. Completed tags also reject late values.
//! A validation error publishes no values from the batch.
//!
//! After validation, admission moves the values into the prepared destination buffers.
//! Each entry contains a tag, a compiled action row, and an owned payload.
//! Admission sends each buffer as `AsyncEvent::PhysicalBatch` through the existing
//! scheduler event mailbox. These sends do not wait for mailbox space.
//! Admission does not clone the payloads.
//!
//! The first successful send invalidates provisional grants and changes the publication generation.
//! The coordinator cannot issue a final execution permit while it processes the batch.
//! After all sends succeed, admission commits the source sequence numbers and frontiers.
//! It retains action/tag reservations and updates the input horizon.
//! The input horizon is the greatest tag that required sources currently permit.
//! Admission then requests scheduler wakes and returns an [`InputReceipt`].
//!
//! A successful receipt confirms publication. It does not confirm reaction completion.
//! Sends to different mailboxes occur in sequence. They are not simultaneous.
//! If a later send fails, admission blocks new input execution permits and requests an abort.
//! It does not remove messages already sent. Work with an existing final permit can finish.
//!
//! ## Store values and execute reactions
//!
//! `SchedulerCore::handle_async_event`, in `sched/core.rs`, receives each batch event.
//! It resolves each compiled action row through the scheduler storage.
//! It stores the payload at the supplied tag and adds the action triggers to the event queue.
//! This path preserves the admitted tag. It does not assign a new timestamp or microstep.
//!
//! A scheduler can store these values before admission finishes the complete batch.
//! Execution still depends on coordinator authority, source progress, and clock pacing.
//! The publication fence makes schedulers check their mailboxes and confirm the current
//! publication generation before the coordinator grants new execution authority.
//! This prevents execution with provisional grants from before the batch publication.
//!
//! With source input enabled for the execution, the scheduler sends `BeginTag`
//! immediately before it processes a tag. The coordinator checks the current grant
//! and input horizon, then returns a final permit through a separate reply channel.
//! An issued final permit establishes the boundary for late input.
//! The coordinated terminal shutdown tag bypasses this input permit check.
//! Each scheduler executes its own reactions. A batch does not make reactions in
//! different Enclaves execute at the same wall-clock time.
//!
//! After a logical tag completes, the scheduler reports completion to the coordinator.
//! On a later admission request, `retention.rs` releases the completed reservations.
//! A retained batch occupies a batch slot until all its destinations complete.
//! Retention stores action/tag metadata. The scheduler storage owns the published payloads.
//!
//! ## Source progress and lifecycle
//!
//! An exclusive source frontier `F` means that future acquisition times are at least `F`.
//! A required source permits tags strictly below `F + minimum target delay`.
//! The minimum target delay is the smallest action delay declared for that source.
//! The input horizon is the smallest limit from all required sources.
//! A required source with no frontier blocks finite tags.
//! Optional sources do not limit the input horizon.
//!
//! Progress attached to a batch takes effect only after all its values are published.
//! A producer can also report progress without values through its source handle.
//! Clock advancement and source progress are independent.
//! The same admission path works with the host monotonic clock and a manual clock.
//!
//! The coordinator starts before deferred reactor initialization.
//! It can accept synchronous submissions while reaction execution remains closed.
//! After initialization succeeds, execution opens and the scheduler workers start.
//! During shutdown, admission closes the request channels and releases waiting callers
//! before backend cleanup can join producer threads.

mod action;
mod admission;
mod batch;
mod config;
mod retention;
mod source;

pub use action::SourceAction;
pub(crate) use admission::AdmissionProcessor;
pub use admission::InputAdmission;
pub use batch::{InputBatch, InputBatchEvent, InputObservation, InputReceipt, InputValue};
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
