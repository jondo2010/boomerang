//! Process tests consume executables built before this test binary starts.
mod application_support;

use std::path::PathBuf;

fn executables() -> PathBuf {
    std::env::var_os("BOOMERANG_TEST_BIN_DIR")
        .map(PathBuf::from)
        .expect("build fixtures with bash boomerang_codegen/tests/run-applications.sh")
}

#[test]
#[ignore = "run bash boomerang_codegen/tests/run-applications.sh"]
fn local_application_initializes_and_routes_inputs() {
    application_support::local(&executables());
}

#[test]
#[ignore = "run bash boomerang_codegen/tests/run-applications.sh"]
fn independent_artifacts_exchange_and_reject_mixed_deployments() {
    application_support::federation(&executables());
}
