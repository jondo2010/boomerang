//! Bounded, loss-aware capture for native tracing events and spans.
//!
//! Install [`BoundedSubscriber`] through native tracing APIs, then prepare each
//! emitting thread with [`CaptureHandle::prepare_current_thread`]. Events retain
//! owned scope snapshots; overflow, rejection and context loss remain observable.
//! The crate is unpublished pending application integration and target qualification.
//!
//! See [`specification`] for the intended contract, supported deployment profile,
//! explicit exclusions, and the evidence required before release.

#![cfg_attr(not(test), forbid(unsafe_code))]
#![deny(unsafe_code)]

// Only the test allocator's System delegation requires unsafe code.
#[cfg(test)]
#[allow(unsafe_code)]
mod test_allocation;

mod capture;
pub use capture::{
    BoundedSubscriber, BuildError, CaptureHandle, Config, LifecycleLoss, LossCount, LossSnapshot,
    OwnedField, PrepareError, ProducerGuard, Record, Scope, Value,
};

#[doc = include_str!("../SPEC.md")]
pub mod specification {}

#[cfg(test)]
mod subscriber_tests;
