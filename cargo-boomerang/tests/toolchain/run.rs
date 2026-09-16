use std::{
    fs,
    process::{Command, Output},
};

use serde_json::Value;

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

#[test]
fn run_rejects_a_custom_target_before_bundle_generation() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "resolution");
    let _manifest = support::fixture_variant("resolution", "production", |deployment| {
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("target-json".into(), "targets/host.json".into());
    });
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
    let deployment = "foreign";
    let foreign_target = if target_lexicon::HOST.to_string() == "x86_64-unknown-linux-gnu" {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let _manifest = support::fixture_variant(deployment, "production", |deployment| {
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("target".into(), foreign_target.into());
    });
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
fn generated_monolith_cli_matches_owned_execution() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let expected =
        support::with_target_directory(&target, || support::owned_reference_summary("production"));
    assert!(
        !target.join("boomerang/production").exists(),
        "owned reference must not generate a launcher"
    );
    let directory = tempfile::tempdir().unwrap();
    let summary = directory.path().join("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(support::fixture_workspace())
        .args(["run", "--deployment", "production", "-s"])
        .arg(&summary)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_TARGET", "invalid-boomerang-test-target")
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed: Value = serde_json::from_slice(&fs::read(summary).unwrap()).unwrap();
    assert_eq!(observed["schema"], 1);
    assert_eq!(observed["final_tag"], expected["final_tag"]);
    assert!(observed["final_tag"]["offset_nanos"].is_string());
    assert!(observed["final_tag"]["microstep"].is_string());
    for counter in [
        "processed_tags",
        "processed_reactions",
        "set_ports",
        "scheduled_actions",
    ] {
        assert!(observed["stats"][counter].is_number());
        assert_eq!(
            observed["stats"][counter], expected["stats"][counter],
            "{counter}"
        );
    }
    assert!(observed["stats"]["processed_events"].is_number());
    assert_ne!(expected["stats"]["processed_events"], 0);
    assert_ne!(observed["stats"]["processed_events"], 0);
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
    let _manifest = support::fixture_variant("runtime-failure", "production", |deployment| {
        deployment["bindings"]["sensor"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["simulated", "runtime-failure"]).unwrap(),
            );
    });
    let output = run_cli("runtime-failure", &[]);
    assert_eq!(output.status.code(), Some(42));
}

/// A failed generated Federate terminates the RTI and blocked peers without a success summary.
#[test]
fn generated_federate_failure_terminates_the_deployment() {
    let _guard = support::toolchain_lock();
    let _hosted = support::hosted_fixture();
    let _manifest = support::fixture_variant("central-failure", "sensor-slice", |deployment| {
        deployment["bindings"]["sensor"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["simulated", "runtime-failure"]).unwrap(),
            );
    });
    support::reset_deployment_output(&support::toolchain_target(), "central-failure");
    let directory = tempfile::tempdir().unwrap();
    let summary = directory.path().join("failed-summary.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(support::fixture_workspace())
        .args(["run", "--deployment", "central-failure", "--summary"])
        .arg(&summary)
        .env("CARGO_TARGET_DIR", support::toolchain_target())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("central RTI"), "{stderr}");
    assert!(!stderr.contains("shutdown timed out"), "{stderr}");
    assert!(
        !summary.exists(),
        "failed deployment published a success summary"
    );
}
