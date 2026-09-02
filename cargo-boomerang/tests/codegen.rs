use cargo_metadata::MetadataCommand;
use std::path::PathBuf;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

#[test]
fn generated_manifest_unions_features_for_one_selected_package() {
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
    let launcher =
        cargo_boomerang::generate_launcher(fixture_workspace(), "production", "host").unwrap();
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
