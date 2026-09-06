use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cargo_metadata::MetadataCommand;
use serde_json::Value;

use super::support;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

fn build_fixture(deployment: &str, target: &Path) -> Output {
    build_fixture_with_options(deployment, target, &[])
}

fn build_fixture_with_options(deployment: &str, target: &Path, options: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .arg("boomerang")
        .arg("--workspace")
        .arg(fixture_workspace())
        .args(options)
        .args(["build", "--deployment", deployment])
        .env("CARGO_TARGET_DIR", target)
        .output()
        .unwrap()
}

fn assert_no_staging_residue(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        assert!(
            !entry.file_name().to_string_lossy().contains(".staging-"),
            "build left staging residue at {}",
            entry.path().display()
        );
        if entry.file_type().unwrap().is_dir() {
            assert_no_staging_residue(&entry.path());
        }
    }
}

#[test]
fn build_reports_cargo_style_progress_without_polluting_stdout() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = build_fixture("production", &target);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    assert!(stderr.contains("Building"), "{stderr}");
    assert!(stderr.contains("Bundling"), "{stderr}");
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
        ],
    );
}

#[test]
fn quiet_build_keeps_its_machine_readable_result_without_progress() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output =
        build_fixture_with_options("production", &target, &["--quiet", "--color", "always"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    support::assert_progress_phases(&stderr, &[]);
    assert!(!stderr.contains('\u{1b}'), "unexpected color: {stderr:?}");
}

#[test]
fn color_always_styles_progress_without_styling_stdout() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = build_fixture_with_options("production", &target, &["--color", "always"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    assert!(!stdout.contains('\u{1b}'), "unexpected color: {stdout:?}");
    assert!(stderr.contains("\u{1b}[1;32m"), "missing color: {stderr:?}");
}

#[test]
fn verbose_build_forwards_nested_cargo_output_between_progress_phases() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let output = build_fixture_with_options("production", &target, &["--verbose"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
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
        ],
    );
    let plain_stderr = support::without_ansi(&stderr);
    let nested = plain_stderr
        .lines()
        .position(|line| {
            let line = line.trim_start();
            line.starts_with("Fresh ") || line.starts_with("Compiling ")
        })
        .expect("verbose output should include nested Cargo activity");
    let building = plain_stderr
        .lines()
        .position(|line| line.split_whitespace().next() == Some("Building"))
        .unwrap();
    assert!(
        building < nested,
        "nested Cargo output was out of order:\n{stderr}"
    );
}

#[test]
fn successful_build_preserves_compiler_warnings_in_phase_order() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "warning-diagnostic");
    let output = build_fixture("warning-diagnostic", &target);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "{stderr}");
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    let plain_stderr = support::without_ansi(&stderr);
    let building = plain_stderr.find("Building launcher").unwrap();
    let warning = plain_stderr
        .find("INTENTIONAL_TARGET_PAYLOAD_WARNING")
        .unwrap_or_else(|| panic!("successful Cargo warning was missing:\n{stderr}"));
    let bundling = plain_stderr.find("Bundling deployment").unwrap();
    assert!(building < warning && warning < bundling, "{stderr}");
}

#[test]
fn broken_payload_preserves_diagnostics_without_publishing_a_bundle() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "broken-payload");
    let result = build_fixture("broken-payload", &target);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let plain_stderr = support::without_ansi(&stderr);

    assert!(!result.status.success(), "{stderr}");
    assert!(
        plain_stderr.contains("intentional target payload build failure"),
        "expected target payload compilation to fail, got:\n{stderr}"
    );
    assert!(
        plain_stderr.contains("deployment 'broken-payload'"),
        "{stderr}"
    );
    assert!(plain_stderr.contains("Federate 'host'"), "{stderr}");
    assert_eq!(
        plain_stderr
            .matches("error: intentional target payload build failure")
            .count(),
        1,
        "compiler diagnostic was duplicated:\n{stderr}"
    );
    assert!(
        plain_stderr.find("Building").unwrap()
            < plain_stderr
                .find("intentional target payload build failure")
                .unwrap(),
        "compiler diagnostic preceded its build status:\n{stderr}"
    );
    let output_directory = target.join("boomerang/broken-payload");
    if output_directory.exists() {
        assert!(!output_directory.join("deployment.json").exists());
        assert!(
            fs::read_dir(&output_directory).unwrap().next().is_none(),
            "failed build left entries in {}",
            output_directory.display()
        );
    }
}

