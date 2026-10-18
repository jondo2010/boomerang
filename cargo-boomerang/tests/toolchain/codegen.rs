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

/// Builds and runs a generated launcher with its explicitly selected clock driver.
#[test]
fn generated_external_clock_launcher_calls_selected_driver_and_executes() {
    generated_clock_fixture(false, true);
}
/// Checks generated input declarations resolve and deliver an admitted value.
#[test]
fn generated_physical_inputs_resolve_declared_targets_and_execute() {
    generated_clock_fixture(true, true);
}
/// Native physical input does not require manual clock authority.
#[test]
fn generated_native_inputs_initialize_before_startup() {
    generated_clock_fixture(true, false);
}
/// Source-free actions use ordinary asynchronous sending through the same initializer.
#[test]
fn generated_source_free_initializer_delivers_an_ordinary_async_action() {
    generated_clock_fixture(false, false);
}
/// Builds and executes capability-aware component initialization in the generated launcher.
fn generated_clock_fixture(with_inputs: bool, manual: bool) {
    let _guard = support::toolchain_lock();
    let workspace = support::copied_fixture_workspace();
    let target = support::toolchain_target();
    let _manifest = support::edit_manifest(workspace.path(), |manifest| {
        let federate = manifest["deployments"]["production"]["federates"]["host"]
            .as_table_mut()
            .unwrap();
        if manual {
            federate.insert(
                "physical-clock".into(),
                toml::Value::try_from(serde_json::json!({"domain": 7})).unwrap(),
            );
        }
        if with_inputs {
            federate.insert("physical-inputs".into(), toml::Value::try_from(serde_json::json!({"max_batch_values": 2, "max_staged_batches": 2, "sources": {"plant": {"component": "sensor", "required": true}}})).unwrap());
        }
    });
    let path = workspace.path().join("sensor-host/Cargo.toml");
    let mut manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../boomerang_runtime");
    let expected_features = match (with_inputs, manual) {
        (false, false) => vec![],
        (false, true) => vec!["hosted-manual-clock"],
        (true, false) => vec!["hosted-source-input"],
        (true, true) => vec!["hosted-manual-clock", "hosted-source-input"],
    };
    manifest["dependencies"].as_table_mut().unwrap().insert(
        "boomerang_runtime".into(),
        toml::Value::try_from(serde_json::json!({"path": runtime, "features": expected_features}))
            .unwrap(),
    );
    std::fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    let path = workspace.path().join("sensor-host/src/lib.rs");
    let mut source = std::fs::read_to_string(&path).unwrap();
    let declarations = match (with_inputs, manual) {
        (true, true) => "source = \"plant\", actions = [external], clock_control = true",
        (true, false) => "source = \"plant\", actions = [external]",
        (false, true) => "clock_control = true",
        (false, false) => "actions = [external]",
    };
    source = source.replace("contract = \"vehicle.sensor\",", &format!("state = DriverState, state_init = DriverState::new, init_context(name = DriverInit, {declarations}), contract = \"vehicle.sensor\","));
    let with_action = with_inputs || !manual;
    let submit = if with_inputs {
        r#"
        use boomerang_runtime::{input::*, clock::*};
        let clock = init.source.clock();
        init.source.submit(InputBatch::new(vec![InputObservation { source: init.source.clone(), sequence: 1, acquired: PhysicalInstant(0), domain: clock.domain(), epoch: clock.epoch(), values: vec![InputValue::new(&init.external, 99u32)] }]).with_progress(vec![(init.source.clone(), PhysicalInstant(101))]))?;
"#
    } else if !manual {
        r#"
        init.send.prepare_action(&init.external, 99u32, None)?.try_send()?;
"#
    } else {
        ""
    };
    let drop_check = if with_action {
        "impl Drop for DriverState { fn drop(&mut self) { assert!(self.seen, \"physical sample was not delivered\"); } }"
    } else {
        ""
    };
    let advance = if manual {
        "init.clock.advance_to(boomerang_runtime::clock::PhysicalInstant(100))?;"
    } else {
        ""
    };
    source = source.replace(
        "use boomerang::prelude::*;",
        &format!(
            r#"
        use boomerang::prelude::*;
        #[derive(Debug)]
        pub struct DriverState {{ seen: bool }}
        {drop_check}
        impl DriverState {{
            fn new(init: DriverInit) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {{
                {submit}
                {advance}
                Ok(Self {{ seen: false }})
            }}
        }}
"#
        ),
    );
    if with_action {
        source = source.replace("Sensor(#[input] command: u32)", "Sensor(#[input] command: u32, #[physical_action] external: u32)").replace("            reaction! {", "            reaction! { observe (external) { assert_eq!(ctx.get_action_value(&mut external), Some(&99)); state.seen = true; } }\n            reaction! {");
    }
    if !with_inputs && !manual {
        source = source.replace("ctx.schedule_shutdown(None);", "").replace(
            "state.seen = true;",
            "state.seen = true; ctx.schedule_shutdown(None);",
        );
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
    let metadata = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(launcher.manifest_path())
        .output()
        .unwrap();
    assert!(
        metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let graph: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
    let runtime = graph["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "boomerang_runtime")
        .unwrap();
    let resolved = graph["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == runtime["id"])
        .unwrap();
    let hosted_features = resolved["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| feature.as_str().unwrap())
        .filter(|feature| feature.starts_with("hosted-") || *feature == "external-clock")
        .collect::<Vec<_>>();
    assert_eq!(
        hosted_features, expected_features,
        "manual={manual}, inputs={with_inputs}: {resolved}"
    );
    eprintln!(
        "manual={manual}, inputs={with_inputs} resolved runtime features: {}",
        resolved["features"]
    );
    launcher.run_locked_offline().unwrap();
}

/// Rejects every invalid action declaration before generated launcher publication.
#[test]
fn physical_input_invalid_action_declarations_are_rejected_before_launcher_publication() {
    let _guard = support::toolchain_lock();
    let workspace = support::copied_fixture_workspace();
    let target = support::toolchain_target();
    let path = workspace.path().join("sensor-host/src/lib.rs");
    let source = std::fs::read_to_string(&path).unwrap().replace("Sensor(#[input] command: u32)", "Sensor(#[input] command: u32, #[physical_action] external: u32, #[logical_action] internal: u32)");
    std::fs::write(path, source).unwrap();
    for (component, source_id, expected) in [
        ("sensor", "plant", "declared component owner"),
        ("controller", "plant", "declared component owner"),
    ] {
        let _manifest = support::edit_manifest(workspace.path(), |manifest| {
            manifest["deployments"]["production"]["federates"]["host"].as_table_mut().unwrap().insert("physical-inputs".into(), toml::Value::try_from(serde_json::json!({"max_batch_values": 2, "max_staged_batches": 2, "sources": {source_id: {"component": component, "required": true}}})).unwrap());
        });
        let error = support::with_target_directory(&target, || {
            cargo_boomerang::generate_launcher(workspace.path(), "production", "host")
        })
        .err()
        .expect("undeclared source must fail before publication");
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

/// Two instances of one source component submit a batch using host monotonic time.
#[test]
fn generated_native_reused_components_submit_one_two_enclave_batch() {
    let _guard = support::toolchain_lock();
    let workspace = support::copied_fixture_workspace();
    let target = support::toolchain_target();
    let _manifest = support::edit_manifest(workspace.path(), |manifest| {
        let deployment = &mut manifest["deployments"]["production"];
        deployment["bindings"] = toml::Value::try_from(serde_json::json!({
            "left": {"package": "sensor-host", "component": "sensor"},
            "right": {"package": "sensor-host", "component": "sensor"}
        }))
        .unwrap();
        deployment["federates"]["host"]["groups"] =
            toml::Value::try_from(["placement/left", "placement/right"]).unwrap();
        deployment["federates"]["host"].as_table_mut().unwrap().insert("physical-inputs".into(), toml::Value::try_from(serde_json::json!({
            "max_batch_values": 2, "max_staged_batches": 2,
            "sources": { "left-policy": {"component": "left", "required": true}, "right-policy": {"component": "right", "required": true} }
        })).unwrap());
    });
    let path = workspace.path().join("sensor-host/Cargo.toml");
    let mut manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["dependencies"].as_table_mut().unwrap().insert("boomerang_runtime".into(), toml::Value::try_from(serde_json::json!({
        "path": PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../boomerang_runtime"), "features": ["hosted-source-input"]
    })).unwrap());
    std::fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    std::fs::write(
        workspace.path().join("vehicle-topology/src/lib.rs"),
        r#"
use boomerang_builder::compiler::{ApplicationTopology, TopologyAuthoringError, TopologyBuilder};
pub fn topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/reused-inputs")?;
    for name in ["left", "right"] {
        let enclave = app.enclave(name)?;
        app.component(name, sensor_host::sensor::definition(), &enclave)?;
    }
    app.finish()
}
"#,
    )
    .unwrap();
    std::fs::write(workspace.path().join("sensor-host/src/lib.rs"), r#"
boomerang::component! {
    pub mod sensor {
        use boomerang::prelude::*;
        use boomerang_runtime::{clock::PhysicalInstant, input::*};
        static PEERS: std::sync::Mutex<Vec<(InputSourceHandle, SourceAction<u32>)>> = std::sync::Mutex::new(Vec::new());
        #[derive(Debug)]
        pub struct DriverState { seen: bool }
        impl DriverState {
            pub fn new(init: DriverInit) -> Result<Self, InputError> {
                let clock = init.source.clock();
                assert!(clock.now().is_ok());
                let mut peers = PEERS.lock().unwrap();
                peers.push((init.source.clone(), init.sample));
                if peers.len() == 2 {
                    let observations = peers.iter().map(|(source, action)| InputObservation {
                        source: source.clone(), sequence: 1, domain: clock.domain(), epoch: clock.epoch(),
                        acquired: PhysicalInstant(0), values: vec![InputValue::new(action, 42)]
                    }).collect();
                    let frontiers: Vec<_> = peers.iter().map(|(source, _)| (source.clone(), PhysicalInstant(1))).collect();
                    let receipt = init.source.submit(InputBatch::new(observations).with_progress(frontiers))?;
                    assert_eq!(receipt.tags.len(), 2);
                    assert!(receipt.tags.iter().all(|tag| *tag == boomerang_runtime::Tag::ZERO));
                    peers.clear();
                }
                Ok(Self { seen: false })
            }
        }
        impl Drop for DriverState { fn drop(&mut self) { assert!(self.seen, "both component instances must receive their sample"); } }
        #[reactor(state = DriverState, state_init = DriverState::new,
            init_context(name = DriverInit, source = "plant", actions = [sample]),
            contract = "vehicle.sensor", contract_version = 1,
            bounds(queue_capacity = 8, payload_bytes = 512, state_bytes = 256, scratch_bytes = 128))]
        pub fn Sensor(#[physical_action] sample: u32) -> impl Reactor {
            reaction! { observe (sample) {
                assert_eq!(ctx.get_action_value(&mut sample), Some(&42));
                assert_eq!(ctx.get_tag(), boomerang_runtime::Tag::ZERO);
                state.seen = true;
                ctx.schedule_shutdown(None);
            } }
        }
    }
}
"#).unwrap();
    assert!(std::process::Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(workspace.path())
        .status()
        .unwrap()
        .success());
    let launcher = support::with_target_directory(&target, || {
        cargo_boomerang::generate_launcher(workspace.path(), "production", "host").unwrap()
    });
    launcher.run_locked_offline().unwrap();
}
