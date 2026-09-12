//! Test-only tracing subscriber helpers.

use tracing_subscriber::filter::EnvFilter;

/// Install a fmt subscriber for tests and always add `directive` to the filter.
///
/// Output is captured by libtest unless the test is run with `-- --nocapture`.
pub fn init_with_directive(directive: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive(directive.parse().expect("valid tracing filter directive"));
    crate::launcher::init_test_tracing_with_filter(filter);
}
