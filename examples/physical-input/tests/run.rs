//! Execute the checked-in application through Cargo's binary target.

use std::process::Command;

/// Preserve timestamped delivery and successful finite-worker teardown.
#[test]
fn sample_arrives_at_acquisition_plus_delay() {
    let output = Command::new(env!("CARGO_BIN_EXE_physical-input"))
        .env_remove("RUST_LOG")
        .output()
        .expect("physical-input starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("received 42 at logical 11 ms (physical 11 ms)"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
