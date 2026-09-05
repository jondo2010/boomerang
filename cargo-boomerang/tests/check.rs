use std::path::PathBuf;

mod support;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

#[test]
fn check_runs_complete_host_analysis_without_building_payloads() {
    let target = support::shared_target("analysis");
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
    let current = tempfile::tempdir().unwrap();
    let target = support::shared_target("analysis");
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
