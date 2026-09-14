//! Executor-independent rules for a reliable, ordered channel within one membership epoch.
//!
//! A projection delivers complete frames once, in submission order, or declares link failure.
//! It owns integrity, acknowledgement, retransmission and reassembly where the medium needs
//! them. Acceptance into a queue is not remote delivery. Deadlines and terminal drain belong
//! to the projection; no transport operation may change logical-time authority.

/// Maximum queued frames at each channel stage, excluding its one active I/O operation.
pub const QUEUE_CAPACITY: usize = 16;
/// Payload frames may not consume the four reserved coordination entries.
/// All accepted frames remain FIFO; this reservation never authorizes overtaking.
pub const PAYLOAD_CAPACITY: usize = QUEUE_CAPACITY - 4;

/// Admission class; it never authorizes reordering accepted frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Logical-time or lifecycle coordination.
    Coordination,
    /// Application data subject to the smaller admission quota.
    Payload,
}
