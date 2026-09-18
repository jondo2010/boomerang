//! Bounded, loss-aware capture for native tracing events and spans.
//!
//! This package currently contains the public design specification and a tested
//! internal explicit-root event backend. It does not yet expose a subscriber or
//! implement producer/span context, and is not ready for publication or runtime use.
//!
//! See [`specification`] for the intended contract, supported deployment profile,
//! explicit exclusions, and the evidence required before release.

#![cfg_attr(not(test), forbid(unsafe_code))]
#![deny(unsafe_code)]

// Only the test allocator's System delegation requires unsafe code.
#[cfg(test)]
#[allow(unsafe_code)]
mod test_allocation;

// The internal capture backend is deliberately gated until context and span
// lifecycle support are implemented by the public subscriber.
#[allow(dead_code)]
mod capture;

#[doc = include_str!("../SPEC.md")]
pub mod specification {}
