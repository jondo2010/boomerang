//! Optional tracing setup for application processes.
//!
//! Enable `hosted-tracing` for a streaming formatter filtered by `RUST_LOG`.
//! `init_tracing` leaves an existing global subscriber active.
//! Retain its guard until all application workers stop so buffered output flushes.
//!
//! Enable `bounded-tracing` for native capture and shutdown JSON export.
//! Pass capacities and exact-target filters through `BoundedTracingConfig`.
//! Initialization fails if another global subscriber exists.
//! Bounded filtering is independent of `RUST_LOG`.
//! Reference and loss-counter ceilings are fixed at 1,024.
//!
//! Bounded export emits schema-v1 record and loss lines to stderr on guard drop.
//! Records contain native fields and copied root-to-leaf scopes.
//! Byte values use JSON arrays; 128-bit integers use decimal strings.
//! Non-finite floats use an `f64_bits` object.
//! Per-process record order describes commit order, not a distributed clock.
//! Incomplete capture does not provide complete causal evidence.
//! Abrupt process termination does not flush capture.
//! Export errors print a diagnostic without replacing an application execution error.
//!
//! These Cargo features are additive. The application selects its initializer.
//! Keep the guard alive through runtime execution and worker shutdown.

#[cfg(feature = "hosted-tracing")]
use std::io::IsTerminal as _;

#[cfg(feature = "hosted-tracing")]
use tracing_subscriber::filter::EnvFilter;
#[cfg(feature = "bounded-tracing")]
mod bounded;
/// Native level filter used by application bounded tracing initializers.
#[cfg(feature = "bounded-tracing")]
pub use tracing::level_filters::LevelFilter;
/// Native setup configuration consumed by the bounded tracing initializer.
#[cfg(feature = "bounded-tracing")]
pub use tracing_bounded::Config as BoundedTracingConfig;
#[cfg(feature = "test-tracing")]
use tracing_subscriber::fmt::format::FmtSpan;

/// Keeps hosted tracing output alive and reports lines dropped by its bounded lossy queue.
#[must_use = "retain this guard until hosted execution and shutdown are complete"]
#[derive(Debug)]
#[cfg(feature = "hosted-tracing")]
pub struct TracingGuard {
    /// Counts formatted lines dropped by the bounded output queue.
    error_counter: tracing_appender::non_blocking::ErrorCounter,
    /// Flushes the output queue when application shutdown releases this guard.
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

#[cfg(feature = "hosted-tracing")]
impl TracingGuard {
    /// Returns the number of formatted lines dropped because the pending-output queue was full.
    pub fn dropped_lines(&self) -> usize {
        self.error_counter.dropped_lines()
    }
}

#[cfg(feature = "hosted-tracing")]
fn non_blocking_writer(
    writer: impl std::io::Write + Send + 'static,
) -> (tracing_appender::non_blocking::NonBlocking, TracingGuard) {
    let (writer, worker) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(true)
        .finish(writer);
    let error_counter = writer.error_counter();
    (
        writer,
        TracingGuard {
            error_counter,
            _worker: worker,
        },
    )
}

/// Installs off-by-default application tracing unless this process already has a subscriber.
///
/// Retain the returned guard until shutdown so the final buffered events are flushed. An already
/// installed subscriber remains active.
#[cfg(feature = "hosted-tracing")]
pub fn init_tracing() -> TracingGuard {
    let filter = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::OFF.into())
        .from_env_lossy();
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let (writer, guard) = non_blocking_writer(std::io::stderr());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .with_writer(writer)
        .try_init();
    guard
}

/// Owns bounded capture through worker shutdown and exports it on drop.
#[must_use = "retain until execution and all workers have stopped"]
#[cfg(feature = "bounded-tracing")]
pub struct BoundedTracingGuard {
    /// Owns capture and exports it after application worker shutdown.
    _guard: bounded::Guard,
}

/// Installs native bounded capture selected at build time.
///
/// Uses the native level and exact-target filter in `config`, independent of `RUST_LOG`.
/// It exports JSON lines to stderr when the guard drops, after workers stop.
/// Installation fails if another global subscriber is installed.
/// Storage capacities and filtering come from `config`; reference/loss ceilings
/// remain fixed at 1,024 by this adapter.
#[cfg(feature = "bounded-tracing")]
pub fn init_bounded_tracing(config: BoundedTracingConfig) -> std::io::Result<BoundedTracingGuard> {
    Ok(BoundedTracingGuard {
        _guard: bounded::init(config)?,
    })
}

/// Installs captured tracing with span lifecycle events for legacy test callers.
#[cfg(feature = "test-tracing")]
pub(crate) fn init_test_tracing_with_filter(filter: EnvFilter) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .with_test_writer()
        .try_init();
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "hosted-tracing")]
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    #[cfg(feature = "bounded-tracing")]
    #[test]
    fn bounded_shutdown_output_retains_native_scope_and_loss() {
        let (subscriber, capture) =
            tracing_bounded::BoundedSubscriber::new(tracing_bounded::Config {
                records: 1,
                ..Default::default()
            })
            .unwrap();
        let _producer = capture.prepare_current_thread().unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "federate",
                federate = 3u32,
                coordination = [7u8; 32].as_slice()
            );
            span.in_scope(|| {
                tracing::info!(event = "earlier");
                tracing::info!(
                    event = "last",
                    tag_offset_ns = i128::MAX,
                    tag_microstep = 7u64
                );
            });
        });
        let mut output = Vec::new();
        super::bounded::write_capture(&mut output, &capture).unwrap();
        let lines: Vec<serde_json::Value> = output
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["fields"]["event"], "last");
        assert_eq!(
            lines[0]["fields"]["tag_offset_ns"],
            "170141183460469231731687303715884105727"
        );
        assert_eq!(lines[0]["fields"]["tag_microstep"], 7);
        assert_eq!(lines[0]["scopes"][0]["fields"]["federate"], 3);
        assert_eq!(
            lines[0]["scopes"][0]["fields"]["coordination"],
            serde_json::json!([7u8; 32].as_slice())
        );
        assert_eq!(lines[1]["loss"]["overwritten_records"]["value"], 1);
        assert_eq!(lines[1]["loss"]["overwritten_records"]["saturated"], false);
        assert_eq!(lines[1]["lifecycle_loss"]["span_admission"]["value"], 0);
    }

    #[derive(Clone, Default)]
    #[cfg(feature = "hosted-tracing")]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    #[cfg(feature = "hosted-tracing")]
    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    #[cfg(feature = "hosted-tracing")]
    fn owned_guard_flushes_final_formatted_event() {
        let captured = Captured::default();
        let (writer, guard) = super::non_blocking_writer(captured.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_level(false)
            .with_target(false)
            .with_writer(writer)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(sequence = 7, "final event");
        });
        assert_eq!(guard.dropped_lines(), 0);
        drop(guard);

        assert_eq!(&*captured.0.lock().unwrap(), b"final event sequence=7\n");
    }
}
