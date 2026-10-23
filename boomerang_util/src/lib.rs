#![doc=include_str!( "../README.md")]
//! Applications select tracing and telemetry through explicit Cargo features.
//! `hosted-tracing` installs a streaming formatter. `bounded-tracing` captures native
//! events for shutdown export. Both expose the `tracing_setup` module.
//! `hosted-telemetry` provides scheduler observation sampling and bounded UDP publication.
//! Applications supply source identities, a positive sampling period, and a shutdown future.
//! Keep telemetry active until scheduler finalization. UDP delivery remains best effort.
//! `runner` provides local hosted reactor construction and execution.
//!
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

#[cfg(feature = "runner")]
pub mod runner;
/// Optional hosted scheduler telemetry sampling and publication.
#[cfg(feature = "hosted-telemetry")]
pub mod telemetry;
#[cfg(feature = "test-tracing")]
pub mod test_tracing;
/// Bounded output retention for standard tracing subscribers.
#[cfg(feature = "hosted-tracing")]
pub mod trace;
/// Optional tracing initialization and shutdown export for application processes.
#[cfg(any(feature = "hosted-tracing", feature = "bounded-tracing"))]
pub mod tracing_setup;
