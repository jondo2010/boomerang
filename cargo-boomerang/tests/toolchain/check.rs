use std::path::PathBuf;

use super::support;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

#[test]
fn check_runs_complete_host_analysis_without_building_payloads() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "check", "--deployment", "production"])
        .current_dir(fixture_workspace())
        .env("CARGO_TARGET_DIR", target.as_path())
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stdout.is_empty(),
        "unexpected stdout: {:?}",
        result.stdout
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    for phase in [
        "Analyzing",
        "Generating",
        "Building",
        "Validating",
        "Publishing",
    ] {
        assert!(stderr.contains(phase), "missing {phase} status:\n{stderr}");
    }
    support::assert_progress_phases(
        &stderr,
        &[
            "Analyzing",
            "Generating",
            "Building",
            "Validating",
            "Publishing",
        ],
    );
    let report_path = target.as_path().join("boomerang/production/check.json");
    assert!(report_path.exists(), "missing {}", report_path.display());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["compiler_schema"], 1);
    assert_eq!(report["deployment"], "production");
    assert!(report["topology_digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("blake3:")));
    assert_eq!(report["resources"]["federates"][0]["id"], "host");
    assert_eq!(
        report["resources"]["federates"][0]["target"],
        target_lexicon::HOST.to_string()
    );
    assert_eq!(
        report["resources"]["federates"][0]["enclaves"][0]["payload_bytes"],
        1024
    );
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert!(!target
        .as_path()
        .join("boomerang/production/artifacts")
        .exists());
}

#[test]
fn check_accepts_an_explicit_workspace_outside_the_current_directory() {
    let _guard = support::toolchain_lock();
    let current = tempfile::tempdir().unwrap();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .arg("boomerang")
        .arg("--workspace")
        .arg(fixture_workspace())
        .args(["check", "--deployment", "production"])
        .current_dir(current.path())
        .env("CARGO_TARGET_DIR", target.as_path())
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(target
        .as_path()
        .join("boomerang/production/check.json")
        .exists());
}

#[test]
fn descriptor_failure_reports_actionable_diagnostic_before_cargo_summary() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "broken-descriptor");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "check", "--deployment", "broken-descriptor"])
        .current_dir(fixture_workspace())
        .env("CARGO_TARGET_DIR", target)
        .output()
        .unwrap();
    let stderr = String::from_utf8(result.stderr).unwrap();
    let plain_stderr = support::without_ansi(&stderr);

    assert!(!result.status.success(), "{stderr}");
    let diagnostic = plain_stderr
        .find("error: intentional descriptor build failure")
        .expect("missing rendered descriptor diagnostic");
    let summary = plain_stderr
        .find("error: could not compile `vehicle-control`")
        .expect("missing Cargo failure summary");
    assert!(
        diagnostic < summary,
        "descriptor diagnostic was out of order:\n{stderr}"
    );
}
