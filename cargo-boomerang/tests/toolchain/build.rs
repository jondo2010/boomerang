#[cfg(feature = "monitor")]
use std::{collections::BTreeSet, net::UdpSocket, time::Duration};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cargo_metadata::MetadataCommand;
use serde_json::Value;

use super::support;

fn fixture_workspace() -> PathBuf {
    support::fixture_workspace()
}

fn build_fixture(deployment: &str, target: &Path) -> Output {
    build_fixture_with_options(deployment, target, &[])
}

fn build_fixture_with_options(deployment: &str, target: &Path, options: &[&str]) -> Output {
    let current = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .current_dir(current.path())
        .arg("boomerang")
        .arg("--workspace")
        .arg(fixture_workspace())
        .args(options)
        .args(["build", "--deployment", deployment])
        .env("CARGO_TARGET_DIR", target)
        .output()
        .unwrap()
}

/// Resolves the sole executable recorded by a single-Federate build result.
fn published_executable(stdout: &str) -> PathBuf {
    let manifest = PathBuf::from(stdout.trim());
    let artifacts = manifest.parent().unwrap().join("artifacts").join("host");
    let mut entries = fs::read_dir(artifacts)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(entries.len(), 1, "unexpected published artifacts");
    entries.pop().unwrap()
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
    let executable = published_executable(&stdout);
    assert!(
        !support::without_ansi(&stderr).contains(executable.to_string_lossy().as_ref()),
        "unexpected published path on quiet stderr: {stderr:?}"
    );
    support::assert_progress_phases(&stderr, &[]);
    assert!(!stderr.contains('\u{1b}'), "unexpected color: {stderr:?}");
}

#[test]
fn verbose_colored_build_preserves_compiler_diagnostics() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "warning-diagnostic");
    let _manifest = support::fixture_variant("warning-diagnostic", "production", |deployment| {
        deployment["bindings"]["controller"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["warning-diagnostic"]).unwrap(),
            );
    });
    let output = build_fixture_with_options(
        "warning-diagnostic",
        &target,
        &["--verbose", "--color", "always"],
    );
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
            "Generating",
            "Building",
            "Validating",
            "Generating",
            "Building",
            "Bundling",
            "Publishing",
            "Published",
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
    assert!(!stdout.contains('\u{1b}'), "unexpected color: {stdout:?}");
    assert!(stderr.contains("\u{1b}[1;32m"), "missing color: {stderr:?}");
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
    let _manifest = support::fixture_variant("broken-payload", "production", |deployment| {
        deployment["bindings"]["controller"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["broken-payload"]).unwrap(),
            );
    });
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
fn build_publishes_reuses_and_protects_a_fingerprinted_bundle() {
    let _guard = support::toolchain_lock();
    let _manifest = support::fixture_variant("production", "production", |deployment| {
        deployment["bindings"]["controller"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["warning-diagnostic"]).unwrap(),
            );
    });
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "production");
    let result = build_fixture("production", &target);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let stdout = String::from_utf8(result.stdout).unwrap();
    let executable = published_executable(&stdout);
    assert!(executable.is_absolute(), "{}", executable.display());
    assert!(
        support::without_ansi(&stderr).contains(&format!(
            "Published Federate 'host' executable {}",
            executable.display()
        )),
        "{stderr}"
    );
    assert!(stderr.contains("Building"), "{stderr}");
    assert!(stderr.contains("Bundling"), "{stderr}");
    support::assert_progress_phases(
        &stderr,
        &[
            "Analyzing",
            "Generating",
            "Building",
            "Generating",
            "Building",
            "Validating",
            "Generating",
            "Building",
            "Bundling",
            "Publishing",
            "Published",
        ],
    );
    let plain_stderr = support::without_ansi(&stderr);
    let building = plain_stderr.find("Building launcher").unwrap();
    let warning = plain_stderr
        .find("INTENTIONAL_TARGET_PAYLOAD_WARNING")
        .expect("default output must retain successful compiler warnings");
    let bundling = plain_stderr.find("Bundling deployment").unwrap();
    assert!(building < warning && warning < bundling, "{stderr}");
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
    // Reuse the generated build to inspect the exact target artifacts, since unfiltered
    // metadata also includes the facade's cfg-disabled hosted dependencies.
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(fixture_workspace(), "production", "host")
    })
    .unwrap();
    let built = launcher.build_locked_offline().unwrap();
    let payload_crates = support::launcher_payload_crates(built.executable_path());
    assert!(payload_crates.contains("sensor_host"));
    assert!(payload_crates.contains("vehicle_control"));
    assert!(
        !payload_crates.contains("boomerang_builder"),
        "{payload_crates:?}"
    );
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
    let artifact = manifest_path
        .parent()
        .unwrap()
        .join(document["artifacts"][0]["path"].as_str().unwrap());
    let manifest_before = fs::read(&manifest_path).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    let artifact_hash_before = blake3::hash(&artifact_before);

    let second = build_fixture("production", &target);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_manifest = fs::canonicalize(PathBuf::from(
        String::from_utf8(second.stdout).unwrap().trim(),
    ))
    .unwrap();

    assert_eq!(second_manifest, manifest_path);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
    let artifact_after = fs::read(&artifact).unwrap();
    assert_eq!(artifact_after, artifact_before);
    assert_eq!(blake3::hash(&artifact_after), artifact_hash_before);
    assert_no_staging_residue(&target.join("boomerang/generated"));
    assert_no_staging_residue(manifest_path.parent().unwrap().parent().unwrap());
    let mut corrupted = fs::read(&artifact).unwrap();
    corrupted[0] ^= 1;
    fs::write(&artifact, &corrupted).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();

    let second = build_fixture("production", &target);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(!second.status.success(), "{stderr}");
    assert!(stderr.contains("conflict"), "{stderr}");
    assert_eq!(fs::read(&artifact).unwrap(), corrupted);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
}

