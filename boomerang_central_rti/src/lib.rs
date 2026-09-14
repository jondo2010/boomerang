#![doc=include_str!("../README.md")]
#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(missing_docs)]

/// Image-backed compiled coordination over ordered transport interfaces.
pub mod compiled;
/// Checked conversions between runtime and wire tags.
pub mod tag_conversion;

/// Shared pure wire tag and delay primitives.
pub use boomerang_federated::protocol;
pub use boomerang_federated::{WireDelay, WireTag};
pub use tag_conversion::{runtime_tag_from_wire, wire_tag_from_runtime, TagConversionError};