#[test]
fn build_publishes_a_valid_fingerprinted_bundle() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let result = build_fixture("production", &target);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let stdout = String::from_utf8(result.stdout).unwrap();
    let manifest_path = fs::canonicalize(PathBuf::from(stdout.trim())).unwrap();
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    let target_directory = fs::canonicalize(&target).unwrap();
    let relative = manifest_path.strip_prefix(&target_directory).unwrap();
    let components = relative
        .iter()
        .map(|component| component.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(components[0..2], ["boomerang", "production"]);
    assert_eq!(components[3], "deployment.json");
    let fingerprint = components[2];
    assert_eq!(fingerprint.len(), 64);
    assert!(
        fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "deployment fingerprint is not lowercase hexadecimal: {fingerprint}"
    );

    let document: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(document["schema"], 1);
    assert_eq!(document["deployment"], "production");
    assert_eq!(document["coordination"]["backend"], "local");
    assert!(document["coordination"]["protocol"].is_null());
    assert_eq!(document["federates"][0]["id"], "host");
    assert_eq!(
        document["federates"][0]["groups"],
        serde_json::json!([
            "placement/backup",
            "placement/controller",
            "placement/sensor"
        ])
    );
    assert_eq!(document["federates"][0]["runtime"], "std");
    assert_eq!(
        document["execution"],
        serde_json::json!({
            "fast_forward": false,
            "keep_alive": false,
            "logical_horizon_nanos": null,
        })
    );
    assert!(document.get("runtime_configuration").is_none());

    let bundle = manifest_path.parent().unwrap();
    let source_lock_hash = blake3::hash(&fs::read(fixture_workspace().join("Cargo.lock")).unwrap())
        .to_hex()
        .to_string();
    let generated_lock_hash =
        blake3::hash(&fs::read(bundle.join("generated/host/Cargo.lock")).unwrap())
            .to_hex()
            .to_string();
    let generated_source_hash =
        blake3::hash(&fs::read(bundle.join("generated/host/src/main.rs")).unwrap())
            .to_hex()
            .to_string();
    assert_eq!(document["source_lock_hash"], source_lock_hash);
    assert_eq!(document["generated_lock_hash"], generated_lock_hash);
    assert_eq!(document["generated_source_hash"], generated_source_hash);
    for collection in ["generated", "artifacts"] {
        let records = document[collection].as_array().unwrap();
        assert!(!records.is_empty(), "missing {collection} file records");
        for record in records {
            let relative = record["path"].as_str().unwrap();
            assert!(!relative.contains('\\'));
            let path = bundle.join(relative);
            let actual = blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string();
            let recorded = record["blake3"].as_str().unwrap();
            assert_eq!(recorded, actual, "wrong hash for {relative}");
            assert!(
                recorded
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "recorded hash is not lowercase hexadecimal: {recorded}"
            );
        }
    }

    let generated_manifest = bundle.join("generated/host/Cargo.toml");
    let metadata = MetadataCommand::new()
        .manifest_path(&generated_manifest)
        .other_options(vec![String::from("--locked"), String::from("--offline")])
        .exec()
        .unwrap();
    let package_names = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<Vec<_>>();
    assert!(!package_names.contains(&"boomerang_builder"));
    assert!(!package_names.contains(&"vehicle-topology"));
    assert!(package_names.contains(&"sensor-host"));
    assert!(package_names.contains(&"vehicle-control"));

    let resources = document["resources"]["federates"][0]["enclaves"]
        .as_array()
        .unwrap();
    assert!(
        resources
            .iter()
            .all(|enclave| enclave.get("event_capacity").is_some()),
        "each Enclave resource record must retain its authoritative event capacity: {resources:?}"
    );
}

