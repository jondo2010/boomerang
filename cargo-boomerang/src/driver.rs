//! Lifecycle and process boundary for generated host descriptor drivers.

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{anyhow, bail, Context, Result};
use boomerang_builder::{
    compiler::ApplicationTopology,
    host_interchange::{
        decode_descriptor_driver_output, DescriptorDriverBinding, DescriptorDriverOutput,
    },
};
use cargo_metadata::Metadata;

use crate::{generated::render_descriptor_driver, resolve_workspace, ResolvedWorkspace};

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

/// Resolves a deployment and runs its generated host descriptor driver offline.
pub fn run_descriptor_driver(
    workspace: impl AsRef<Path>,
    deployment_name: &str,
) -> Result<DriverOutput> {
    let resolved = resolve_workspace(workspace, deployment_name)?;
    run_resolved_descriptor_driver(&resolved)
}

/// Runs the generated host descriptor driver for an already resolved workspace.
pub(crate) fn run_resolved_descriptor_driver(resolved: &ResolvedWorkspace) -> Result<DriverOutput> {
    let generated = render_descriptor_driver(resolved)?;
    let driver_parent = resolved
        .target_directory()
        .join("boomerang/descriptor-driver");
    fs::create_dir_all(&driver_parent)
        .with_context(|| format!("failed to prepare {}", driver_parent.display()))?;
    let driver = tempfile::Builder::new()
        .prefix("driver-")
        .tempdir_in(&driver_parent)
        .with_context(|| format!("failed to prepare {}", driver_parent.display()))?;
    let crate_dir = driver.path();
    write(crate_dir.join("Cargo.toml"), generated.manifest)?;
    let source_dir = crate_dir.join("src");
    fs::create_dir(&source_dir)
        .with_context(|| format!("failed to prepare {}", source_dir.display()))?;
    write(source_dir.join("main.rs"), generated.main)?;
    let generated_lock = crate_dir.join("Cargo.lock");
    fs::copy(resolved.lockfile().path.as_path(), &generated_lock)
        .with_context(|| format!("failed to prepare {}", generated_lock.display()))?;

    let reconcile = cargo(
        crate_dir,
        ["metadata", "--format-version", "1", "--offline"],
    )?;
    let mut build_log = String::from_utf8_lossy(&reconcile.stderr).into_owned();
    require_success("lock reconciliation", &reconcile, &build_log)?;
    let metadata = cargo(
        crate_dir,
        ["metadata", "--format-version", "1", "--locked", "--offline"],
    )?;
    build_log.push_str(&String::from_utf8_lossy(&metadata.stderr));
    require_success("locked metadata verification", &metadata, &build_log)?;
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout)
        .context("failed to decode generated Cargo metadata")?;
    validate_generated_graph(resolved, &metadata)?;

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
fn write(path: PathBuf, contents: String) -> Result<()> {
    fs::write(&path, contents).with_context(|| format!("failed to prepare {}", path.display()))
}
/// Invokes the current Cargo executable with deterministic arguments in the generated crate.
fn cargo<const N: usize>(crate_dir: &Path, arguments: [&str; N]) -> Result<Output> {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(arguments)
        .current_dir(crate_dir)
        .env("BOOMERANG_DESCRIPTOR_DRIVER", "1")
        .output()
        .with_context(|| format!("failed to invoke Cargo in {}", crate_dir.display()))
}
/// Converts a failed phase into a diagnostic preserving accumulated stderr.
fn require_success(phase: &'static str, output: &Output, build_log: &str) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "descriptor-driver {phase} failed with {}\n{build_log}",
            output.status
        )
    }
}

/// Confirms exact root selections and keeps every transitive package within the source lock graph.
fn validate_generated_graph(
    resolved: &crate::ResolvedWorkspace,
    metadata: &Metadata,
) -> Result<()> {
    let root = metadata
        .root_package()
        .ok_or_else(|| anyhow!("generated metadata has no root package"))?;
    let graph = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| anyhow!("generated metadata has no resolve graph"))?;
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
        bail!("generated root packages differ: expected {expected:?}, found {root_dependencies:?}");
    }
    for node in graph.nodes.iter().filter(|node| node.id != root.id) {
        let id = node.id.to_string();
        if node
            .features
            .iter()
            .any(|feature| *feature == "__boomerang_payload")
        {
            bail!("package {id} activates reserved payload facet");
        }
        if !resolved.locked_package_ids().contains(&id) {
            bail!("package {id} was absent from source metadata");
        }
    }
    Ok(())
}
