#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use boomerang_builder::compiler::{
    lower, CoordinationBackendId, CoordinationSelection, FederateConfig, FederateId,
    ImplementationBinding, PlacementAssignment, PlacementGroupId, ResolvedDeployment,
    RuntimeBackendId, TargetTriple,
};
use boomerang_runtime::{
    execute_owned_federate,
    image::{
        CompiledDeploymentImage, EnclaveImage, FederateImage, FederateIndex, GlobalFederationImage,
        IdentityRange,
    },
    Config,
};
use serde_json::{json, Value};
use tinymap::{TableRange, TinyMapView};

pub fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

pub fn copied_fixture_workspace() -> tempfile::TempDir {
    fn copy_tree(source: &Path, destination: &Path) {
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == "target" {
                continue;
            }
            let destination = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir(&destination).unwrap();
                copy_tree(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    let source = fixture_workspace();
    let destination = tempfile::tempdir_in(source.parent().unwrap()).unwrap();
    copy_tree(&source, destination.path());
    destination
}

pub fn shared_target(lane: &str) -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cargo-boomerang-fixtures");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        root
    });
    root.join(lane)
}

pub fn toolchain_target() -> PathBuf {
    shared_target("toolchain")
}

pub fn reset_deployment_output(target: &Path, deployment: &str) {
    let output = target.join("boomerang").join(deployment);
    if output.exists() {
        std::fs::remove_dir_all(output).unwrap();
    }
}

