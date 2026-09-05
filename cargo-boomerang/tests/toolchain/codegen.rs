use cargo_metadata::MetadataCommand;
use std::path::PathBuf;

use super::support;

fn fixture_workspace() -> PathBuf {
    support::fixture_workspace()
}

#[test]
fn generated_manifest_unions_features_for_one_selected_package() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(fixture_workspace(), "feature-union", "host")
    })
    .unwrap();
    launcher.build_locked_offline().unwrap();
    let manifest = std::fs::read_to_string(launcher.manifest_path()).unwrap();
    let manifest = manifest.parse::<toml::Table>().unwrap();
    let features = &manifest["dependencies"]["implementation_0"]["features"];
    assert_eq!(
        features.to_string(),
        r#"["__boomerang_payload", "controller-selected", "sensor-selected"]"#
    );
}

#[test]
fn generated_single_federate_launcher_executes_typed_local_route_without_builder() {
    let _guard = support::toolchain_lock();
    let target = tempfile::tempdir().unwrap();
    let workspace = fixture_workspace();
    let (resolved, launcher) = support::with_target_directory(target.path(), || {
        (
            cargo_boomerang::resolve_workspace(&workspace, "production").unwrap(),
            cargo_boomerang::generate_launcher(&workspace, "production", "host").unwrap(),
        )
    });
    let first = launcher.build_locked_offline().unwrap();
    assert!(first.compiled_artifacts() > 0);
    let first_executable = std::fs::read(first.executable_path()).unwrap();
    let second = launcher.build_locked_offline().unwrap();
    assert_eq!(second.compiled_artifacts(), 0);
    assert_ne!(second.executable_path(), first.executable_path());
    assert_eq!(
        blake3::hash(&std::fs::read(second.executable_path()).unwrap()),
        blake3::hash(&first_executable)
    );

    let mut relative = first
        .executable_path()
        .strip_prefix(std::fs::canonicalize(resolved.target_directory()).unwrap())
        .unwrap()
        .components();
    assert_eq!(relative.next().unwrap().as_os_str(), "b");
    let locator = relative.next().unwrap().as_os_str().to_str().unwrap();
    assert_eq!(locator.len(), 33);
    assert!(locator.starts_with('l'));
    assert!(locator[1..]
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    launcher.run_locked_offline().unwrap();
    launcher.check_locked_offline().unwrap();

    let metadata = MetadataCommand::new()
        .manifest_path(launcher.manifest_path())
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
}

#[test]
fn generated_launcher_check_and_run_apply_federate_cargo_configuration() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(fixture_workspace(), "profile-config", "host")
    })
    .unwrap();
    let check = launcher.check_locked_offline();
    let run = launcher.run_locked_offline();
    assert!(check.is_ok(), "check failed: {check:#?}");
    assert!(run.is_ok(), "run failed: {run:#?}");
}

#[test]
fn generated_launcher_rejects_changed_configured_files_before_cargo() {
    let _guard = support::toolchain_lock();
    let fixture = support::copied_fixture_workspace();
    let target = tempfile::tempdir().unwrap();
    let launcher = support::with_target_directory(target.path(), || {
        cargo_boomerang::generate_launcher(fixture.path(), "resolution", "host")
    })
    .unwrap();

    let target_json = fixture.path().join("targets/host.json");
    let original_target_json = std::fs::read(&target_json).unwrap();
    std::fs::write(&target_json, b"changed target JSON").unwrap();
    let error = launcher.check_locked_offline().unwrap_err().to_string();
    assert!(error.contains("configured target JSON changed"), "{error}");

    std::fs::write(&target_json, original_target_json).unwrap();
    let cargo_config = fixture.path().join(".cargo/host.toml");
    std::fs::write(&cargo_config, b"changed Cargo configuration").unwrap();
    let error = launcher.check_locked_offline().unwrap_err().to_string();
    assert!(
        error.contains("configured Cargo configuration changed"),
        "{error}"
    );
}

#[test]
fn generated_launcher_renders_normalized_deployment_execution_policy() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(fixture_workspace(), "execution", "host")
    })
    .unwrap();
    launcher.build_locked_offline().unwrap();
    let source = std::fs::read_to_string(launcher.source_path()).unwrap();
    assert!(source.contains("fast_forward: true"), "{source}");
    assert!(source.contains("keep_alive: true"), "{source}");
    assert!(
        source.contains("timeout: Some(boomerang_runtime::Duration::nanoseconds_i128(1000000000))"),
        "{source}"
    );
    assert!(source.contains("physical_event_q_size: 1024"), "{source}");
    assert!(
        source.contains("BOOMERANG_EXECUTION_SUMMARY_V1"),
        "{source}"
    );
    assert!(source.contains("create_new(true)"), "{source}");
    assert!(source.contains("execution.stats()"), "{source}");
    assert!(source.contains("execution.final_tag()"), "{source}");
}
