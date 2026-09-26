//! # Boomerang telemetry monitor
//!
//! This hosted crate receives Boomerang's internal telemetry records. It consumes
//! [`boomerang_telemetry::TelemetryRecord`] directly. The producer, receiver,
//! and deployment are built atomically as one closed system, so their record
//! layouts may evolve together; this is not an independently stable protocol.
//!
//! [`Receiver`] bounds its source registry, total copied identity metadata,
//! and each source's scheduler history. A source is the complete telemetry
//! identity: run and artifact IDs, process ID and incarnation, role, and optional
//! Federate and Enclave IDs. Scheduler and exporter-health record groups maintain
//! independent sequence ordering. Stale or reordered records cannot replace current
//! values or history.
//!
//! Scheduler rates derive only from accepted scheduler records and strictly
//! increasing sender observation times, never packet arrival times. A
//! [`MonitorSnapshot`] is the sole rendering input; renderers must not mutate
//! receiver state or infer rates. Presentation-only dashboard work is tracked
//! separately under #263 and must reuse these receiver semantics.

mod receiver;
mod render;
mod serve;

pub use receiver::{
    ExporterHealthSample, ExporterHealthSnapshot, IngestOutcome, MalformedCounters,
    MonitorSnapshot, Receiver, ReceiverConfig, ReceiverCounters, SchedulerRates, SchedulerSample,
    SchedulerSnapshot, SequenceSnapshot, SourceIdentitySnapshot, SourceSnapshot,
};
pub use render::{render_json, render_text};
pub use serve::{serve, MonitorError, MonitorOptions};

#[cfg(test)]
mod tests {
    use super::ReceiverConfig;

    #[test]
    fn default_receiver_configuration_is_bounded() {
        let config = ReceiverConfig::default();
        assert!(config.max_sources > 0);
        assert!(config.history_capacity > 0);
        assert!(config.max_metadata_bytes > 0);
    }
}
