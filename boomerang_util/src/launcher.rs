//! Hosted process policy shared by generated Boomerang launchers.

use std::io::{IsTerminal as _, Write as _};

use tracing_subscriber::filter::EnvFilter;
#[cfg(feature = "test-tracing")]
use tracing_subscriber::fmt::format::FmtSpan;

/// Private protocol variable through which a supervisor requests a summary.
pub const EXECUTION_SUMMARY_ENV: &str = "BOOMERANG_EXECUTION_SUMMARY_V1";

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
    let stats = execution.stats();
    writeln!(
        file,
        concat!(
            "{{\"schema\":1,\"stats\":{{",
            "\"processed_tags\":\"{}\",",
            "\"processed_reactions\":\"{}\",",
            "\"processed_events\":\"{}\",",
            "\"set_ports\":\"{}\",",
            "\"scheduled_actions\":\"{}\"}},",
            "\"final_tag\":{{\"offset_nanos\":\"{}\",",
            "\"microstep\":\"{}\"}}}}",
        ),
        stats.processed_tags(),
        stats.processed_reactions(),
        stats.processed_events(),
        stats.set_ports(),
        stats.scheduled_actions(),
        execution.final_tag().offset().whole_nanoseconds(),
        execution.final_tag().microstep(),
    )
}
