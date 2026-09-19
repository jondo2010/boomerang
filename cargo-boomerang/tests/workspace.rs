use std::{fs, path::Path, path::PathBuf};

use cargo_boomerang::resolve_workspace;
use cargo_metadata::MetadataCommand;

mod support;

fn fixture_workspace() -> PathBuf {
    support::fixture_workspace()
}

fn copy_without_lockfile(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "Cargo.lock" {
            continue;
        }
        let destination = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_without_lockfile(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
fn resolution_returns_exact_package_ids_and_rejects_nonmembers() {
    let _manifest = support::fixture_variant("resolution", "production", |deployment| {
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("target".into(), "x86_64-unknown-linux-gnu".into());
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("target-json".into(), "targets/host.json".into());
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("cargo-config".into(), ".cargo/host.toml".into());
    });
    let _outside = support::fixture_variant("outside-member", "production", |deployment| {
        deployment.as_table_mut().unwrap().insert(
            "bindings".into(),
            toml::toml! { "vehicle/sensor" = { package = "outside-member" } }.into(),
        );
    });
    let workspace = fixture_workspace();
    let mut metadata = MetadataCommand::new();
    metadata
        .current_dir(&workspace)
        .manifest_path(workspace.join("Cargo.toml"))
        .other_options(vec![String::from("--locked")]);
    let metadata = metadata.exec().unwrap();
    let resolved = resolve_workspace(&workspace, "resolution").unwrap();

    assert_eq!(resolved.deployment_name(), "resolution");
    assert_eq!(resolved.topology().package, "vehicle-topology");
    assert_eq!(resolved.topology().entry, "vehicle_topology::topology");
    let topology = resolved.package("vehicle-topology").unwrap();
    // Cargo and std may spell the same Windows path with different verbatim prefixes.
    assert_eq!(
        fs::canonicalize(&topology.manifest_path).unwrap(),
        fs::canonicalize(workspace.join("vehicle-topology/Cargo.toml")).unwrap()
    );
    assert_eq!(
        topology.id,
        metadata
            .packages
            .iter()
            .find(|package| package.name == "vehicle-topology")
            .unwrap()
            .id
    );

    let sensor_binding = &resolved.deployment().bindings["sensor"];
    assert_eq!(sensor_binding.package, "sensor-host");
    assert_eq!(sensor_binding.features, ["simulated"]);
    let sensor = resolved.package("sensor-host").unwrap();
    assert_eq!(
        sensor.id,
        metadata
            .packages
            .iter()
            .find(|package| package.name == "sensor-host")
            .unwrap()
            .id
    );

    let host = &resolved.deployment().federates["host"];
    assert_eq!(host.target.as_deref(), Some("x86_64-unknown-linux-gnu"));
    assert_eq!(
        fs::canonicalize(host.target_json.as_ref().unwrap()).unwrap(),
        fs::canonicalize(workspace.join("targets/host.json")).unwrap()
    );
    assert_eq!(
        fs::canonicalize(host.cargo_config.as_ref().unwrap()).unwrap(),
        fs::canonicalize(workspace.join(".cargo/host.toml")).unwrap()
    );

    assert_eq!(
        resolved.lockfile().path,
        fs::canonicalize(workspace.join("Cargo.lock")).unwrap()
    );
    assert_eq!(
        resolved.lockfile().digest,
        [
            0x5d, 0xeb, 0xd8, 0xe1, 0xe7, 0x4a, 0x71, 0x7d, 0x12, 0x1e, 0x9b, 0x03, 0x20, 0x71,
            0xe2, 0x73, 0xb0, 0xbe, 0xa6, 0x31, 0x4f, 0xe5, 0x88, 0xae, 0x4a, 0xd3, 0xfb, 0xdb,
            0x03, 0x14, 0x43, 0xae,
        ]
    );

    let error = resolve_workspace(&workspace, "outside-member").unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("deployment 'outside-member'"), "{error}");
    assert!(message.contains("binding 'vehicle/sensor'"), "{error}");
    assert!(
        message.contains("package 'outside-member' must be a member of the application workspace"),
        "{error}"
    );

    let unlocked = tempfile::tempdir().unwrap();
    copy_without_lockfile(&workspace, unlocked.path());
    let error = resolve_workspace(unlocked.path(), "resolution").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("failed to resolve locked Cargo metadata"),
        "{error:#}"
    );
    assert!(!unlocked.path().join("Cargo.lock").exists());
}
