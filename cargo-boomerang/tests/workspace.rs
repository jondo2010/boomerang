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
        host.target_json.as_deref(),
        Some(
            fs::canonicalize(workspace.join("targets/host.json"))
                .unwrap()
                .as_path()
        )
    );
    assert_eq!(
        host.cargo_config.as_deref(),
        Some(
            fs::canonicalize(workspace.join(".cargo/host.toml"))
                .unwrap()
                .as_path()
        )
    );

    assert_eq!(
        resolved.lockfile().path,
        fs::canonicalize(workspace.join("Cargo.lock")).unwrap()
    );
    assert_eq!(
        resolved.lockfile().digest,
        [
            0x6c, 0x35, 0xdf, 0xd6, 0x84, 0x92, 0x0c, 0x25, 0x3f, 0x27, 0xd0, 0x7d, 0x68, 0x11,
            0xc6, 0x61, 0xaf, 0xea, 0xfe, 0x02, 0x33, 0x17, 0x69, 0xa2, 0xc2, 0x2f, 0xf7, 0x36,
            0xdf, 0xff, 0xc9, 0xea,
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
