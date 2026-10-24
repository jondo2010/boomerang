//! Compiled central-RTI execution over ordered transport interfaces.
//!
//! Dense keys are server-local bindings; transports identify members by their stable compiled
//! identity before dispatch. The in-memory adapters serve testing/reference execution.
//! Image dependencies are authoritative and are never recomputed from a topology.
//!
//! ## Efficient centralized coordination
//!
//! The central RTI follows [Improving the Efficiency of Coordinating Timed Events in
//! Distributed Systems](https://doi.org/10.1145/3726301.3728399).
//! Implementations must preserve the following ordering and progress requirements.
//!
//! ### Payload admission and grants
//!
//! Each Federate sends payloads through its reliable ordered RTI connection.
//! The RTI must record incoming tag obligations before forwarding payloads.
//! Each destination channel must order payloads before grants that depend on them.
//! The destination must admit those payloads before the grants become effective.
//!
//! The earliest incoming message tag (EIMT) includes compiled minimum-delay paths,
//! participant progress, and outstanding incoming tags. Progress publications must
//! not conceal earlier messages in transit. A safe horizon must strictly precede
//! EIMT, subject to stop and lifecycle constraints. Tag predecessor and delay
//! calculations must use checked arithmetic with explicit `Never` and `Forever`
//! semantics. A Federate can execute multiple local events within an authorized
//! horizon without a control exchange for each event.
//!
//! ### Progress publication and completion
//!
//! Downstream next-event bounds (DNET) suppress next-event reports (NET) only when
//! downstream participants do not need them and the Federate already has grant
//! authority. The Federate must retain its latest suppressed report. It must send
//! the required update when DNET tightens. Payload submission must update the
//! local bound. Suppression must preserve publication revisions and scheduler
//! progress.
//!
//! Latest Tag Complete (LTC) reports cumulative completion through a tag.
//! It retires incoming obligations through that tag. A Federate must report
//! completion after network-input reactions, their resulting outputs, and all
//! participating Enclaves finish. Queue receipt alone does not establish
//! completion. Local completion tracking must continue when no external report
//! is necessary. Lifecycle reporting remains independent. The protocol requires
//! neither per-payload application acknowledgments nor a duplicate receiver
//! queue solely for LTC decisions.
//!
//! ### Resource bounds and failures
//!
//! In-transit accounting and transport storage must have explicit resource bounds.
//! Saturation must preserve completion and control progress or produce a bounded
//! declared failure. It must never discard an outstanding obligation. It must
//! never block the sole reader while waiting for completion that releases capacity.
//! The transport remains responsible for reliable delivery.
//!
//! Transitions must remain safe under delayed LTC and NET reports and
//! cross-participant interleavings. Stale NET values must not regress grants or
//! deadlock shutdown. Unexpected link loss must not authorize a terminal horizon
//! as though the upstream Federate had normally resigned.
//!
//! These requirements apply outside zero-delay cycles, including positive-delay
//! cycles. Constructive zero-delay coordination requires separate PTAG/ABS
//! analysis and conformance evidence before these efficiency rules apply.
//!
//! Direct payload routing remains deferred. It requires separate safety evidence
//! for conservative accounting and payload admission before effective grants
//! across separate channels. The centralized baseline requires neither direct
//! routing nor an additional control exchange for each payload.
//!
//! ## Coordination tracing
//!
//! Compiled execution emits optional structured [`tracing`] events at the
//! `boomerang::coordination` target. Client/RTI events carry the compiler-issued coordination
//! fingerprint and an owning typed Federate key where applicable (a server-wide failure has
//! no single owner). Runtime and channel events inherit context from
//! [`RtiClientBindings::execution_span`](crate::compiled::RtiClientBindings::execution_span);
//! enter it before connecting and executing a Federate.
//! Application entry points enter the span and propagate the subscriber to worker threads.
//! Route, tag, and revision fields are included when that event concerns them.
//! The `event` field identifies a lifecycle action such as
//! `coordination.payload.sent` or `coordination.grant.received`.
//!
//! No event includes application payload bytes, stable display labels, or an error's diagnostic
//! text. Applications select off, bounded retention, or hosted export by configuring their
//! `tracing` subscriber; the runtime itself owns no trace buffer or exporter.
//!
//! Publications are NET, cumulative completions are LTC, and suppression advice is DNET.
//! `publication.suppressed` records a locally skipped NET; `publication.restored` records its
//! transmission after tightened DNET. `rti.grant.issued` records revision-specific TAG authority;
//! `rti.accounting.completed` records retirement, not merely receipt, of queued input.
//! Reaction completion includes outbound encoding/submission; cancellation means that execution
//! or this outbound work returned an error or unwound. Boundary rejection reasons are static
//! categories, never decoder diagnostics.
//!
//! Correlate `local_route` with the owning Enclave's `RouteIndex` table; `route` on RTI/client
//! events is the distinct deployment-wide `RtiRouteIndex`. Resolve their common boundary and
//! display labels from compiled artifacts outside execution. Events describe local transitions,
//! not a global clock: an enqueue can wake another thread before its sender logs completion.
//! Use FIFO, tags, revisions, and these typed keys to reconstruct causal order.
mod client;
pub mod hosted;
pub mod in_memory;
mod state;
#[cfg(test)]
mod tests;

use crate::WireTag;
use boomerang_runtime::image::{FederateIndex, RtiRouteIndex};
pub use client::{CentralRtiClient, RtiClientBindings};
pub use state::{CompiledRti, RtiResourceError};

/// Compiler-issued shared coordination digest, identical at the core and wire boundary.
pub use boomerang_federated::wire::CoordinationFingerprint as CoordinationIdentity;

/// Terminal compiled session or transport failure at a hosted interface boundary.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CentralRtiError {
    /// A compiled coordination storage budget could not be satisfied.
    #[error("central RTI: {0}")]
    Resource(#[from] RtiResourceError),
    /// A terminal coordination diagnostic independent of any transport implementation.
    #[error("central RTI: {0}")]
    Coordination(String),
    /// Original hosted failure shared with all observers of the terminated connection.
    #[error("central RTI: {0}")]
    Hosted(#[source] std::sync::Arc<hosted::HostedError>),
}
impl CentralRtiError {
    /// Records a coordination diagnostic at the core boundary.
    pub fn new(message: impl Into<String>) -> Self {
        Self::Coordination(message.into())
    }
}
impl From<hosted::HostedError> for CentralRtiError {
    fn from(error: hosted::HostedError) -> Self {
        Self::Hosted(std::sync::Arc::new(error))
    }
}

/// Owned upstream records in the compiler's original route-key domain.
pub type RtiRequest = boomerang_federated::wire::Request<RtiRouteIndex, Vec<u8>, String>;
/// Owned downstream records; their definition is shared with borrowed wire messages.
pub type RtiReply = boomerang_federated::wire::Reply<RtiRouteIndex, Vec<u8>, String>;

/// One server-local delivery selected by the compiled member domain.
#[derive(Clone, Debug)]
pub struct RtiDelivery {
    /// Recipient in the fingerprint-verified coordination image member domain.
    pub member: FederateIndex,
    /// Ordered reply for that recipient.
    pub reply: RtiReply,
}

/// Nonblocking reliable ordered submission shared by control and payload producers.
pub trait RtiRequestSink: Send + Sync {
    /// Accepts one request in the same ordering domain as every other request on this connection.
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError>;
}
/// Bounded receive interface implemented by the selected central-RTI transport.
pub trait RtiReplySource: Send {
    /// Receives in order, returning `None` on timeout and an error on connection loss.
    fn receive(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<RtiReply>, CentralRtiError>;
}
