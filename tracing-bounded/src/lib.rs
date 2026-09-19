//! Bounded, loss-aware capture for native tracing events and spans.
//!
//! This package currently contains the public design specification only. It does
//! not yet implement a subscriber and is not ready for publication or runtime use.
//!
//! See [`specification`] for the intended contract, supported deployment profile,
//! explicit exclusions, and the evidence required before release.

#![forbid(unsafe_code)]

#[doc = include_str!("../SPEC.md")]
pub mod specification {}
