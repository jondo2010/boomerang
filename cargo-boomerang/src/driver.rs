//! Lifecycle and process boundary for generated host descriptor drivers.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{anyhow, bail, Context, Result};
use boomerang_builder::{
    compiler::ApplicationTopology,
    host_interchange::{
        decode_descriptor_output, decode_topology_output, DescriptorDriverBinding,
        DescriptorDriverOutput,
    },
};
use cargo_metadata::{Message, Metadata, PackageId};

use crate::{
    codegen::rendered_compiler_diagnostics,
    facet::{CompilerWrappers, Facet},
    generated::{render_descriptor_driver, render_topology_driver, GeneratedCrate},
    generated_cache::{
        artifact_matches, copy_private_artifact, generated_cargo_program,
        resolve_generated_workspace, GeneratedRole, GeneratedWorkspaceRequest, RequestIdentity,
        RequestIdentityBuilder,
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
    selected_packages: BTreeSet<String>,
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
        self.selected_packages.iter().map(String::as_str)
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
    run_resolved_descriptor_driver(&resolved, &crate::CommandOutput::silent())
}

/// Runs the generated host descriptor driver for an already resolved workspace.
pub(crate) fn run_resolved_descriptor_driver(
    resolved: &ResolvedWorkspace,
    output: &crate::CommandOutput,
) -> Result<DriverOutput> {
    output.status(crate::output::Phase::Generating, "topology driver")?;
    let topology_source = render_topology_driver(resolved)?;
    output.status(crate::output::Phase::Building, "topology driver")?;
    let wrapper = prepare_compiler_wrapper(resolved, output)?;
    let topology = build_and_run_host_stage(
        resolved,
        GeneratedRole::Topology,
        topology_source,
        &wrapper,
        output,
    )?;
    let descriptors = run_host_stage(resolved, GeneratedRole::Descriptor, &wrapper, output)?;
    let joined = DescriptorDriverOutput::try_new(
        decode_topology_output(topology.0.as_slice())?,
        decode_descriptor_output(descriptors.0.as_slice())?,
    )?;
    Ok(DriverOutput {
        output: joined,
        build_log: topology.1 + &descriptors.1,
        compiled_artifacts: topology.2 + descriptors.2,
        selected_packages: resolved
            .deployment()
            .bindings
            .values()
            .map(|binding| binding.package.clone())
            .collect(),
    })
}

/// Builds a small native compiler wrapper once per source/lock/compiler context.
pub(crate) fn prepare_compiler_wrapper(
    resolved: &ResolvedWorkspace,
    output: &crate::CommandOutput,
) -> Result<PathBuf> {
    let generated = GeneratedCrate {
        manifest: "[package]\nname = \"boomerang-facet-rustc\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n".to_owned(),
        main: include_str!("facet_rustc.rs").to_owned(),
    };
    let (executable, _, _, _) = build_host_program(
        resolved,
        generated,
        GeneratedRole::CompilerWrapper,
        None,
        output,
    )?;
    Ok(executable)
}

fn host_program_name(role: GeneratedRole) -> &'static str {
    match role {
        GeneratedRole::Topology => "boomerang-topology-driver",
        GeneratedRole::Descriptor => "boomerang-descriptor-driver",
        GeneratedRole::CompilerWrapper => "boomerang-facet-rustc",
        GeneratedRole::Launcher => unreachable!("target launchers use their own build path"),
    }
}

fn run_host_stage(
    resolved: &ResolvedWorkspace,
    role: GeneratedRole,
    wrapper: &Path,
    output: &crate::CommandOutput,
) -> Result<(Vec<u8>, String, usize)> {
    let topology = role == GeneratedRole::Topology;
    let label = if topology {
        "topology driver"
    } else {
        "descriptor driver"
    };
    output.status(crate::output::Phase::Generating, label)?;
    let generated = if topology {
        render_topology_driver(resolved)?
    } else {
        render_descriptor_driver(resolved)?
    };
    output.status(crate::output::Phase::Building, label)?;
    build_and_run_host_stage(resolved, role, generated, wrapper, output)
}

fn build_and_run_host_stage(
    resolved: &ResolvedWorkspace,
    role: GeneratedRole,
    generated: GeneratedCrate,
    wrapper: &Path,
    output: &crate::CommandOutput,
) -> Result<(Vec<u8>, String, usize)> {
    let label = if role == GeneratedRole::Topology {
        "topology driver"
    } else {
        "descriptor driver"
    };
    let (executable, mut log, count, _execution) =
        build_host_program(resolved, generated, role, Some(wrapper), output)?;
    let result = Command::new(&executable)
        .output()
        .with_context(|| format!("failed to execute {label}"))?;
    log.push_str(&String::from_utf8_lossy(&result.stderr));
    require_success("execution", &result, &log)?;
    Ok((result.stdout, log, count))
}

