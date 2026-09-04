use cargo_metadata::MetadataCommand;
use std::path::PathBuf;

mod support;

fn fixture_workspace() -> PathBuf {
    support::fixture_workspace()
}

#[test]
fn generated_manifest_unions_features_for_one_selected_package() {
    support::with_target_directory(
        &support::shared_target("launcher"),
        generated_manifest_unions_features_for_one_selected_package_inner,
    );
}

fn generated_manifest_unions_features_for_one_selected_package_inner() {
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "feature-union", "host").unwrap();
    let manifest = std::fs::read_to_string(launcher.manifest_path()).unwrap();
    let manifest = manifest.parse::<toml::Table>().unwrap();
    let features = &manifest["dependencies"]["implementation_0"]["features"];
    assert_eq!(
        features.to_string(),
        r#"["__boomerang_payload", "controller-selected", "sensor-selected"]"#
    );
    launcher.check_locked_offline().unwrap();
}

#[test]
fn generated_single_federate_launcher_executes_typed_local_route_without_builder() {
    let target = tempfile::tempdir().unwrap();
    support::with_target_directory(
        target.path(),
        generated_single_federate_launcher_executes_typed_local_route_without_builder_inner,
    );
}

fn generated_single_federate_launcher_executes_typed_local_route_without_builder_inner() {
    let workspace = fixture_workspace();
    let resolved = cargo_boomerang::resolve_workspace(&workspace, "production").unwrap();
    let launcher = cargo_boomerang::generate_launcher(&workspace, "production", "host").unwrap();
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
    support::with_target_directory(&support::shared_target("launcher"), || {
        let launcher =
            cargo_boomerang::generate_launcher(fixture_workspace(), "profile-config", "host")
                .unwrap();
        let check = launcher.check_locked_offline();
        let run = launcher.run_locked_offline();

        assert!(check.is_ok(), "check failed: {check:#?}");
        assert!(run.is_ok(), "run failed: {run:#?}");
    });
}

#[test]
fn generated_launcher_renders_normalized_deployment_execution_policy() {
    support::with_target_directory(
        &support::shared_target("launcher"),
        generated_launcher_renders_normalized_deployment_execution_policy_inner,
    );
}

fn generated_launcher_renders_normalized_deployment_execution_policy_inner() {
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "execution", "host").unwrap();
    let source = std::fs::read_to_string(launcher.source_path()).unwrap();
    assert!(source.contains("fast_forward: true"), "{source}");
    assert!(source.contains("keep_alive: true"), "{source}");
    assert!(
        source.contains("timeout: Some(boomerang_runtime::Duration::nanoseconds_i128(1000000000))"),
        "{source}"
    );
    assert!(source.contains("physical_event_q_size: 1024"), "{source}");
}

#[test]
fn generated_launcher_build_preserves_json_compiler_diagnostics() {
    let target = tempfile::tempdir().unwrap();
    support::with_target_directory(
        target.path(),
        generated_launcher_build_preserves_json_compiler_diagnostics_inner,
    );
}

fn generated_launcher_build_preserves_json_compiler_diagnostics_inner() {
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "broken-payload", "host").unwrap();

    let error = launcher
        .build_locked_offline()
        .err()
        .expect("broken payload launcher build must fail");
    assert!(
        error
            .to_string()
            .contains("intentional target payload build failure"),
        "expected rendered compiler diagnostic, got:\n{error:#}"
    );
}
