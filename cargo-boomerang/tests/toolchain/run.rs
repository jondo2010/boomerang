use std::{
    fs,
    process::{Command, Output},
};

use serde_json::{json, Value};

use super::support;

/// Runs the installed Cargo plugin for one fixture deployment in the shared toolchain target.
fn run_cli(deployment: &str) -> Output {
    let target = support::toolchain_target();
    Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(support::fixture_workspace())
        .args(["run", "--deployment", deployment])
        .env("CARGO_TARGET_DIR", target)
        .output()
        .unwrap()
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
    let _guard = support::toolchain_lock();
    let expected_directory = tempfile::tempdir().unwrap();
    let expected_path = expected_directory.path().join("summary.json");
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let built = support::with_target_directory(&target, || {
        let launcher =
            cargo_boomerang::generate_launcher(support::fixture_workspace(), "production", "host")
                .unwrap();
        launcher.build_locked_offline().unwrap()
    });
    let expected_status = Command::new(built.executable_path())
        .env("BOOMERANG_EXECUTION_SUMMARY_V1", &expected_path)
        .status()
        .unwrap();
    assert!(expected_status.success());
    let expected: Value = serde_json::from_slice(&fs::read(&expected_path).unwrap()).unwrap();

    let observed = support::with_target_directory(&target, || {
        cargo_boomerang::run(support::fixture_workspace(), "production")
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
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(support::fixture_workspace(), "execution", "host")
            .unwrap()
    });
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
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "resolution");
    let result = support::with_target_directory(&target, || {
        cargo_boomerang::run(support::fixture_workspace(), "resolution")
    });
    let error = result.unwrap_err().to_string();
    assert!(error.contains("custom target JSON"), "{error}");
    assert!(!target.join("boomerang/resolution").exists());
}

#[test]
fn run_rejects_a_foreign_native_target_before_bundle_generation() {
    let _guard = support::toolchain_lock();
    let deployment = if target_lexicon::HOST.to_string() == "x86_64-unknown-linux-gnu" {
        "foreign-aarch64-macos"
    } else {
        "foreign-x86-linux"
    };
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, deployment);
    let result = support::with_target_directory(&target, || {
        cargo_boomerang::run(support::fixture_workspace(), deployment)
    });
    let error = result.unwrap_err().to_string();

    assert!(error.contains("is not the host target"), "{error}");
    assert!(!target.join("boomerang").join(deployment).exists());
}

#[test]
fn run_forwards_application_streams_without_reframing() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
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
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sensor scheduling shutdown\n"
    );
}

#[test]
fn run_propagates_the_generated_application_exit_code() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "runtime-failure");
    let output = run_cli("runtime-failure");
    assert_eq!(output.status.code(), Some(42));
}
