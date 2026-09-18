//! Hosted process policy shared by generated Boomerang launchers.

#[cfg(feature = "hosted-tracing")]
use std::io::IsTerminal as _;
use std::io::Write as _;

#[cfg(feature = "hosted-tracing")]
use tracing_subscriber::filter::EnvFilter;
#[cfg(feature = "bounded-tracing")]
mod bounded;
/// Native setup configuration consumed by the bounded launcher initializer.
#[cfg(feature = "bounded-tracing")]
pub use tracing_bounded::Config as BoundedTracingConfig;
#[cfg(feature = "test-tracing")]
use tracing_subscriber::fmt::format::FmtSpan;

/// Private protocol variable through which a supervisor requests a summary.
pub const EXECUTION_SUMMARY_ENV: &str = "BOOMERANG_EXECUTION_SUMMARY_V1";

/// Schema-v1 summary emitted through the private supervisor protocol.
#[derive(serde::Serialize)]
struct ExecutionSummaryDocumentV1<'a> {
    /// Protocol schema version.
    schema: u32,
    /// Aggregate runtime scheduling counters.
    stats: &'a boomerang_runtime::Stats,
    /// Final logical tag reached by the Federate.
    final_tag: FinalTagDocumentV1,
}

/// Schema-v1 final logical tag.
#[derive(serde::Serialize)]
struct FinalTagDocumentV1 {
    /// Signed logical offset in nanoseconds.
    offset_nanos: String,
    /// Superdense microstep count.
    microstep: String,
}

/// Keeps hosted tracing output alive and reports lines dropped by its bounded lossy queue.
#[must_use = "retain this guard until hosted execution and shutdown are complete"]
#[derive(Debug)]
#[cfg(feature = "hosted-tracing")]
pub struct TracingGuard {
    error_counter: tracing_appender::non_blocking::ErrorCounter,
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

/// Installs off-by-default launcher tracing unless this process already has a subscriber.
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

/// Owns bounded coordination capture through worker shutdown and exports it on drop.
#[must_use = "retain until execution and all workers have stopped"]
#[cfg(feature = "bounded-tracing")]
pub struct BoundedTracingGuard {
    _guard: bounded::Guard,
}

/// Installs native bounded coordination capture selected at build time.
///
/// Captures only the native `boomerang::coordination` target at DEBUG, independent of `RUST_LOG`.
/// It exports JSON lines to stderr when the guard drops, after workers stop.
/// Installation fails if another global subscriber is installed.
/// The eight storage capacities come from `config`; filtering and reference/loss
/// ceilings are fixed by the launcher policy, regardless of their supplied values.
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

/// Writes the optional version-1 execution summary requested by a supervisor.
pub fn write_execution_summary(
    execution: &boomerang_runtime::FederateExecution,
) -> std::io::Result<()> {
    let Some(path) = std::env::var_os(EXECUTION_SUMMARY_ENV) else {
        return Ok(());
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let document = ExecutionSummaryDocumentV1 {
        schema: 1,
        stats: execution.stats(),
        final_tag: FinalTagDocumentV1 {
            offset_nanos: execution
                .final_tag()
                .offset()
                .whole_nanoseconds()
                .to_string(),
            microstep: execution.final_tag().microstep().to_string(),
        },
    };
    serde_json::to_writer(&mut file, &document).map_err(std::io::Error::other)?;
    writeln!(file)
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