fn build_host_program(
    resolved: &ResolvedWorkspace,
    generated_source: GeneratedCrate,
    role: GeneratedRole,
    wrapper: Option<&Path>,
    output: &crate::CommandOutput,
) -> Result<(PathBuf, String, usize, Option<tempfile::TempDir>)> {
    let cargo_program = generated_cargo_program();
    let application_workspace = resolved.lockfile().path.parent().expect("lockfile parent");
    let roots = if role == GeneratedRole::CompilerWrapper {
        BTreeSet::new()
    } else {
        resolved.host_stage_package_ids(role == GeneratedRole::Topology)
    };
    let identity = host_request_identity(
        role,
        generated_source.manifest.as_bytes(),
        generated_source.main.as_bytes(),
        &resolved.lockfile().digest,
        &roots,
        &cargo_program,
        include_bytes!("facet_rustc.rs"),
    );
    let request = GeneratedWorkspaceRequest {
        role,
        identity,
        manifest: generated_source.manifest.as_bytes(),
        source: generated_source.main.as_bytes(),
        source_lockfile: &resolved.lockfile().path,
        source_lock_digest: &resolved.lockfile().digest,
    };
    let log = std::cell::RefCell::new(String::new());
    let compiler_wrappers = CompilerWrappers::resolve(application_workspace, None)?;
    let facet = if role == GeneratedRole::Descriptor {
        Facet::Descriptor
    } else {
        Facet::Hosted
    };
    let host = target_lexicon::HOST.to_string();
    let metadata = |directory: &Path, locked| {
        let manifest = directory.join("Cargo.toml");
        let mut arguments = vec![
            OsStr::new("metadata"),
            OsStr::new("--manifest-path"),
            manifest.as_os_str(),
            OsStr::new("--format-version"),
            OsStr::new("1"),
            OsStr::new("--filter-platform"),
            OsStr::new(&host),
            OsStr::new("--offline"),
        ];
        if locked {
            arguments.push(OsStr::new("--locked"));
        }
        cargo(
            &cargo_program,
            application_workspace,
            arguments,
            wrapper.map(|path| (facet, path, &compiler_wrappers)),
            output,
        )
    };
    let (generated, package) = resolve_generated_workspace(
        resolved.target_directory(),
        &request,
        |directory| {
            let result = metadata(directory, false)?;
            retain_cargo_diagnostics(&mut log.borrow_mut(), &result, "", output);
            require_success("lock reconciliation", &result, &log.borrow())
        },
        |directory| {
            let result = metadata(directory, true)?;
            retain_cargo_diagnostics(&mut log.borrow_mut(), &result, "", output);
            require_success("locked metadata verification", &result, &log.borrow())?;
            let metadata: Metadata = serde_json::from_slice(&result.stdout)?;
            validate_generated_graph(resolved, &metadata, &roots)
        },
    )?;
    let (executable, count, private) = generated.with_locked_target(|target| {
        let build = cargo(
            &cargo_program,
            application_workspace,
            [
                OsStr::new("build"),
                OsStr::new("--locked"),
                OsStr::new("--offline"),
                OsStr::new("--message-format=json-render-diagnostics"),
                OsStr::new("--manifest-path"),
                generated.manifest_path().as_os_str(),
                OsStr::new("--target-dir"),
                target.as_os_str(),
                OsStr::new("--target"),
                OsStr::new(&host),
            ],
            wrapper.map(|path| (facet, path, &compiler_wrappers)),
            output,
        )?;
        let diagnostics = rendered_compiler_diagnostics(&build.stdout)?;
        retain_cargo_diagnostics(&mut log.borrow_mut(), &build, &diagnostics, output);
        require_success("build", &build, &log.borrow())?;
        let (executable, count) = descriptor_artifact(
            &build,
            &package,
            &generated.manifest_path(),
            host_program_name(role),
        )?;
        if role == GeneratedRole::CompilerWrapper {
            Ok((executable, count, None))
        } else {
            let (private, executable) = copy_private_artifact(&executable, target)?;
            Ok((executable, count, Some(private)))
        }
    })?;
    Ok((executable, log.into_inner(), count, private))
}

/// Retains Cargo diagnostics only when they were not already shown or intentionally suppressed.
fn retain_cargo_diagnostics(
    build_log: &mut String,
    cargo_output: &Output,
    rendered: &str,
    output: &crate::CommandOutput,
) {
    if cargo_output.status.success() && !output.retains_successful_cargo_stderr() {
        return;
    }
    build_log.push_str(rendered);
    build_log.push_str(&String::from_utf8_lossy(&cargo_output.stderr));
}

/// Canonically identifies a descriptor request, preserving every Cargo executable path byte.
#[cfg(all(test, unix))]
fn descriptor_request_identity(
    manifest: &[u8],
    source: &[u8],
    source_lock_digest: &[u8; 32],
    driver_package_ids: &BTreeSet<String>,
    cargo_program: &OsStr,
) -> RequestIdentity {
    host_request_identity(
        GeneratedRole::Descriptor,
        manifest,
        source,
        source_lock_digest,
        driver_package_ids,
        cargo_program,
        include_bytes!("facet_rustc.rs"),
    )
}