/// Publishes one canonical generated workspace and artifact per compiled Federate.
#[test]
fn generated_central_deployment_publishes_isolated_artifacts_and_exchanges_tagged_payload() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "sensor-slice");
    let _hosted = support::hosted_fixture();
    let directory = tempfile::tempdir().unwrap();
    let summary = directory.path().join("summary.json");
    let result = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(fixture_workspace())
        .args(["run", "--deployment", "sensor-slice", "--summary"])
        .arg(&summary)
        // A legacy runtime switch must not override the compiled hosted choice.
        .env("BOOMERANG_TRACE_MODE", "bounded")
        .env("RUST_LOG", "boomerang::coordination=debug")
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let trace = String::from_utf8_lossy(&result.stderr);
    for event in [
        "coordination.reaction.started",
        "coordination.publication.sent",
        "coordination.codec.encoded",
        "coordination.transport.encoded",
        "coordination.transport.decoded",
        "coordination.rti.payload.forwarded",
        "coordination.rti.grant.issued",
        "coordination.rti.accounting.completed",
        "coordination.boundary.admitted",
        "coordination.reaction.finished",
    ] {
        assert!(trace.contains(event), "missing {event}: {trace}");
    }
    for (event, member) in [
        ("coordination.transport.encoded", 0),
        ("coordination.transport.decoded", 1),
        ("coordination.boundary.admitted", 1),
    ] {
        let line = trace.lines().find(|line| line.contains(event)).unwrap();
        assert!(line.contains(&format!("federate={member}")), "{line}");
        assert!(line.contains("coordination=["), "{line}");
        assert!(line.contains("route=0"), "{line}");
    }
    let fingerprint = |line: &str| {
        line.split("coordination=")
            .nth(1)
            .and_then(|rest| rest.split_once(']').map(|(value, _)| value.to_owned()))
            .unwrap()
    };
    let encoded = trace
        .lines()
        .find(|line| line.contains("coordination.transport.encoded"))
        .unwrap();
    for event in [
        "coordination.transport.decoded",
        "coordination.boundary.admitted",
        "coordination.rti.payload.forwarded",
    ] {
        let line = trace.lines().find(|line| line.contains(event)).unwrap();
        assert_eq!(fingerprint(line), fingerprint(encoded));
    }
    let queued = trace
        .lines()
        .find(|line| line.contains("coordination.transport.queued") && line.contains("federate=1"))
        .expect("server queue trace has peer context");
    assert_eq!(fingerprint(queued), fingerprint(encoded));

    // Build separate artifacts for each subscriber choice. Bounded
    // capture is lossy under contention, but must report every kind of loss
    // separately and must not change the application result.
    let mut bounded_document: Option<Value> = None;
    for variant in ["off", "bounded", "bounded-small", "bounded-filter"] {
        let mode = if variant == "off" { "off" } else { "bounded" };
        let deployment = format!("sensor-slice-{variant}");
        support::reset_deployment_output(&target, &deployment);
        let _variant = support::fixture_variant(&deployment, "sensor-slice", |config| {
            config
                .as_table_mut()
                .unwrap()
                .insert("tracing".into(), mode.into());
            if variant == "bounded-small" {
                config.as_table_mut().unwrap().insert(
                    "bounded-tracing".into(),
                    toml::toml! {
                        records = 1
                        bytes = 2048
                        span-bytes = 512
                    }
                    .into(),
                );
                config["federates"]["sensor"]
                    .as_table_mut()
                    .unwrap()
                    .insert("bounded-tracing".into(), toml::toml! { records = 2 }.into());
                config["rti"]
                    .as_table_mut()
                    .unwrap()
                    .insert("bounded-tracing".into(), toml::toml! { records = 3 }.into());
            }
            if variant == "bounded-filter" {
                config.as_table_mut().unwrap().insert(
                    "bounded-tracing".into(),
                    toml::toml! { level = "trace"
                    targets = ["boomerang::runtime"] }
                    .into(),
                );
                config["federates"]["sensor"]
                    .as_table_mut()
                    .unwrap()
                    .insert(
                        "bounded-tracing".into(),
                        toml::toml! { level = "debug"
                        targets = ["boomerang::coordination"] }
                        .into(),
                    );
                config["rti"].as_table_mut().unwrap().insert(
                    "bounded-tracing".into(),
                    toml::toml! { level = "off" }.into(),
                );
            }
        });
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
            .args(["boomerang", "--workspace"])
            .arg(fixture_workspace())
            .args(["run", "--deployment", &deployment])
            .env(
                "BOOMERANG_TRACE_MODE",
                if mode == "off" { "bounded" } else { "off" },
            )
            .env("RUST_LOG", "boomerang::coordination=debug")
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{mode}: {stderr}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("sensor received command 42"));
        let bundles: Vec<_> = fs::read_dir(target.join("boomerang").join(&deployment))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.join("deployment.json").is_file())
            .collect();
        assert_eq!(bundles.len(), 1);
        let trace_document: Value =
            serde_json::from_slice(&fs::read(bundles[0].join("deployment.json")).unwrap()).unwrap();
        if variant == "bounded" {
            bounded_document = Some(trace_document.clone());
        } else if variant == "bounded-small" {
            let previous = bounded_document.as_ref().unwrap();
            assert_ne!(trace_document["fingerprint"], previous["fingerprint"]);
            assert_eq!(trace_document["coordination"], previous["coordination"]);
            assert_eq!(trace_document["federates"], previous["federates"]);
            for (limits, records) in [
                (
                    &trace_document["resources"]["federates"][0]["bounded_tracing"],
                    1,
                ),
                (
                    &trace_document["resources"]["federates"][1]["bounded_tracing"],
                    2,
                ),
                (&trace_document["resources"]["rti_bounded_tracing"], 3),
            ] {
                assert_eq!(limits["records"], records);
                assert_eq!(limits["bytes"], 2048);
                assert_eq!(limits["span_bytes"], 512);
                assert_eq!(limits["fields"], 32);
                assert_eq!(limits["spans"], 64);
                assert_eq!(limits["span_fields"], 16);
                assert_eq!(limits["producers"], 64);
                assert_eq!(limits["depth"], 16);
            }
        }
        for role in ["rti", "federates/host", "federates/sensor"] {
            assert_trace_dependencies(
                &bundles[0].join(format!("generated/{role}/Cargo.toml")),
                mode,
            );
        }
        if mode == "off" {
            assert!(!stderr.contains("coordination."), "{stderr}");
            continue;
        }
        let documents: Vec<Value> = stderr
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str(line).expect("complete bounded JSON line"))
            .collect();
        let losses: Vec<_> = documents
            .iter()
            .filter(|line| line["kind"] == "loss")
            .collect();
        if variant == "bounded-filter" {
            let resources = &trace_document["resources"];
            assert_eq!(
                resources["federates"][0]["bounded_tracing"]["level"],
                "trace"
            );
            assert_eq!(
                resources["federates"][1]["bounded_tracing"]["level"],
                "debug"
            );
            assert_eq!(resources["rti_bounded_tracing"]["level"], "off");
            assert_eq!(
                resources["rti_bounded_tracing"]["targets"],
                serde_json::json!(["boomerang::runtime"])
            );
            let previous = bounded_document.as_ref().unwrap();
            assert_ne!(trace_document["fingerprint"], previous["fingerprint"]);
            assert_eq!(trace_document["coordination"], previous["coordination"]);
            assert_eq!(trace_document["federates"], previous["federates"]);
            // Host runs TRACE runtime events; sensor overrides with DEBUG coordination;
            // RTI inherits the target but disables its subscriber output with OFF.
            assert!(
                documents.iter().any(|record| record["fields"]["event"]
                    == "runtime.scheduler.tag_processed"
                    && record["level"] == "TRACE"),
                "{stderr}"
            );
            assert!(
                documents
                    .iter()
                    .any(|record| record["fields"]["event"] == "coordination.reaction.started"),
                "{stderr}"
            );
            assert!(
                documents
                    .iter()
                    .filter(|record| record["kind"] == "record")
                    .all(|record| record["target"] != "boomerang::coordination"
                        || record["level"] != "TRACE"),
                "{stderr}"
            );
        }
        assert_eq!(
            losses.len(),
            3,
            "RTI and both Federates must export loss: {stderr}"
        );
        let mut retained = Vec::new();
        for loss in losses {
            retained.push(
                documents
                    .iter()
                    .filter(|record| record["kind"] == "record" && record["pid"] == loss["pid"])
                    .count(),
            );
            if variant == "bounded-small" {
                assert!(
                    loss["loss"]["overwritten_records"]["value"]
                        .as_u64()
                        .unwrap()
                        > 0,
                    "{loss}"
                );
            }
            assert_eq!(loss["loss"]["unsupported_value"]["value"], 0, "{loss}");
            assert_eq!(loss["loss"]["field_limit"]["value"], 0, "{loss}");
            assert_eq!(loss["loss"]["byte_limit"]["value"], 0, "{loss}");
            assert_eq!(
                loss["lifecycle_loss"]["producer_admission"]["value"], 0,
                "{loss}"
            );
            if variant != "bounded-filter" {
                assert!(
                    documents
                        .iter()
                        .any(|record| record["kind"] == "record" && record["pid"] == loss["pid"]),
                    "{stderr}"
                );
            }
        }
        if variant == "bounded-small" {
            retained.sort_unstable();
            assert_eq!(retained, [1, 2, 3], "{stderr}");
        }
        if variant == "bounded-filter" {
            assert_eq!(
                retained.iter().filter(|&&count| count == 0).count(),
                1,
                "only RTI is disabled: {stderr}"
            );
        }
        for record in documents.iter().filter(|line| line["kind"] == "record") {
            assert_eq!(record["trace_schema"], 1);
            if variant != "bounded-filter" {
                assert_eq!(record["target"], "boomerang::coordination");
            }
            assert!(record["fields"].get("payload").is_none());
        }
    }

    assert!(String::from_utf8_lossy(&result.stdout).contains("sensor received command 42"));
    let summary: Value = serde_json::from_slice(&fs::read(summary).unwrap()).unwrap();
    assert_eq!(summary["final_tag"]["offset_nanos"], "1000000");
    assert_eq!(summary["final_tag"]["microstep"], "0");
    let manifests = fs::read_dir(target.join("boomerang/sensor-slice"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("deployment.json"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1);
    let manifest = &manifests[0];
    let bundle = manifest.parent().unwrap();
    for role in ["rti", "federates/host", "federates/sensor"] {
        assert_trace_dependencies(
            &bundle.join(format!("generated/{role}/Cargo.toml")),
            "hosted",
        );
    }
    let document: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let host_target = target_lexicon::HOST.to_string();
    let mut federate_metadata = document["federates"].clone();
    let claims = federate_metadata
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|federate| {
            let claim = federate
                .as_object_mut()
                .unwrap()
                .remove("image_fingerprint")
                .unwrap();
            assert_eq!(claim.as_str().unwrap().len(), 64);
            claim
        })
        .collect::<Vec<_>>();
    assert_ne!(claims[0], claims[1]);
    verify_generated_wire_contract(bundle, &document, &target);
    assert_eq!(
        federate_metadata,
        serde_json::json!([
            {
                "id": "host",
                "groups": ["placement/backup", "placement/controller"],
                "target": host_target,
                "toolchain": null,
                "profile": null,
                "runtime": "std",
                "target_json_hash": null,
                "cargo_config_hash": null,
            },
            {
                "id": "sensor",
                "groups": ["placement/sensor"],
                "target": target_lexicon::HOST.to_string(),
                "toolchain": null,
                "profile": null,
                "runtime": "std",
                "target_json_hash": null,
                "cargo_config_hash": blake3::hash(
                    &fs::read(fixture_workspace().join(".cargo/sensor-slice.toml")).unwrap()
                ).to_hex().to_string(),
            }
        ])
    );

    let artifacts = document["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(artifacts[0]["federate"], "host");
    assert_eq!(artifacts[1]["federate"], "sensor");
    assert_ne!(artifacts[0]["path"], artifacts[1]["path"]);
    assert_ne!(artifacts[0]["blake3"], artifacts[1]["blake3"]);
    for artifact in artifacts {
        let path = bundle.join(artifact["path"].as_str().unwrap());
        assert_eq!(
            artifact["blake3"],
            blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
        );
    }

    let generated = document["generated"].as_array().unwrap();
    assert_eq!(generated.len(), 6);
    assert_eq!(
        generated
            .iter()
            .map(|record| record["federate"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["host", "host", "host", "sensor", "sensor", "sensor"]
    );
    let host_source =
        fs::read_to_string(bundle.join("generated/federates/host/src/main.rs")).unwrap();
    assert!(
        host_source.contains("static FEDERATE: FederateIndex = FederateIndex::new(0);"),
        "{host_source}"
    );
    assert!(!host_source.contains("FederateSliceImage"), "{host_source}");
    assert!(!host_source.contains("FederateSliceView"), "{host_source}");
    assert!(
        host_source.contains("IndexSpan::new(0, 2)"),
        "{host_source}"
    );
    assert_eq!(
        host_source
            .matches("StorageBounds::new(1, 2, 16, 1024, 512, 256)")
            .count(),
        2,
        "{host_source}"
    );
    let sensor_source =
        fs::read_to_string(bundle.join("generated/federates/sensor/src/main.rs")).unwrap();
    assert!(
        sensor_source.contains("static FEDERATE: FederateIndex = FederateIndex::new(1);"),
        "{sensor_source}"
    );
    assert!(
        !sensor_source.contains("FederateSliceImage"),
        "{sensor_source}"
    );
    assert!(
        !sensor_source.contains("FederateSliceView"),
        "{sensor_source}"
    );
    assert!(
        sensor_source.contains("IndexSpan::new(2, 1)"),
        "{sensor_source}"
    );
    assert!(
        sensor_source.contains("StorageBounds::new(1, 2, 8, 512, 256, 128)"),
        "{sensor_source}"
    );
    assert_eq!(
        document["resources"]["federates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|federate| (
                federate["id"].as_str().unwrap(),
                federate["target"].as_str().unwrap(),
                federate["runtime"].as_str().unwrap(),
            ))
            .collect::<Vec<_>>(),
        [
            ("host", host_target.as_str(), "std"),
            ("sensor", host_target.as_str(), "std")
        ]
    );

    let rti = &document["rti"];
    assert_eq!(rti["target"], host_target);
    assert_eq!(rti["profile"], Value::Null);
    assert!(rti["artifact"]["path"]
        .as_str()
        .unwrap()
        .starts_with("artifacts/rti/"));
    let rti_executable = bundle.join(rti["artifact"]["path"].as_str().unwrap());
    assert_eq!(
        rti["artifact"]["blake3"],
        blake3::hash(&fs::read(rti_executable).unwrap())
            .to_hex()
            .to_string()
    );
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(bundle.join("generated/rti/Cargo.toml"))
        .other_options(vec![String::from("--locked"), String::from("--offline")])
        .exec()
        .unwrap();
    let packages = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<Vec<_>>();
    assert!(packages.contains(&"boomerang_central_rti"));
    for forbidden in [
        "boomerang_builder",
        "vehicle-topology",
        "vehicle-control",
        "sensor-host",
    ] {
        assert!(
            !packages.contains(&forbidden),
            "RTI links {forbidden}: {packages:?}"
        );
    }
    for (federate, present, absent) in [
        ("host", "vehicle-control", "sensor-host"),
        ("sensor", "sensor-host", "vehicle-control"),
    ] {
        let metadata = MetadataCommand::new()
            .manifest_path(bundle.join(format!("generated/federates/{federate}/Cargo.toml")))
            .other_options(vec!["--locked".into(), "--offline".into()])
            .exec()
            .unwrap();
        let packages = metadata
            .packages
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>();
        assert!(packages.contains(&present), "{packages:?}");
        for forbidden in [absent, "vehicle-topology"] {
            assert!(!packages.contains(&forbidden), "{packages:?}");
        }
        assert!(packages.contains(&"boomerang_central_rti"));
        let launcher = support::with_target_directory(&target, || {
            cargo_boomerang::generate_launcher(fixture_workspace(), "sensor-slice", federate)
        })
        .unwrap();
        let built = launcher.build_locked_offline().unwrap();
        let payload_crates = support::launcher_payload_crates(built.executable_path());
        for required in ["boomerang_runtime", &present.replace('-', "_")] {
            assert!(payload_crates.contains(required), "{payload_crates:?}");
        }
        for forbidden in [
            "boomerang_builder",
            "vehicle_topology",
            &absent.replace('-', "_"),
        ] {
            assert!(!payload_crates.contains(forbidden), "{payload_crates:?}");
        }
    }
    assert!(sensor_source.contains(".bind_enclave("), "{sensor_source}");
    assert!(
        sensor_source.contains("EnclaveIndex::new(2)"),
        "{sensor_source}"
    );
    assert!(
        sensor_source.contains("boundary/controller%2Fcommand/sensor%2Fcommand/c0"),
        "{sensor_source}"
    );
    assert!(!sensor_source.contains("IdentityRange"), "{sensor_source}");
    assert!(!sensor_source.contains("IDENTITIES"), "{sensor_source}");
    assert!(
        sensor_source.contains("FederateImage::new("),
        "{sensor_source}"
    );
    assert!(
        sensor_source.contains("FederateId::new(\"sensor\")"),
        "{sensor_source}"
    );
    let sensor = bundle.join(artifacts[1]["path"].as_str().unwrap());
    let output = Command::new(sensor)
        .env_remove("BOOMERANG_RTI_ADDRESS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BOOMERANG_RTI_ADDRESS is required"),
        "{stderr}"
    );
}

#[test]
#[cfg(feature = "monitor")]
fn generated_central_deployment_feeds_monitor_json_without_changing_payload_exchange() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "sensor-telemetry");
    let _hosted = support::hosted_fixture();
    let _telemetry = support::fixture_variant("sensor-telemetry", "sensor-slice", |config| {
        config
            .as_table_mut()
            .unwrap()
            .insert("telemetry".into(), "hosted".into());
    });
    // Compile before starting the monitor's idle deadline.
    let built = build_fixture("sensor-telemetry", &target);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    let listen = probe.local_addr().unwrap();
    drop(probe);
    let mut monitor = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "monitor", "--listen", &listen.to_string()])
        // The host's initial/final rounds contribute at most eight records.
        // Ten also includes the sensor, regardless of shutdown ordering.
        .args(["--json", "--max-records", "10", "--idle-timeout", "120s"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Observe the listener binding before launching a short-lived deployment.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if monitor.try_wait().unwrap().is_some() {
            let output = monitor.wait_with_output().unwrap();
            panic!(
                "monitor exited before binding: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        match UdpSocket::bind(listen) {
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => break,
            Ok(probe) => drop(probe),
            Err(error) => panic!("failed to probe monitor listener: {error}"),
        }
        if std::time::Instant::now() >= deadline {
            monitor.kill().unwrap();
            let output = monitor.wait_with_output().unwrap();
            panic!(
                "monitor did not bind: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--workspace"])
        .arg(fixture_workspace())
        .args(["run", "--deployment", "sensor-telemetry"])
        .env("BOOMERANG_TELEMETRY_ENDPOINT", listen.to_string())
        // Keep startup skew from adding periodic rounds before the sensor starts.
        .env("BOOMERANG_TELEMETRY_PERIOD_MS", "60000")
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    let monitored = monitor.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("sensor received command 42"));
    assert!(
        monitored.status.success(),
        "{}",
        String::from_utf8_lossy(&monitored.stderr)
    );
    let snapshot: Value = serde_json::from_slice(&monitored.stdout).unwrap();
    assert_eq!(snapshot["counters"]["accepted"], 10);

    let mut run_ids = BTreeSet::new();
    let mut scheduler_sources = BTreeSet::new();
    for source in snapshot["sources"].as_array().unwrap() {
        let identity = &source["identity"];
        let run_id: [u8; 16] = serde_json::from_value(identity["run_id"].clone()).unwrap();
        run_ids.insert(run_id);
        if !source["scheduler"].is_null() {
            assert_eq!(identity["role"], "Federate");
            scheduler_sources.insert((
                identity["federate_id"].as_str().unwrap().to_owned(),
                identity["enclave_id"].as_str().unwrap().to_owned(),
            ));
        }
    }
    assert_eq!(
        run_ids.len(),
        1,
        "generated processes must share one run ID"
    );
    assert_eq!(
        scheduler_sources,
        BTreeSet::from([
            ("host".to_owned(), "backup".to_owned()),
            ("host".to_owned(), "controller".to_owned()),
            ("sensor".to_owned(), "sensor".to_owned()),
        ])
    );
}

#[test]
#[cfg(feature = "monitor")]
fn monitor_rejects_invalid_options_before_binding() {
    let occupied = UdpSocket::bind("127.0.0.1:0").unwrap();
    let listen = occupied.local_addr().unwrap().to_string();
    for options in [
        vec!["--listen", "not-a-socket"],
        vec!["--listen", &listen, "--max-records", "0"],
        vec!["--listen", &listen, "--max-records", "invalid"],
        vec!["--listen", &listen, "--idle-timeout", "invalid"],
        vec!["--listen", &listen, "--idle-timeout", "0s"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
            .args(["boomerang", "monitor"])
            .args(&options)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{options:?}: {output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
#[cfg(not(feature = "monitor"))]
fn monitor_requires_feature() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "monitor", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
}

fn assert_trace_dependencies(manifest: &std::path::Path, mode: &str) {
    let output = Command::new("cargo")
        .args(["tree", "--manifest-path"])
        .arg(manifest)
        .args([
            "--locked",
            "--offline",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    for (package, present) in [
        ("tracing-bounded", mode == "bounded"),
        ("tracing-subscriber", mode == "hosted"),
        ("tracing-appender", mode == "hosted"),
    ] {
        assert_eq!(
            tree.lines()
                .any(|line| line.starts_with(&format!("{package} v"))),
            present,
            "{mode}: {tree}"
        );
    }
}

/// Executes the portable codec/admission contract compiled with the actual generated RTI tables.
fn verify_generated_wire_contract(bundle: &Path, document: &Value, target: &Path) {
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir(scratch.path().join("src")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock", "src/main.rs"] {
        fs::copy(
            bundle.join("generated/rti").join(file),
            scratch.path().join(file),
        )
        .unwrap();
    }
    let source_path = scratch.path().join("src/main.rs");
    let source = fs::read_to_string(&source_path).unwrap();
    fs::write(
        &source_path,
        format!("{source}\n{}", include_str!("wire_contract.rs")),
    )
    .unwrap();
    let result = Command::new("cargo")
        .args(["test", "--locked", "--offline", "--manifest-path"])
        .arg(scratch.path().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(target.join("wire-contract"))
        .current_dir(fixture_workspace())
        .env(
            "EXPECTED_COORDINATION",
            document["coordination"]["identity"].as_str().unwrap(),
        )
        .env(
            "EXPECTED_MAPPING",
            document["coordination"]["wire"]["mapping"]
                .as_str()
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn build_normalizes_deployment_execution_policy_into_every_published_artifact() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "execution");
    support::reset_deployment_output(&target, "execution-equivalent");
    let _manifest = support::fixture_variant("execution", "production", |deployment| {
        deployment.as_table_mut().unwrap().insert(
            "execution".into(),
            toml::toml! {
                fast-forward = true
                keep-alive = true
                logical-horizon = "1000ms"
            }
            .into(),
        );
    });
    let _equivalent = support::fixture_variant("execution-equivalent", "execution", |deployment| {
        deployment["execution"]
            .as_table_mut()
            .unwrap()
            .insert("logical-horizon".into(), "1s".into());
    });
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
    assert!(source.contains("physical_event_q_size: 1024"), "{source}");
    assert!(
        source.contains("boomerang_util::launcher::write_execution_summary(&execution)?"),
        "{source}"
    );
    let generated_manifest: toml::Value = toml::from_str(
        &fs::read_to_string(manifest.parent().unwrap().join("generated/host/Cargo.toml")).unwrap(),
    )
    .unwrap();
    let dependencies = generated_manifest["dependencies"].as_table().unwrap();
    assert_eq!(
        dependencies["boomerang_util"]["features"]
            .as_array()
            .unwrap(),
        &[toml::Value::String("hosted-tracing".into())]
    );
    assert!(!dependencies.contains_key("tracing-subscriber"));
    let executable = published_executable(manifest.to_str().unwrap());
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn build_applies_configured_release_profile_and_cargo_configuration() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    support::reset_deployment_output(&target, "profile-config");
    let _manifest = support::fixture_variant("profile-config", "production", |deployment| {
        deployment["bindings"]["controller"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["profile-config-probe"]).unwrap(),
            );
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("profile".into(), "release".into());
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("cargo-config".into(), ".cargo/profile-config.toml".into());
    });
    let result = build_fixture("profile-config", &target);

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
    assert!(manifest.exists(), "missing {}", manifest.display());
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(fixture_workspace(), "profile-config", "host")
    })
    .unwrap();
    launcher.check_locked_offline().unwrap();
    launcher.run_locked_offline().unwrap();
}
