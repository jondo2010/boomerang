//! Hosted process policy shared by generated Boomerang launchers.

use std::io::{IsTerminal as _, Write as _};

use tracing_subscriber::filter::EnvFilter;
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

/// Installs launcher tracing unless this process already has a subscriber.
pub fn init_tracing() {
    let filter = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::OFF.into())
        .from_env_lossy();
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .with_writer(std::io::stderr)
        .try_init();
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
