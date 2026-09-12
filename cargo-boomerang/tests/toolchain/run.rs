use std::{
    fs,
    process::{Command, Output},
};

use serde_json::{json, Value};

use super::support;

/// Runs the installed Cargo plugin for one fixture deployment in the shared toolchain target.
fn run_cli(deployment: &str, environment: &[(&str, &str)]) -> Output {
    run_cli_with_options(deployment, &[], environment)
}

/// Runs the installed Cargo plugin with global CLI options for one fixture deployment.
fn run_cli_with_options(
    deployment: &str,
    options: &[&str],
    environment: &[(&str, &str)],
) -> Output {
    let target = support::toolchain_target();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"));
    command
        .args(["boomerang", "--workspace"])
        .arg(support::fixture_workspace())
        .args(options)
        .args(["run", "--deployment", deployment])
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RUST_LOG");
    command.envs(environment.iter().copied()).output().unwrap()
}

/// Runs the installed Cargo plugin with options specific to the `run` command.
fn run_cli_with_run_options(deployment: &str, options: &[&str]) -> Output {
    let target = support::toolchain_target();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"));
    command
        .args(["boomerang", "--workspace"])
        .arg(support::fixture_workspace())
        .args(["run", "--deployment", deployment])
        .args(options)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

fn summary_json(summary: &cargo_boomerang::ExecutionSummary) -> Value {
    let stats = summary.stats();
    json!({
        "schema": 1,
        "stats": {
            "processed_tags": stats.processed_tags(),
            "processed_reactions": stats.processed_reactions(),
            "processed_events": stats.processed_events(),
            "set_ports": stats.set_ports(),
            "scheduled_actions": stats.scheduled_actions(),
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
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let expected =
        support::with_target_directory(&target, || support::owned_reference_summary("production"));
    assert!(
        !target.join("boomerang/production").exists(),
        "owned reference execution must not generate a launcher"
    );

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
    assert_ne!(expected["stats"]["processed_events"], 0);
    assert_ne!(observed["stats"]["processed_events"], 0);
}

/// Persists the decoded execution summary when the CLI receives `-s`.
#[test]
fn run_writes_the_requested_execution_summary() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let directory = tempfile::tempdir().unwrap();
    let summary_path = directory.path().join("result.json");
    let summary_path_argument = summary_path.to_str().unwrap();

    let output = run_cli_with_run_options("production", &["-s", summary_path_argument]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(document["schema"], 1);
    assert!(document["stats"]["processed_tags"].is_number());
    assert!(document["stats"]["processed_reactions"].is_number());
    assert!(document["stats"]["processed_events"].is_number());
    assert!(document["stats"]["set_ports"].is_number());
    assert!(document["stats"]["scheduled_actions"].is_number());
    assert!(document["final_tag"]["offset_nanos"].is_string());
    assert!(document["final_tag"]["microstep"].is_string());
}

#[test]
fn generated_launcher_manifest_uses_canonical_host_support() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(support::fixture_workspace(), "execution", "host")
            .unwrap()
    });
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(launcher.manifest_path()).unwrap()).unwrap();
    let dependencies = manifest["dependencies"].as_table().unwrap();

    assert_eq!(
        dependencies["boomerang_util"]["features"]
            .as_array()
            .unwrap(),
        &[toml::Value::String(String::from("launcher"))]
    );
    assert!(!dependencies.contains_key("tracing-subscriber"));
    launcher.build_locked_offline().unwrap();
    launcher.run_locked_offline().unwrap();
}

/// Rejects distributed execution until the compiled central RTI runner lands.
#[test]
fn run_rejects_distributed_execution_until_issue_131() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "sensor-slice");
    let error = support::with_target_directory(&target, || {
        cargo_boomerang::run(support::fixture_workspace(), "sensor-slice")
    })
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "distributed deployment execution is unsupported until issue #131"
    );
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
    let output = run_cli("production", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "sensor received command 42\n"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Building"), "{stderr}");
    assert!(stderr.contains("Running"), "{stderr}");
    support::assert_progress_phases(
        &stderr,
        &[
            "Analyzing",
            "Generating",
            "Building",
            "Validating",
            "Generating",
            "Building",
            "Bundling",
            "Publishing",
            "Published",
            "Validating",
            "Running",
        ],
    );
    let plain_stderr = support::without_ansi(&stderr);
    let running_line = plain_stderr
        .lines()
        .find(|line| line.split_whitespace().next() == Some("Running"))
        .expect("run reports its executable");
    let reported = running_line
        .trim_start()
        .strip_prefix("Running ")
        .unwrap()
        .split_once(" (deployment ")
        .unwrap()
        .0;
    let reported = std::path::Path::new(reported);
    assert!(reported.is_absolute(), "{}", reported.display());
    assert_eq!(
        reported.file_name().unwrap(),
        format!("launcher{}", std::env::consts::EXE_SUFFIX).as_str()
    );
    assert!(
        plain_stderr.find(running_line).unwrap()
            < plain_stderr.find("sensor scheduling shutdown").unwrap(),
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("sensor scheduling shutdown\n").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.ends_with("sensor scheduling shutdown\n"), "{stderr}");
}

#[test]
fn quiet_run_suppresses_tool_progress_but_not_application_streams() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = run_cli_with_options("production", &["--quiet"], &[]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout, "sensor received command 42\n");
    support::assert_progress_phases(&stderr, &[]);
    assert!(stderr.contains("sensor scheduling shutdown\n"), "{stderr}");
}

#[test]
fn implicit_host_run_overrides_ambient_cargo_build_target() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = run_cli(
        "production",
        &[("CARGO_BUILD_TARGET", "invalid-boomerang-test-target")],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "sensor received command 42\n"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    support::assert_progress_phases(
        &stderr,
        &[
            "Analyzing",
            "Generating",
            "Building",
            "Validating",
            "Generating",
            "Building",
            "Bundling",
            "Publishing",
            "Published",
            "Validating",
            "Running",
        ],
    );
}

#[test]
fn generated_launcher_honors_rust_log_trace() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = run_cli("production", &[("RUST_LOG", "trace")]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "sensor received command 42\n"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("TRACE"), "{stderr}");
    assert!(stderr.contains("boomerang_runtime"), "{stderr}");
}

#[test]
fn run_propagates_the_generated_application_exit_code() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "runtime-failure");
    let output = run_cli("runtime-failure", &[]);
    assert_eq!(output.status.code(), Some(42));
}
