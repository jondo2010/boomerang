#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use boomerang_builder::compiler::{
    CoordinationSelection, FederateConfig, FederateId, ImplementationBinding, PlacementAssignment,
    PlacementGroupId, ResolvedDeployment, RuntimeBackendId, TargetTriple,
};
use boomerang_runtime::{execute_owned_federate, image::FederateIndex, Config};
use serde_json::{json, Value};

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
                config.recovery,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .unwrap();
    let coordination = match resolved.deployment().coordination.as_ref() {
        None => CoordinationSelection::Local,
        Some(coordination) => CoordinationSelection::Distributed {
            backend: coordination.backend,
        },
    };
    let compiled = ResolvedDeployment::new(
        driver.topology().clone(),
        bindings,
        placements,
        federates,
        coordination,
        [],
    )
    .unwrap()
    .lower()
    .unwrap();
    compiled.validate().unwrap();

    let selected = FederateIndex::new(0);
    let execution = compiled.with_image(|image| {
        execute_owned_federate(
            &image,
            selected,
            owned_reference_payloads::bindings(),
            Config::default(),
        )
        .unwrap()
    });
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
    const PHASES: [&str; 8] = [
        "Analyzing",
        "Generating",
        "Building",
        "Validating",
        "Bundling",
        "Publishing",
        "Published",
        "Running",
    ];
    let plain_stderr = without_ansi(stderr);
    let actual = plain_stderr
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let phase = words.next()?;
            if phase == "Running" && words.next().is_some_and(|word| word.starts_with('`')) {
                return None;
            }
            Some(phase)
        })
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
