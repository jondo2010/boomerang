#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod protocol;

pub use protocol::{WireDelay, WireTag};

/// Canonical bounded frames and exact closed-world peer admission.
pub mod wire;