fn host_request_identity(
    role: GeneratedRole,
    manifest: &[u8],
    source: &[u8],
    source_lock_digest: &[u8; 32],
    driver_package_ids: &BTreeSet<String>,
    cargo_program: &OsStr,
    wrapper_semantics: &[u8],
) -> RequestIdentity {
    let mut identity = RequestIdentityBuilder::new(role);
    identity.field("facet-wrapper", Some(wrapper_semantics));
    identity.field("manifest", Some(manifest));
    identity.field("source", Some(source));
    identity.field("source-lock-digest", Some(source_lock_digest));
    for package_id in driver_package_ids {
        identity.field("driver-package-id", Some(package_id.as_bytes()));
    }
    identity.field("cargo-program", Some(cargo_program.as_encoded_bytes()));
    identity.field("target", Some(b"host"));
    identity.field("profile", Some(b"default"));
    identity.field("toolchain", Some(b"default"));
    identity.finish()
}

/// Invokes Cargo from the application workspace with deterministic generated-manifest arguments.
fn cargo(
    program: &OsStr,
    application_workspace: &Path,
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    facet: Option<(Facet, &Path, &CompilerWrappers)>,
    output: &crate::CommandOutput,
) -> Result<Output> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(application_workspace)
        .env("BOOMERANG_DESCRIPTOR_DRIVER", "1");
    if let Some((facet, wrapper, configured)) = facet {
        facet.configure(&mut command, wrapper, configured);
    }
    output.configure(&mut command);
    let cargo_output = command.output().with_context(|| {
        format!(
            "failed to invoke Cargo in {}",
            application_workspace.display()
        )
    })?;
    output.forward_cargo_stderr(&cargo_output)?;
    Ok(cargo_output)
}

/// Extracts the generated descriptor executable and non-fresh artifact count from Cargo messages.
fn descriptor_artifact(
    build: &Output,
    package: &PackageId,
    manifest: &Path,
    binary: &str,
) -> Result<(PathBuf, usize)> {
    let mut executable = None;
    let mut compiled_artifacts = 0;
    for message in Message::parse_stream(Cursor::new(build.stdout.as_slice())) {
        if let Message::CompilerArtifact(artifact) =
            message.context("failed to decode Cargo build message")?
        {
            compiled_artifacts += usize::from(!artifact.fresh);
            if artifact_matches(&artifact, package, manifest, binary)? {
                let artifact = artifact
                    .executable
                    .context("descriptor driver Cargo artifact has no executable")?;
                if executable.replace(artifact.into_std_path_buf()).is_some() {
                    bail!("generated Cargo manifest produced multiple descriptor binaries");
                }
            }
        }
    }
    let executable = executable
        .ok_or_else(|| anyhow!("generated Cargo manifest produced no descriptor binary"))?;
    Ok((executable, compiled_artifacts))
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
    expected: &BTreeSet<String>,
) -> Result<PackageId> {
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
    if &root_dependencies != expected {
        bail!("generated root packages differ: expected {expected:?}, found {root_dependencies:?}");
    }
    // Host interchange dependencies belong to the generated tooling, not to
    // author crates. Follow only the exact builder root selected above.
    let mut tool_packages = BTreeSet::new();
    let mut pending = root_node
        .deps
        .iter()
        .filter(|dependency| dependency.pkg == resolved.host_builder().id)
        .map(|dependency| dependency.pkg.clone())
        .collect::<Vec<_>>();
    while let Some(package) = pending.pop() {
        if tool_packages.insert(package.clone()) {
            if let Some(node) = graph.nodes.iter().find(|node| node.id == package) {
                pending.extend(node.deps.iter().map(|dependency| dependency.pkg.clone()));
            }
        }
    }
    for node in graph.nodes.iter().filter(|node| node.id != root.id) {
        let id = node.id.to_string();
        if !resolved.locked_package_ids().contains(&id) && !tool_packages.contains(&node.id) {
            bail!("package {id} was absent from source metadata");
        }
    }
    Ok(root.id.clone())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        collections::BTreeSet,
        ffi::{OsStr, OsString},
        os::unix::ffi::OsStringExt as _,
    };

    use super::{descriptor_request_identity, host_request_identity};
    use crate::generated_cache::GeneratedRole;

    #[test]
    fn descriptor_request_identity_distinguishes_non_unicode_cargo_programs() {
        let first = OsString::from_vec(b"/tmp/cargo-\x80".to_vec());
        let second = OsString::from_vec(b"/tmp/cargo-\x81".to_vec());
        let package_ids = BTreeSet::new();

        assert_ne!(
            descriptor_request_identity(b"manifest", b"source", &[0; 32], &package_ids, &first),
            descriptor_request_identity(b"manifest", b"source", &[0; 32], &package_ids, &second),
        );
    }

    #[test]
    fn host_request_identity_tracks_compiler_wrapper_semantics() {
        let package_ids = BTreeSet::new();
        let identity = |wrapper| {
            host_request_identity(
                GeneratedRole::Topology,
                b"manifest",
                b"source",
                &[0; 32],
                &package_ids,
                OsStr::new("cargo"),
                wrapper,
            )
        };
        assert_ne!(identity(b"first wrapper"), identity(b"second wrapper"));
    }
}
