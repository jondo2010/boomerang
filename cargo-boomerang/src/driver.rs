//! Lifecycle and process boundary for generated host descriptor drivers.

use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use boomerang_builder::{
    compiler::ApplicationTopology,
    host_interchange::{
        decode_descriptor_driver_output, DescriptorDriverBinding, DescriptorDriverOutput,
        HostInterchangeError,
    },
};
use cargo_metadata::Metadata;
use thiserror::Error;

use crate::{generated::render_descriptor_driver, resolve_workspace, WorkspaceError};

/// Validated descriptor-driver result plus Cargo diagnostics captured from stderr.
pub struct DriverOutput {
    /// Validated topology and implementation descriptors decoded from stdout.
    output: DescriptorDriverOutput,
    /// Combined stderr from lock reconciliation and driver compilation/execution.
    build_log: String,
}

impl DriverOutput {
    /// Returns the canonical application topology emitted by the topology entry point.
    pub fn topology(&self) -> &ApplicationTopology {
        self.output.topology()
    }
    /// Returns validated logical-component-to-descriptor bindings.
    pub fn bindings(&self) -> &[DescriptorDriverBinding] {
        self.output.bindings()
    }
    /// Iterates selected implementation package names in canonical lexical order.
    pub fn selected_packages(&self) -> impl Iterator<Item = &str> {
        self.output
            .bindings()
            .iter()
            .map(|binding| binding.implementation().as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
    }
    /// Returns Cargo and generated-process diagnostics captured exclusively from stderr.
    pub fn build_log(&self) -> &str {
        &self.build_log
    }
}

/// Failure while generating, compiling, running, or decoding a descriptor driver.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct DescriptorDriverError {
    /// Complete user-facing diagnostic, including captured Cargo stderr when available.
    message: String,
}
impl From<WorkspaceError> for DescriptorDriverError {
    fn from(error: WorkspaceError) -> Self {
        driver_error(error)
    }
}
impl From<HostInterchangeError> for DescriptorDriverError {
    fn from(error: HostInterchangeError) -> Self {
        driver_error(error)
    }
}
/// Resolves a deployment and runs its generated host descriptor driver offline.
pub fn run_descriptor_driver(
    workspace: impl AsRef<Path>,
    deployment_name: &str,
) -> Result<DriverOutput, DescriptorDriverError> {
    let resolved = resolve_workspace(workspace, deployment_name)?;
    let generated = render_descriptor_driver(&resolved).map_err(driver_error)?;
    let driver_parent = resolved
        .target_directory()
        .join("boomerang/descriptor-driver");
    fs::create_dir_all(&driver_parent).map_err(|error| path_error(&driver_parent, error))?;
    let driver = tempfile::Builder::new()
        .prefix("driver-")
        .tempdir_in(&driver_parent)
        .map_err(|error| path_error(&driver_parent, error))?;
    let crate_dir = driver.path();
    write(crate_dir.join("Cargo.toml"), generated.manifest)?;
    let source_dir = crate_dir.join("src");
    fs::create_dir(&source_dir).map_err(|error| path_error(&source_dir, error))?;
    write(source_dir.join("main.rs"), generated.main)?;
    let generated_lock = crate_dir.join("Cargo.lock");
    fs::copy(resolved.lockfile().path.as_path(), &generated_lock)
        .map_err(|error| path_error(&generated_lock, error))?;

    let reconcile = cargo(crate_dir, ["generate-lockfile", "--offline"])?;
    let mut build_log = String::from_utf8_lossy(&reconcile.stderr).into_owned();
    require_success("lock reconciliation", &reconcile, &build_log)?;
    let metadata = cargo(
        crate_dir,
        ["metadata", "--format-version", "1", "--locked", "--offline"],
    )?;
    build_log.push_str(&String::from_utf8_lossy(&metadata.stderr));
    require_success("locked metadata verification", &metadata, &build_log)?;
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout).map_err(driver_error)?;
    validate_generated_graph(&resolved, &metadata)?;

    let target_dir = crate_dir.join("target");
    let target_dir = target_dir.to_string_lossy().into_owned();
    let run = cargo(
        crate_dir,
        [
            "run",
            "--quiet",
            "--locked",
            "--offline",
            "--target-dir",
            target_dir.as_str(),
        ],
    )?;
    build_log.push_str(&String::from_utf8_lossy(&run.stderr));
    require_success("build or execution", &run, &build_log)?;
    let output = decode_descriptor_driver_output(run.stdout.as_slice())?;
    Ok(DriverOutput { output, build_log })
}
/// Writes one generated file with its path attached to filesystem diagnostics.
fn write(path: PathBuf, contents: String) -> Result<(), DescriptorDriverError> {
    fs::write(&path, contents).map_err(|error| path_error(&path, error))
}
/// Invokes the current Cargo executable with deterministic arguments in the generated crate.
fn cargo<const N: usize>(
    crate_dir: &Path,
    arguments: [&str; N],
) -> Result<Output, DescriptorDriverError> {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(arguments)
        .current_dir(crate_dir)
        .env("BOOMERANG_DESCRIPTOR_DRIVER", "1")
        .output()
        .map_err(driver_error)
}
/// Converts a failed phase into a diagnostic preserving accumulated stderr.
fn require_success(
    phase: &'static str,
    output: &Output,
    build_log: &str,
) -> Result<(), DescriptorDriverError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(driver_error(format!(
            "descriptor-driver {phase} failed with {}\n{build_log}",
            output.status
        )))
    }
}
/// Attaches a filesystem path to a generated-crate preparation failure.
fn path_error(path: &Path, error: io::Error) -> DescriptorDriverError {
    driver_error(format!(
        "failed to prepare generated descriptor driver at {}: {error}",
        path.display()
    ))
}
/// Converts any displayable failure into the descriptor driver's public error wrapper.
fn driver_error(error: impl std::fmt::Display) -> DescriptorDriverError {
    DescriptorDriverError {
        message: error.to_string(),
    }
}

/// Confirms exact root selections and keeps every transitive package within the source lock graph.
fn validate_generated_graph(
    resolved: &crate::ResolvedWorkspace,
    metadata: &Metadata,
) -> Result<(), DescriptorDriverError> {
    let root = metadata
        .root_package()
        .ok_or_else(|| driver_error("generated metadata has no root package"))?;
    let graph = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| driver_error("generated metadata has no resolve graph"))?;
    let root_node = graph
        .nodes
        .iter()
        .find(|node| node.id == root.id)
        .expect("Cargo resolve graph contains its root");
    let root_dependencies = root_node
        .deps
        .iter()
        .map(|dependency| dependency.pkg.to_string())
        .collect::<BTreeSet<_>>();
    let expected = resolved.driver_package_ids();
    if root_dependencies != expected {
        return Err(driver_error(format!(
            "generated root packages differ: expected {expected:?}, found {root_dependencies:?}"
        )));
    }
    for node in graph.nodes.iter().filter(|node| node.id != root.id) {
        let id = node.id.to_string();
        if node
            .features
            .iter()
            .any(|feature| *feature == "__boomerang_payload")
        {
            return Err(driver_error(format!(
                "package {id} activates reserved payload facet"
            )));
        }
        if !resolved.locked_package_ids().contains(&id) {
            return Err(driver_error(format!(
                "package {id} was absent from source metadata"
            )));
        }
    }
    Ok(())
}
