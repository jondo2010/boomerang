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

/// Materializes one stable package tree per test binary under Cargo's test target.
/// The next run (or `cargo clean`) removes it; no process-static TempDir leaks into source.
pub fn fixture_workspace() -> PathBuf {
    static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE
        .get_or_init(|| {
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace");
            let destination = shared_target("workspace");
            copy_tree(&source, &destination);
            destination
        })
        .clone()
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "target" {
            continue;
        }
        let output = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &output);
        } else if entry.file_name() == "Cargo.toml" {
            // Keep fixture packages together, but resolve dependencies on repository crates
            // before moving the workspace away from its original directory depth.
            let mut manifest: toml::Value =
                toml::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(dependencies) = manifest
                    .get_mut(section)
                    .and_then(toml::Value::as_table_mut)
                {
                    for (_, dependency) in dependencies.iter_mut() {
                        if let Some(path) = dependency.get_mut("path") {
                            let relative = path.as_str().unwrap();
                            // The tiny fixture's peer dependencies start with ../; repository
                            // dependencies climb further. Absolute paths are already relocated.
                            if relative.starts_with("../../") {
                                *path = std::fs::canonicalize(source.join(relative))
                                    .unwrap()
                                    .to_str()
                                    .unwrap()
                                    .into();
                            }
                        }
                    }
                }
            }
            std::fs::write(output, toml::to_string(&manifest).unwrap()).unwrap();
        } else {
            std::fs::copy(entry.path(), output).unwrap();
        }
    }
}

pub fn copied_fixture_workspace() -> tempfile::TempDir {
    let destination = tempfile::tempdir().unwrap();
    copy_tree(&fixture_workspace(), destination.path());
    destination
}

/// Restores a scenario's manifest even when one of its assertions panics.
#[must_use]
pub struct ManifestGuard {
    path: PathBuf,
    original: String,
}

impl Drop for ManifestGuard {
    fn drop(&mut self) {
        std::fs::write(&self.path, &self.original).expect("restore fixture manifest");
    }
}

pub fn edit_manifest(workspace: &Path, edit: impl FnOnce(&mut toml::Value)) -> ManifestGuard {
    let path = workspace.join("Boomerang.toml");
    let original = std::fs::read_to_string(&path).unwrap();
    let mut manifest = toml::from_str(&original).unwrap();
    edit(&mut manifest);
    let guard = ManifestGuard { path, original };
    std::fs::write(&guard.path, toml::to_string(&manifest).unwrap()).unwrap();
    guard
}

/// Adds only the configuration delta needed by a scenario. Call while holding toolchain_lock.
pub fn fixture_variant(
    name: &str,
    base: &str,
    edit: impl FnOnce(&mut toml::Value),
) -> ManifestGuard {
    edit_manifest(&fixture_workspace(), |manifest| {
        let mut deployment = manifest["deployments"][base].clone();
        edit(&mut deployment);
        manifest["deployments"]
            .as_table_mut()
            .unwrap()
            .insert(name.into(), deployment);
    })
}

pub fn hosted_fixture() -> ManifestGuard {
    edit_manifest(&fixture_workspace(), |manifest| {
        manifest["topology"]["entry"] = "vehicle_topology::tagged_topology".into();
        manifest["deployments"]["sensor-slice"]["rti"]["target"] =
            target_lexicon::HOST.to_string().into();
    })
}

pub fn shared_target(lane: &str) -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        // One test binary must not delete another's workspace or coverage objects.
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("cargo-boomerang-fixtures")
            .join(env!("CARGO_CRATE_NAME"));
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
            "vehicle-control::controller" => {
                owned_reference_payloads::controller::__boomerang::binding_manifest()
            }
            "sensor-host::sensor" => {
                owned_reference_payloads::sensor::__boomerang::binding_manifest()
            }
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
            image,
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
            "processed_tags": stats.processed_tags(),
            "processed_reactions": stats.processed_reactions(),
            "processed_events": stats.processed_events(),
            "set_ports": stats.set_ports(),
            "scheduled_actions": stats.scheduled_actions(),
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

/// Inspect emitted target libraries from generated launcher builds, excluding host tooling builds.
/// Unfiltered Cargo metadata also lists the facade's cfg-disabled hosted dependencies.
pub fn launcher_payload_crates(executable: &Path) -> std::collections::BTreeSet<String> {
    fn collect(directory: &Path, crates: &mut std::collections::BTreeSet<String>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(&path, crates);
            } else if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("rlib" | "rmeta")
            ) {
                let filename = path.file_stem().unwrap().to_str().unwrap();
                if let Some(name) = filename
                    .strip_prefix("lib")
                    .and_then(|value| value.split('-').next())
                {
                    crates.insert(name.to_owned());
                }
            }
        }
    }
    // BuiltLauncher returns a private executable copy directly below its build target.
    // Inspect only this launcher's explicit target tree, not host tooling or other builds.
    let build_target = executable.parent().unwrap().parent().unwrap();
    let payload = build_target.join(target_lexicon::HOST.to_string());
    let mut crates = std::collections::BTreeSet::new();
    collect(&payload, &mut crates);
    assert!(
        crates.contains("boomerang_runtime"),
        "no target runtime artifact found: {crates:?}"
    );
    crates
}
