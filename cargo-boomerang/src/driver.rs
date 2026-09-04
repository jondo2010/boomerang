//! Lifecycle and process boundary for generated host descriptor drivers.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    io::Cursor,
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
use cargo_metadata::{Message, Metadata, TargetKind};

use crate::{
    generated::render_descriptor_driver,
    generated_cache::{
        generated_cargo_program, resolve_generated_workspace, GeneratedRole,
        GeneratedWorkspaceRequest, RequestIdentityBuilder,
    },
    resolve_workspace, ResolvedWorkspace,
};

/// Validated descriptor-driver result plus Cargo diagnostics captured from stderr.
pub struct DriverOutput {
    /// Validated topology and implementation descriptors decoded from stdout.
    output: DescriptorDriverOutput,
    /// Combined stderr from lock reconciliation and driver compilation/execution.
    build_log: String,
    /// Number of compiler artifacts Cargo rebuilt while preparing this driver.
    compiled_artifacts: usize,
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
    /// Returns the number of non-fresh Cargo compiler artifacts from this build.
    pub const fn compiled_artifacts(&self) -> usize {
        self.compiled_artifacts
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
    let cargo_program = generated_cargo_program();
    let mut identity = RequestIdentityBuilder::new(GeneratedRole::Descriptor);
    identity.field("manifest", Some(generated.manifest.as_bytes()));
    identity.field("source", Some(generated.main.as_bytes()));
    identity.field("source-lock-digest", Some(&resolved.lockfile().digest));
    for package_id in resolved.driver_package_ids() {
        identity.field("driver-package-id", Some(package_id.as_bytes()));
    }
    identity.field(
        "cargo-program",
        Some(cargo_program.to_string_lossy().as_bytes()),
    );
    identity.field("target", Some(b"host"));
    identity.field("profile", Some(b"default"));
    identity.field("toolchain", Some(b"default"));
    let request = GeneratedWorkspaceRequest {
        role: GeneratedRole::Descriptor,
        identity: identity.finish(),
        manifest: generated.manifest.as_bytes(),
        source: generated.main.as_bytes(),
        source_lockfile: &resolved.lockfile().path,
        source_lock_digest: &resolved.lockfile().digest,
    };
    let build_log = std::cell::RefCell::new(String::new());
    let generated = resolve_generated_workspace(
        resolved.target_directory(),
        &request,
        |directory| {
            let reconciliation = cargo(
                &cargo_program,
                directory,
                ["metadata", "--format-version", "1", "--offline"],
            )?;
            let mut log = build_log.borrow_mut();
            log.push_str(&String::from_utf8_lossy(&reconciliation.stderr));
            require_success("lock reconciliation", &reconciliation, &log)
        },
        |directory| {
            let metadata = cargo(
                &cargo_program,
                directory,
                ["metadata", "--format-version", "1", "--locked", "--offline"],
            )?;
            let mut log = build_log.borrow_mut();
            log.push_str(&String::from_utf8_lossy(&metadata.stderr));
            require_success("locked metadata verification", &metadata, &log)?;
            let metadata: Metadata = serde_json::from_slice(&metadata.stdout)
                .context("failed to decode generated Cargo metadata")?;
            validate_generated_graph(resolved, &metadata)
        },
    )?;
    let (execution, executable, compiled_artifacts) = generated.with_locked_target(|target| {
        let build = cargo(
            &cargo_program,
            generated.directory(),
            [
                OsStr::new("build"),
                OsStr::new("--locked"),
                OsStr::new("--offline"),
                OsStr::new("--message-format=json-render-diagnostics"),
                OsStr::new("--manifest-path"),
                generated.manifest_path().as_os_str(),
                OsStr::new("--target-dir"),
                target.as_os_str(),
            ],
        )?;
        let mut log = build_log.borrow_mut();
        log.push_str(&String::from_utf8_lossy(&build.stderr));
        let artifact = descriptor_artifact(&build, &mut log);
        require_success("build", &build, &log)?;
        let (executable, compiled_artifacts) = artifact?;
        let (execution, executable) = copy_private_descriptor(&executable)?;
        Ok((execution, executable, compiled_artifacts))
    })?;
    let descriptor = Command::new(&executable)
        .output()
        .with_context(|| format!("failed to execute {}", executable.display()))?;
    build_log
        .borrow_mut()
        .push_str(&String::from_utf8_lossy(&descriptor.stderr));
    require_success("execution", &descriptor, &build_log.borrow())?;
    let output = decode_descriptor_driver_output(descriptor.stdout.as_slice())?;
    drop(execution);
    Ok(DriverOutput {
        output,
        build_log: build_log.into_inner(),
        compiled_artifacts,
    })
}
/// Invokes the current Cargo executable with deterministic arguments in the generated crate.
fn cargo<I, S>(program: &OsStr, crate_dir: &Path, arguments: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(arguments)
        .current_dir(crate_dir)
        .env("BOOMERANG_DESCRIPTOR_DRIVER", "1")
        .output()
        .with_context(|| format!("failed to invoke Cargo in {}", crate_dir.display()))
}

/// Extracts the generated descriptor executable and non-fresh artifact count from Cargo messages.
fn descriptor_artifact(build: &Output, build_log: &mut String) -> Result<(PathBuf, usize)> {
    let mut executable = None;
    let mut compiled_artifacts = 0;
    for message in Message::parse_stream(Cursor::new(build.stdout.as_slice())) {
        match message.context("failed to decode Cargo build message")? {
            Message::CompilerArtifact(artifact) => {
                if !artifact.fresh {
                    compiled_artifacts += 1;
                }
                if artifact.target.name == "boomerang-descriptor-driver"
                    && artifact
                        .target
                        .kind
                        .iter()
                        .any(|kind| kind == &TargetKind::Bin)
                {
                    let artifact = artifact.executable.ok_or_else(|| {
                        anyhow!("descriptor driver Cargo artifact has no executable")
                    })?;
                    if executable.replace(artifact.into_std_path_buf()).is_some() {
                        bail!("generated Cargo manifest produced multiple descriptor binaries");
                    }
                }
            }
            Message::CompilerMessage(message) => {
                if let Some(rendered) = message.message.rendered {
                    build_log.push_str(&rendered);
                }
            }
            _ => {}
        }
    }
    let executable = executable
        .ok_or_else(|| anyhow!("generated Cargo manifest produced no descriptor binary"))?;
    Ok((executable, compiled_artifacts))
}

/// Copies the Cargo-selected descriptor binary into an invocation-private executable path.
fn copy_private_descriptor(source: &Path) -> Result<(tempfile::TempDir, PathBuf)> {
    let execution = tempfile::Builder::new()
        .prefix("boomerang-descriptor-")
        .tempdir()
        .context("failed to prepare private descriptor execution directory")?;
    let filename = source.file_name().ok_or_else(|| {
        anyhow!(
            "descriptor executable {} has no file name",
            source.display()
        )
    })?;
    let destination = execution.path().join(filename);
    let source_hash = hash_file(source)?;
    fs::copy(source, &destination)
        .with_context(|| format!("failed to copy descriptor executable {}", source.display()))?;
    if source_hash != hash_file(&destination)? {
        bail!("copied descriptor executable differs from Cargo artifact");
    }
    Ok((execution, destination))
}

/// Hashes one executable using the same BLAKE3 identity used for generated source inputs.
fn hash_file(path: &Path) -> Result<blake3::Hash> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(blake3::hash(&bytes))
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
