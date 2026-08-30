use std::{fs, path::Path, path::PathBuf};

use cargo_boomerang::{resolve_workspace, WorkspaceError};
use cargo_metadata::MetadataCommand;

fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
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
    let workspace = fixture_workspace();
    let mut metadata = MetadataCommand::new();
    metadata
        .current_dir(&workspace)
        .manifest_path(workspace.join("Cargo.toml"))
        .other_options(vec![String::from("--locked")]);
    let metadata = metadata.exec().unwrap();
    let resolved = resolve_workspace(&workspace, "production").unwrap();

    assert_eq!(resolved.topology.name, "vehicle-topology");
    assert_eq!(resolved.topology.entry, "vehicle_topology::topology");
    assert_eq!(
        resolved.topology.manifest_path,
        workspace.join("vehicle-topology/Cargo.toml")
    );
    assert_eq!(
        resolved.topology.id,
        metadata
            .packages
            .iter()
            .find(|package| package.name == "vehicle-topology")
            .unwrap()
            .id
    );

    let sensor = &resolved.bindings["vehicle/sensor"];
    assert_eq!(sensor.name, "sensor-host");
    assert_eq!(
        sensor.id,
        metadata
            .packages
            .iter()
            .find(|package| package.name == "sensor-host")
            .unwrap()
            .id
    );
    assert_eq!(sensor.features, ["simulated"]);
    assert_eq!(sensor.facets.descriptor, "__boomerang_descriptor");
    assert_eq!(sensor.facets.payload, "__boomerang_payload");

    let host = &resolved.federates["host"];
    assert_eq!(host.target.as_deref(), Some("x86_64-unknown-linux-gnu"));
    assert_eq!(
        host.target_json.as_deref(),
        Some(workspace.join("targets/host.json").as_path())
    );
    assert_eq!(
        host.cargo_config.as_deref(),
        Some(workspace.join(".cargo/host.toml").as_path())
    );

    assert_eq!(
        resolved.lockfile.path,
        fs::canonicalize(workspace.join("Cargo.lock")).unwrap()
    );
    assert_eq!(
        resolved.lockfile.digest,
        [
            0x22, 0x0a, 0x2c, 0x31, 0xf0, 0xbd, 0xdf, 0xe3, 0x55, 0x90, 0xc7, 0xdc, 0x60, 0x87,
            0xa1, 0xcb, 0x5f, 0xa2, 0x97, 0x41, 0xa3, 0x9c, 0xa0, 0xab, 0xdf, 0x7d, 0x46, 0x72,
            0x36, 0xba, 0x31, 0x61,
        ]
    );

    let error = resolve_workspace(&workspace, "outside-member").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("deployment 'outside-member'"), "{error}");
    assert!(message.contains("binding 'vehicle/sensor'"), "{error}");
    assert!(
        message.contains("package 'outside-member' must be a member of the application workspace"),
        "{error}"
    );

    let unlocked = tempfile::tempdir().unwrap();
    copy_without_lockfile(&workspace, unlocked.path());
    let error = resolve_workspace(unlocked.path(), "production").unwrap_err();
    assert!(matches!(error, WorkspaceError::Metadata { .. }));
    assert!(!unlocked.path().join("Cargo.lock").exists());
}
