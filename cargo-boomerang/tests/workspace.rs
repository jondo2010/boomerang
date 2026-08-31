use std::{fs, path::Path, path::PathBuf};

use cargo_boomerang::resolve_workspace;
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
    let resolved = resolve_workspace(&workspace, "resolution").unwrap();

    assert_eq!(resolved.deployment_name(), "resolution");
    assert_eq!(resolved.topology().package, "vehicle-topology");
    assert_eq!(resolved.topology().entry, "vehicle_topology::topology");
    let topology = resolved.package("vehicle-topology").unwrap();
    assert_eq!(
        topology.manifest_path,
        workspace.join("vehicle-topology/Cargo.toml")
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
        host.target_json.as_deref(),
        Some(workspace.join("targets/host.json").as_path())
    );
    assert_eq!(
        host.cargo_config.as_deref(),
        Some(workspace.join(".cargo/host.toml").as_path())
    );

    assert_eq!(
        resolved.lockfile().path,
        fs::canonicalize(workspace.join("Cargo.lock")).unwrap()
    );
    assert_eq!(
        resolved.lockfile().digest,
        [
            0x2b, 0x71, 0x15, 0x58, 0xa0, 0x6e, 0xca, 0xe0, 0xf5, 0xea, 0x37, 0x0b, 0x85, 0xb1,
            0xbc, 0x0d, 0x59, 0x61, 0x2f, 0x43, 0x64, 0x6d, 0xa3, 0x67, 0xfb, 0x01, 0xa0, 0xd6,
            0xea, 0x22, 0x72, 0x90,
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
