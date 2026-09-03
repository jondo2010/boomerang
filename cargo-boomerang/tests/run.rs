use std::{
    ffi::OsString,
    fs,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    path::PathBuf,
    process::{Command, Output},
    sync::{Mutex, OnceLock},
};

use serde_json::{json, Value};

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

/// Runs the installed Cargo plugin for one fixture deployment in an isolated target directory.
fn run_cli(deployment: &str) -> Output {
    let target = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(fixture_workspace())
        .args(["run", "--deployment", deployment])
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .unwrap()
}

fn target_directory_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TargetDirectoryGuard {
    previous: Option<OsString>,
}

impl TargetDirectoryGuard {
    fn set(target: &std::path::Path) -> Self {
        let previous = std::env::var_os("CARGO_TARGET_DIR");
        // SAFETY: the caller holds the process-global target-directory lock.
        unsafe { std::env::set_var("CARGO_TARGET_DIR", target) };
        Self { previous }
    }
}

impl Drop for TargetDirectoryGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => {
                // SAFETY: the caller holds the process-global target-directory lock.
                unsafe { std::env::set_var("CARGO_TARGET_DIR", previous) };
            }
            None => {
                // SAFETY: the caller holds the process-global target-directory lock.
                unsafe { std::env::remove_var("CARGO_TARGET_DIR") };
            }
        }
    }
}

fn with_target_directory<T>(target: &std::path::Path, operation: impl FnOnce() -> T) -> T {
    let _lock = target_directory_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let guard = TargetDirectoryGuard::set(target);
    let result = catch_unwind(AssertUnwindSafe(operation));
    drop(guard);
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}

fn summary_json(summary: &cargo_boomerang::ExecutionSummary) -> Value {
    let stats = summary.stats();
    json!({
        "schema": 1,
        "stats": {
            "processed_tags": stats.processed_tags().to_string(),
            "processed_reactions": stats.processed_reactions().to_string(),
            "processed_events": stats.processed_events().to_string(),
            "set_ports": stats.set_ports().to_string(),
            "scheduled_actions": stats.scheduled_actions().to_string(),
        },
        "final_tag": {
            "offset_nanos": summary.final_tag().offset().whole_nanoseconds().to_string(),
            "microstep": summary.final_tag().microstep().to_string(),
        },
    })
}

#[test]
fn generated_monolith_matches_owned_reference_execution_summary() {
    let expected_directory = tempfile::tempdir().unwrap();
    let expected_path = expected_directory.path().join("summary.json");
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "production", "host").unwrap();
    let built = launcher.build_locked_offline().unwrap();
    let expected_status = Command::new(built.executable_path())
        .env("BOOMERANG_EXECUTION_SUMMARY_V1", &expected_path)
        .status()
        .unwrap();
    assert!(expected_status.success());
    let expected: Value = serde_json::from_slice(&fs::read(&expected_path).unwrap()).unwrap();

    let target = tempfile::tempdir().unwrap();
    let observed = with_target_directory(target.path(), || {
        cargo_boomerang::run(fixture_workspace(), "production")
    })
    .unwrap();
    assert!(observed.status().success());
    let observed = summary_json(observed.summary().unwrap());
    assert_eq!(observed["final_tag"], expected["final_tag"]);
    for counter in [
        "processed_tags",
        "processed_reactions",
        "set_ports",
        "scheduled_actions",
    ] {
        assert_eq!(observed["stats"][counter], expected["stats"][counter]);
    }
    assert_ne!(expected["stats"]["processed_events"], "0");
    assert_ne!(observed["stats"]["processed_events"], "0");
}

#[test]
fn generated_launcher_emits_the_versioned_execution_summary_writer() {
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "execution", "host").unwrap();
    let source = fs::read_to_string(launcher.source_path()).unwrap();

    assert!(
        source.contains("BOOMERANG_EXECUTION_SUMMARY_V1"),
        "{source}"
    );
    assert!(source.contains("create_new(true)"), "{source}");
    assert!(source.contains("execution.stats()"), "{source}");
    assert!(source.contains("execution.final_tag()"), "{source}");
}

#[test]
fn run_rejects_a_custom_target_before_bundle_generation() {
    let target = tempfile::tempdir().unwrap();
    let result = with_target_directory(target.path(), || {
        cargo_boomerang::run(fixture_workspace(), "resolution")
    });
    let error = result.unwrap_err().to_string();
    assert!(error.contains("custom target JSON"), "{error}");
    assert!(!target.path().join("boomerang/resolution").exists());
}

#[test]
fn run_forwards_application_streams_without_reframing() {
    let output = run_cli("production");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "sensor received command 42\n"
    );
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("sensor scheduling shutdown\n"));
}

#[test]
fn run_propagates_the_generated_application_exit_code() {
    let output = run_cli("runtime-failure");
    assert_eq!(output.status.code(), Some(42));
}
