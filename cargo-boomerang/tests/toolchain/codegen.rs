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
    let launcher = support::with_target_directory(target.path(), || {
        cargo_boomerang::generate_launcher(&workspace, "production", "host").unwrap()
    });
    let source = std::fs::read_to_string(launcher.source_path()).unwrap();
    let generated = syn::parse_file(&source).unwrap();
    let static_item = |name: &str| {
        generated
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Static(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("generated {name} must be a typed static"))
    };
    let images = static_item("ENCLAVE_IMAGES");
    assert_eq!(
        quote::quote!(#images).to_string(),
        quote::quote! {
            static ENCLAVE_IMAGES: [EnclaveImage<'static>; 3] = [E0_IMAGE, E1_IMAGE, E2_IMAGE];
        }
        .to_string(),
    );
    assert!(!source.contains("E0_VIEW"), "{source}");
    assert!(!source.contains("ENCLAVE_VIEWS"), "{source}");
    let federate_view = static_item("FEDERATE_VIEW");
    assert_eq!(
        quote::quote!(#federate_view).to_string(),
        quote::quote! {
            static FEDERATE_VIEW: FederateImageView<'static> =
                match FederateImageView::new(FEDERATE_IMAGE, &ENCLAVE_IMAGES,) {
                    Ok(view) => view,
                    Err(_) => panic!("invalid generated Federate image"),
                };
        }
        .to_string(),
    );
    assert!(
        source.contains("execute_owned_federate_slice_with_observations("),
        "{source}"
    );
    assert!(source.contains("&FEDERATE_VIEW"), "{source}");
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

#[test]
fn generated_external_clock_launcher_calls_selected_driver_and_executes() {
    generated_clock_fixture(false);
}
#[test]
fn generated_physical_inputs_resolve_declared_targets_and_execute() {
    generated_clock_fixture(true);
}
fn generated_clock_fixture(with_inputs: bool) {
    let _guard = support::toolchain_lock();
    let workspace = support::copied_fixture_workspace();
    let target = support::toolchain_target();
    let _manifest = support::edit_manifest(workspace.path(), |manifest| {
        manifest["deployments"]["production"]["federates"]["host"]
            .as_table_mut()
            .unwrap()
            .insert(
                "physical-clock".into(),
                toml::Value::try_from(
                    if with_inputs {
                        serde_json::json!({"domain": 7, "binding": "sensor", "entry": "drive_clock", "inputs": {"max_batch_values": 2, "max_staged_batches": 2, "sources": {"plant": {"required": false, "targets": {"sample": {"enclave": "sensor", "binding": "action/sensor/external"}}}}}})
                    } else { serde_json::json!({"domain": 7, "binding": "sensor", "entry": "drive_clock"}) },
                )
                .unwrap(),
            );
    });
    let path = workspace.path().join("sensor-host/Cargo.toml");
    let mut manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../boomerang_runtime");
    manifest["dependencies"].as_table_mut().unwrap().insert(
        "boomerang_runtime".into(),
        toml::Value::try_from(serde_json::json!({"path": runtime, "features": ["external-clock"]}))
            .unwrap(),
    );
    std::fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    let path = workspace.path().join("sensor-host/src/lib.rs");
    let mut source = std::fs::read_to_string(&path).unwrap();
    if with_inputs {
        source = source.replace("Sensor(#[input] command: u32)", "Sensor(#[input] command: u32, #[physical_action] external: u32)").replace("            reaction! {", "            reaction! { observe (external) { assert_eq!(ctx.get_action_value(&mut external), Some(&99)); } }\n            reaction! {");
        source.push_str(r#"
#[cfg(boomerang_facet = "payload")]
pub fn drive_clock(clock: boomerang_runtime::physical_clock::ManualClock, inputs: boomerang_runtime::physical_input::InputAdmission) -> Result<(), boomerang_runtime::physical_input::InputError> {
    use boomerang_runtime::{physical_input::*, physical_time::*};
    let source = inputs.source("plant").unwrap();
    let target = inputs.target(source, "sample").unwrap();
    inputs.submit(vec![InputObservation { source, sequence: 1, acquired: PhysicalTimeNanos(0), domain: clock.domain(), epoch: clock.epoch(), values: vec![InputValue::new(target, 99u32)] }], &[])?;
    clock.advance_to(PhysicalTimeNanos(100)).unwrap();
    Ok(())
}
"#);
    } else {
        source.push_str(r#"
#[cfg(boomerang_facet = "payload")]
pub fn drive_clock(clock: boomerang_runtime::physical_clock::ManualClock) -> Result<(), boomerang_runtime::physical_time::PhysicalClockError> {
    assert_eq!(clock.domain(), boomerang_runtime::physical_time::PhysicalClockDomainId(7));
    clock.advance_to(boomerang_runtime::physical_time::PhysicalTimeNanos(100))
}
"#);
    }
    std::fs::write(path, source).unwrap();
    assert!(std::process::Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(workspace.path())
        .status()
        .unwrap()
        .success());
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(workspace.path(), "production", "host").unwrap()
    });
    let source = std::fs::read_to_string(launcher.source_path()).unwrap();
    assert!(
        source.contains("::drive_clock(") && source.contains("physical_clock.clone()"),
        "{source}"
    );
    assert!(std::fs::read_to_string(launcher.manifest_path())
        .unwrap()
        .contains("external-clock"));
    launcher.run_locked_offline().unwrap();
}