pub fn toolchain_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Executes the analyzed fixture deployment through fixture-owned payload bindings.
pub fn owned_reference_summary(deployment_name: &str) -> Value {
    let workspace = fixture_workspace();
    let resolved = cargo_boomerang::resolve_workspace(&workspace, deployment_name).unwrap();
    let driver = cargo_boomerang::run_descriptor_driver(&workspace, deployment_name).unwrap();
    for binding in driver.bindings() {
        let payload = match binding.implementation().as_str() {
            "vehicle-control" => {
                owned_reference_payloads::controller::__boomerang::BINDING_MANIFEST
            }
            "sensor-host" => owned_reference_payloads::sensor::__boomerang::BINDING_MANIFEST,
            implementation => panic!("unexpected fixture implementation {implementation}"),
        };
        assert_eq!(
            binding
                .descriptor()
                .descriptor_fingerprint_input()
                .fingerprint(),
            payload.descriptor_fingerprint(),
            "fixture-owned payload must match the analyzed descriptor"
        );
    }
    let bindings = driver.bindings().iter().map(|binding| {
        ImplementationBinding::new(
            binding.component().clone(),
            binding.implementation().clone(),
            binding.descriptor().clone(),
        )
    });
    let placements = resolved
        .deployment()
        .federates
        .iter()
        .flat_map(|(federate, config)| {
            config.groups.iter().map(move |group| {
                Ok(PlacementAssignment::new(
                    PlacementGroupId::new(group)?,
                    FederateId::new(federate.as_str())?,
                ))
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    let federates = resolved
        .deployment()
        .federates
        .iter()
        .map(|(id, config)| {
            let target = config
                .target
                .clone()
                .unwrap_or_else(|| target_lexicon::HOST.to_string());
            Ok(FederateConfig::new(
                FederateId::new(id.as_str())?,
                TargetTriple::new(target)?,
                RuntimeBackendId::new(config.runtime.as_str())?,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    let coordination = match resolved.deployment().coordination.as_ref() {
        None => CoordinationSelection::Local,
        Some(coordination) => CoordinationSelection::Distributed {
            backend: CoordinationBackendId::new(match coordination.backend {
                cargo_boomerang::CoordinationBackend::CentralRti => "central-rti",
                cargo_boomerang::CoordinationBackend::PeerToPeer => "peer-to-peer",
            })
            .unwrap(),
        },
    };
    let compiled = lower(
        &ResolvedDeployment::new(
            driver.topology().clone(),
            bindings,
            placements,
            federates,
            coordination,
            [],
        )
        .unwrap(),
    )
    .unwrap();
    compiled.validate().unwrap();

    let mut identity_data = String::new();
    let mut federate_images = Vec::new();
    let mut enclave_images = Vec::<EnclaveImage<'_>>::new();
    for federate in compiled.federates() {
        let enclave_start = u32::try_from(enclave_images.len()).unwrap();
        enclave_images.extend(federate.enclaves().iter().map(|enclave| enclave.image()));
        let enclave_len = u32::try_from(federate.enclaves().len()).unwrap();
        let mut append_identity = |value: &dyn std::fmt::Display| {
            let start = u32::try_from(identity_data.len()).unwrap();
            let value = value.to_string();
            let len = u32::try_from(value.len()).unwrap();
            identity_data.push_str(&value);
            IdentityRange::new(start, len)
        };
        let id = append_identity(federate.id());
        let target = append_identity(federate.target());
        let runtime = append_identity(federate.runtime());
        federate_images.push(FederateImage::new(
            id,
            target,
            runtime,
            TableRange::new(enclave_start, enclave_len),
        ));
    }
    let members = compiled
        .federation()
        .members()
        .iter()
        .map(|member| {
            let index = compiled
                .federates()
                .iter()
                .position(|federate| federate.id() == member)
                .unwrap();
            FederateIndex::new(u32::try_from(index).unwrap())
        })
        .collect::<Vec<_>>();
    let image = CompiledDeploymentImage {
        identity_data: &identity_data,
        federation: GlobalFederationImage::new(&members, &[]),
        federates: TinyMapView::new(&federate_images),
        enclaves: TinyMapView::new(&enclave_images),
        coordination: compiled.coordination(),
    };
    let execution = execute_owned_federate(
        &image,
        FederateIndex::new(0),
        owned_reference_payloads::bindings(),
        Config::default(),
    )
    .unwrap();
    let stats = execution.stats();
    json!({
        "schema": 1,
        "stats": {
            "processed_tags": stats.processed_tags().to_string(),
            "processed_reactions": stats.processed_reactions().to_string(),
            "processed_events": stats.processed_events().to_string(),
            "set_ports": stats.set_ports().to_string(),
            "scheduled_actions": stats.scheduled_actions().to_string(),
        },
        "final_tag": {
            "offset_nanos": execution.final_tag().offset().whole_nanoseconds().to_string(),
            "microstep": execution.final_tag().microstep().to_string(),
        },
    })
}

/// Removes terminal styling before making semantic assertions about CLI output.
pub fn without_ansi(output: &str) -> String {
    anstream::adapter::strip_str(output).to_string()
}

/// Asserts the complete ordered sequence of cargo-boomerang progress labels.
pub fn assert_progress_phases(stderr: &str, expected: &[&str]) {
    const PHASES: [&str; 7] = [
        "Analyzing",
        "Generating",
        "Building",
        "Validating",
        "Bundling",
        "Publishing",
        "Running",
    ];
    let plain_stderr = without_ansi(stderr);
    let actual = plain_stderr
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| PHASES.contains(word))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "unexpected progress sequence:\n{stderr}");
}

struct TargetDirectoryGuard(Option<OsString>);

impl Drop for TargetDirectoryGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(previous) => unsafe { std::env::set_var("CARGO_TARGET_DIR", previous) },
            None => unsafe { std::env::remove_var("CARGO_TARGET_DIR") },
        }
    }
}

pub fn with_target_directory<T>(target: &Path, operation: impl FnOnce() -> T) -> T {
    static LOCK: Mutex<()> = Mutex::new(());
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _guard = TargetDirectoryGuard(std::env::var_os("CARGO_TARGET_DIR"));
    unsafe { std::env::set_var("CARGO_TARGET_DIR", target) };
    operation()
}
