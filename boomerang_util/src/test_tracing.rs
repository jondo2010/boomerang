//! Test-only tracing subscriber helpers.

use tracing_subscriber::{filter::EnvFilter, fmt::format::FmtSpan};

/// Install a fmt subscriber for tests and always add `directive` to the filter.
///
/// Output is captured by libtest unless the test is run with `-- --nocapture`.
pub fn init_with_directive(directive: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive(directive.parse().expect("valid tracing filter directive"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .with_test_writer()
        .try_init();
}
