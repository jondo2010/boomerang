//! # Boomerang telemetry monitor
//!
//! This hosted crate receives Boomerang's internal telemetry records. It consumes
//! [`boomerang_telemetry::TelemetryRecord`] directly. The producer, receiver,
//! and deployment are built atomically as one closed system, so their record
//! layouts may evolve together; this is not an independently stable protocol.
//!
//! [`Receiver`] owns bounded source registration, copied identity metadata,
//! and scheduler history. A source is the complete telemetry identity: run and
//! artifact IDs, process ID and incarnation, role, and optional Federate and
//! Enclave IDs. Scheduler and exporter-health record groups each maintain their
//! own sequence ordering. Stale or reordered records cannot replace current
//! values or history.
//!
//! Scheduler rates derive only from accepted scheduler records and strictly
//! increasing sender observation times, never packet arrival times. A
//! [`MonitorSnapshot`] is the sole rendering input; renderers must not mutate
//! receiver state or infer rates. Presentation-only dashboard work is tracked
//! separately under #263 and must reuse these receiver semantics.

mod receiver {
    /// Limits for receiver-owned state.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ReceiverConfig {
        pub max_sources: usize,
        pub history_capacity: usize,
        pub max_metadata_bytes: usize,
    }

    impl Default for ReceiverConfig {
        fn default() -> Self {
            Self {
                max_sources: 64,
                history_capacity: 120,
                max_metadata_bytes: 64 * 1024,
            }
        }
    }

    /// Receiver state; Task 2 supplies ingest and snapshot behavior.
    pub struct Receiver;

    /// Receiver observation; Task 2 supplies its data shape.
    pub struct MonitorSnapshot;
}

mod render {}

mod serve {
    use super::receiver::MonitorSnapshot;

    /// Monitor command options; Task 3 supplies its data shape.
    pub struct MonitorOptions;

    /// Monitor command error; Task 3 supplies its variants.
    #[derive(Debug, thiserror::Error)]
    #[error("monitor serving is not implemented yet")]
    pub struct MonitorError;

    /// Serve telemetry; Task 3 supplies the UDP loop.
    pub fn serve(_options: &MonitorOptions) -> Result<MonitorSnapshot, MonitorError> {
        unimplemented!("monitor serving is implemented in Task 3")
    }
}

pub use receiver::{MonitorSnapshot, Receiver, ReceiverConfig};
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