#[test]
fn repeated_build_preserves_the_same_published_bundle() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let first = build_fixture("production", &target);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_manifest = PathBuf::from(String::from_utf8(first.stdout).unwrap().trim());
    let first_document: Value =
        serde_json::from_slice(&fs::read(&first_manifest).unwrap()).unwrap();
    let artifact = first_manifest
        .parent()
        .unwrap()
        .join(first_document["artifacts"][0]["path"].as_str().unwrap());
    let manifest_before = fs::read(&first_manifest).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    let artifact_hash_before = blake3::hash(&artifact_before);

    let second = build_fixture("production", &target);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_manifest = PathBuf::from(String::from_utf8(second.stdout).unwrap().trim());

    assert_eq!(second_manifest, first_manifest);
    assert_eq!(fs::read(&first_manifest).unwrap(), manifest_before);
    let artifact_after = fs::read(&artifact).unwrap();
    assert_eq!(artifact_after, artifact_before);
    assert_eq!(blake3::hash(&artifact_after), artifact_hash_before);
    assert_no_staging_residue(&target.join("boomerang/generated"));
    assert_no_staging_residue(first_manifest.parent().unwrap().parent().unwrap());
}

#[test]
fn build_normalizes_deployment_execution_policy_into_every_published_artifact() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "execution");
    support::reset_deployment_output(&target, "execution-equivalent");
    let result = build_fixture("execution", &target);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let manifest = PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
    let document: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(document["schema"], 1);
    assert_eq!(
        document["execution"],
        serde_json::json!({
            "fast_forward": true,
            "keep_alive": true,
            "logical_horizon_nanos": 1_000_000_000u64,
        })
    );
    assert!(document.get("runtime_configuration").is_none());

    let equivalent = build_fixture("execution-equivalent", &target);
    assert!(
        equivalent.status.success(),
        "{}",
        String::from_utf8_lossy(&equivalent.stderr)
    );
    let equivalent_manifest = PathBuf::from(String::from_utf8(equivalent.stdout).unwrap().trim());
    let equivalent_document: Value =
        serde_json::from_slice(&fs::read(&equivalent_manifest).unwrap()).unwrap();
    assert_eq!(equivalent_document["execution"], document["execution"]);
    assert_eq!(equivalent_document["fingerprint"], document["fingerprint"]);
    assert_eq!(
        equivalent_manifest.parent().unwrap().file_name(),
        manifest.parent().unwrap().file_name(),
        "equivalent policies must publish under the same fingerprint"
    );
    assert_eq!(
        equivalent_document["generated_source_hash"],
        document["generated_source_hash"]
    );

    let source = fs::read_to_string(
        manifest
            .parent()
            .unwrap()
            .join("generated/host/src/main.rs"),
    )
    .unwrap();
    assert!(source.contains("fast_forward: true"), "{source}");
    assert!(source.contains("keep_alive: true"), "{source}");
    assert!(
        source.contains("timeout: Some(boomerang_runtime::Duration::nanoseconds_i128(1000000000))"),
        "{source}"
    );
    let equivalent_source = fs::read_to_string(
        equivalent_manifest
            .parent()
            .unwrap()
            .join("generated/host/src/main.rs"),
    )
    .unwrap();
    assert_eq!(equivalent_source, source);
}

#[test]
fn corrupted_published_artifact_causes_a_conflict_without_overwrite() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let first = build_fixture("production", &target);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let manifest = PathBuf::from(String::from_utf8(first.stdout).unwrap().trim());
    let document: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let artifact = manifest
        .parent()
        .unwrap()
        .join(document["artifacts"][0]["path"].as_str().unwrap());
    let mut corrupted = fs::read(&artifact).unwrap();
    corrupted[0] ^= 1;
    fs::write(&artifact, &corrupted).unwrap();
    let manifest_before = fs::read(&manifest).unwrap();

    let second = build_fixture("production", &target);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(!second.status.success(), "{stderr}");
    assert!(stderr.contains("conflict"), "{stderr}");
    assert_eq!(fs::read(&artifact).unwrap(), corrupted);
    assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
}

#[test]
fn build_accepts_an_explicit_workspace_outside_the_current_directory() {
    let _guard = support::toolchain_lock();
    let current = tempfile::tempdir().unwrap();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let result = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .arg("boomerang")
        .arg("--workspace")
        .arg(fixture_workspace())
        .args(["build", "--deployment", "production"])
        .current_dir(current.path())
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = fs::canonicalize(PathBuf::from(
        String::from_utf8(result.stdout).unwrap().trim(),
    ))
    .unwrap();
    assert!(manifest.exists(), "missing {}", manifest.display());
    assert!(manifest.starts_with(fs::canonicalize(target).unwrap()));
}

#[test]
fn build_applies_configured_release_profile_and_cargo_configuration() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "profile-config");
    let result = build_fixture("profile-config", &target);

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
    assert!(manifest.exists(), "missing {}", manifest.display());
}
