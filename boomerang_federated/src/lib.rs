#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod protocol;

pub use protocol::{WireDelay, WireTag};

/// Portable semantic coordination oracle for conformance traces.
///
/// This test-support API is excluded from normal portable execution. Coordination projections
/// enable the `conformance` feature in their development dependencies to run shared vectors.
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;

/// Canonical bounded frames and exact closed-world peer admission.
pub mod wire;

/// Bounded FIFO admission and direction rules for reliable ordered channels.
pub mod channel;
