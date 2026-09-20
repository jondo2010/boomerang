use std::path::PathBuf;

use super::support;

fn fixture_workspace() -> PathBuf {
    support::fixture_workspace()
}

#[test]
fn generated_manifest_unions_features_for_one_selected_package() {
    let _guard = support::toolchain_lock();
    let target = support::toolchain_target();
    let _manifest = support::fixture_variant("feature-union", "production", |deployment| {
        deployment["bindings"]["controller"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["controller-selected"]).unwrap(),
            );
        deployment["bindings"]["backup"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                toml::Value::try_from(["sensor-selected"]).unwrap(),
            );
    });
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
        r#"["controller-selected", "sensor-selected"]"#
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
    let source = std::fs::read_to_string(launcher.source_path()).unwrap();
    assert!(
        source.contains("EnclaveImageView::new(&E0_IMAGE)"),
        "{source}"
    );
    assert!(
        source.contains("invalid generated Enclave image E0"),
        "{source}"
    );
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
    let payload_crates = support::launcher_payload_crates(first.executable_path());
    for present in ["boomerang_runtime", "sensor_host", "vehicle_control"] {
        assert!(payload_crates.contains(present), "{payload_crates:?}");
    }
    assert!(
        !payload_crates.contains("boomerang_builder"),
        "{payload_crates:?}"
    );
}

#[test]
fn generated_launcher_rejects_changed_configured_files_before_cargo() {
    let _guard = support::toolchain_lock();
    let _manifest = support::fixture_variant("resolution", "production", |deployment| {
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("target-json".into(), "targets/host.json".into());
        deployment["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert("cargo-config".into(), ".cargo/host.toml".into());
    });
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

/// Rejects unsupported coordination selections before creating any launcher workspace.
#[test]
fn generated_launcher_rejects_unsupported_coordination_before_publication() {
    let _guard = support::toolchain_lock();
    let workspace = support::copied_fixture_workspace();
    let target = tempfile::tempdir().unwrap();
    let manifest = workspace.path().join("Boomerang.toml");
    let original = std::fs::read_to_string(&manifest)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    for (selection, expected) in [
        (
            "backend",
            "distributed coordination projection is not implemented",
        ),
        (
            "transport",
            "unsupported generated central-rti boundary configuration",
        ),
        (
            "codec",
            "unsupported generated central-rti boundary configuration",
        ),
    ] {
        let mut changed = original.clone();
        let deployment = changed["deployments"]["sensor-slice"]
            .as_table_mut()
            .unwrap();
        if selection == "backend" {
            deployment["coordination"]
                .as_table_mut()
                .unwrap()
                .insert("backend".into(), "peer-to-peer".into());
            deployment.remove("rti");
        } else {
            deployment["boundaries"]["boundary/controller%2Fcommand/sensor%2Fcommand/c0"]
                [selection] = if selection == "transport" {
                "udp"
            } else {
                "postcard"
            }
            .into();
        }
        std::fs::write(&manifest, toml::to_string(&changed).unwrap()).unwrap();
        let result = support::with_target_directory(target.path(), || {
            cargo_boomerang::generate_launcher(workspace.path(), "sensor-slice", "host")
        });
        let error = result
            .err()
            .expect("unsupported coordination must be rejected");
        assert!(
            format!("{error:#}").contains(expected),
            "{selection}: {error:#}"
        );
        assert!(
            !target
                .path()
                .join("boomerang/generated/v1/launcher")
                .exists(),
            "{selection} wrote a generated launcher before validating coordination"
        );
    }
}
