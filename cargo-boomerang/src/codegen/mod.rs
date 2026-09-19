//! Cached static launcher generation for one compiled Federate.
//!
//! # Compiled-image data flow
//!
//! ```text
//! ResolvedDeployment::lower()
//!     -> OwnedCompiledDeployment
//!     -> FederateSlice
//!     -> generated Rust static tables
//!     -> CompiledDeploymentImage
//!     -> runtime validation and execution
//! ```
//!
//! Code generation is a renderer for an already-lowered image. It preserves deployment-wide typed
//! keys, `IndexSpan` ownership, and `SliceRange` relationship coordinates assigned by the host
//! compiler; it must not renumber tables or allocate a second key domain.
//!
//! Stable identities are emitted as ordinary Rust string literals. Rust and the linker own their
//! placement in static read-only data; this module must not concatenate identities into a custom
//! byte blob or generate byte-offset identity ranges.

mod fingerprints;
mod rti;
mod rust;
pub(crate) use rust::format_rust;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use boomerang_runtime::binding::{
    payload_fingerprint_compile_input_key, PAYLOAD_MACRO_ABI_COMPILE_INPUT,
};
use cargo_metadata::{Message, Metadata, PackageId};

use crate::{
    check::{analyze, AnalyzedDeployment},
    generated::{dependency, runtime_sibling_dependency},
    generated_cache::{
        artifact_matches, copy_private_artifact, generated_cargo_program,
        resolve_generated_workspace, GeneratedRole, GeneratedWorkspace, GeneratedWorkspaceRequest,
        RequestIdentity, RequestIdentityBuilder,
    },
    DriverOutput, ResolvedFederate, ResolvedWorkspace,
};

/// A persistent Cargo crate containing one generated static Federate launcher.
pub struct GeneratedLauncher {
    /// Validated Cargo-native generated workspace and its locked short target.
    workspace: GeneratedWorkspace,
    /// Canonical application workspace used for Cargo configuration discovery.
    application_workspace: PathBuf,
    /// Cargo executable snapshotted for the complete generated-launcher request.
    cargo_program: OsString,
    /// Generated wrapper that adds the target facet after Cargo resolves compiler flags.
    compiler_wrapper: PathBuf,
    /// Effective configured compiler wrapper chained behind the facet wrapper.
    compiler_wrappers: crate::facet::CompilerWrappers,
    /// Cargo output policy forwarded to generated launcher commands.
    output: crate::CommandOutput,
    /// Exact generated root package selected by locked Cargo metadata.
    package_id: PackageId,
    /// Path to the generated Cargo manifest.
    manifest_path: PathBuf,
    /// Path to the generated Rust executable source.
    source_path: PathBuf,
    /// Path to the copied source workspace lockfile.
    lockfile_path: PathBuf,
    /// Host-verified payload compatibility inputs passed to Cargo.
    compile_inputs: Vec<(String, String)>,
    /// Canonical configured files and exact bytes included in the request identity.
    configured_files: ConfiguredFiles,
    /// Target and Cargo configuration selected for this Federate.
    federate: ResolvedFederate,
}

#[derive(Debug, Eq, PartialEq)]
/// Canonical configured-file paths and bytes retained for locked preflight validation.
struct ConfiguredFiles {
    /// Optional custom target specification included in launcher identity and Cargo arguments.
    target_json: Option<(PathBuf, Vec<u8>)>,
    /// Optional Cargo configuration included in launcher identity and Cargo arguments.
    cargo_config: Option<(PathBuf, Vec<u8>)>,
}

/// Host-process facilities selected by a configured runtime backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LauncherCapabilities {
    /// Whether generated code links hosted launcher support.
    hosted: bool,
}

/// Resolves launcher facilities from the runtime backend without target inference.
fn launcher_capabilities(runtime: &str) -> Result<LauncherCapabilities> {
    match runtime {
        "std" => Ok(LauncherCapabilities { hosted: true }),
        unsupported => bail!("unsupported runtime '{unsupported}'"),
    }
}

impl ConfiguredFiles {
    fn new(federate: &ResolvedFederate) -> Result<Self> {
        Ok(Self {
            target_json: configured_file(
                federate.target_json.as_deref(),
                "configured target JSON",
            )?,
            cargo_config: configured_file(
                federate.cargo_config.as_deref(),
                "configured Cargo configuration",
            )?,
        })
    }

    fn apply_canonical_paths(&self, federate: &mut ResolvedFederate) {
        federate.target_json = self.target_json.as_ref().map(|(path, _)| path.clone());
        federate.cargo_config = self.cargo_config.as_ref().map(|(path, _)| path.clone());
    }

    fn validate(&self) -> Result<()> {
        for (description, expected) in [
            ("configured target JSON", &self.target_json),
            ("configured Cargo configuration", &self.cargo_config),
        ] {
            let actual = configured_file(
                expected.as_ref().map(|(path, _)| path.as_path()),
                description,
            )?;
            ensure!(actual == *expected, "{description} changed");
        }
        Ok(())
    }
}

/// Successful offline build artifact for a generated static Federate launcher.
///
/// The executable is copied into an invocation-private directory before the cache lock is released.
pub struct BuiltLauncher {
    /// Keeps the copied executable directory alive for the caller's use.
    _private_directory: tempfile::TempDir,
    /// Invocation-private path to the verified generated executable.
    executable_path: PathBuf,
    /// Number of non-fresh Cargo compiler artifacts emitted by the build.
    compiled_artifacts: usize,
}

impl BuiltLauncher {
    /// Returns the invocation-private executable copied from the generated launcher build.
    pub fn executable_path(&self) -> &Path {
        &self.executable_path
    }

    /// Returns the number of non-fresh Cargo compiler artifacts from this build.
    pub const fn compiled_artifacts(&self) -> usize {
        self.compiled_artifacts
    }
}

impl GeneratedLauncher {
    /// Returns the generated launcher's Cargo manifest path.
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Returns the generated launcher's Rust source path.
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Returns the generated launcher's copied Cargo lockfile path.
    pub fn lockfile_path(&self) -> &Path {
        &self.lockfile_path
    }

    /// Builds the generated launcher offline with its locked dependency graph.
    pub fn build_locked_offline(&self) -> Result<BuiltLauncher> {
        self.with_locked_target(|target_dir| self.build_locked_offline_in(target_dir))
    }

    /// Builds and privately copies the launcher while the request target is locked.
    fn build_locked_offline_in(&self, target_dir: &Path) -> Result<BuiltLauncher> {
        let arguments = self.configured_arguments("build", target_dir, true);
        let output = self.cargo(arguments)?;
        require_success("locked offline launcher build", &output)?;
        let mut executable_paths = BTreeSet::new();
        let mut compiled_artifacts = 0;
        for message in Message::parse_stream(output.stdout.as_slice()) {
            let artifact = match message.context("failed to parse generated Cargo build message")? {
                Message::CompilerArtifact(artifact) => artifact,
                Message::TextLine(line) => {
                    bail!("generated Cargo build emitted non-JSON output: {line}");
                }
                _ => continue,
            };
            compiled_artifacts += usize::from(!artifact.fresh);
            if !artifact_matches(
                &artifact,
                &self.package_id,
                &self.manifest_path,
                "boomerang-static-launcher",
            )? {
                continue;
            }
            let executable = artifact.executable.ok_or_else(|| {
                anyhow!("generated launcher build emitted a binary without an executable")
            })?;
            if executable_paths
                .replace(executable.into_std_path_buf())
                .is_some()
            {
                bail!("generated launcher build produced multiple executable artifacts");
            }
        }
        if executable_paths.len() != 1 {
            bail!(
                "generated launcher build produced {} executable artifacts; expected exactly one",
                executable_paths.len()
            );
        }
        let executable = executable_paths
            .into_iter()
            .next()
            .expect("exactly one generated executable was required");
        let (_private_directory, executable_path) = copy_private_artifact(&executable, target_dir)?;
        Ok(BuiltLauncher {
            _private_directory,
            executable_path,
            compiled_artifacts,
        })
    }

    /// Checks the generated launcher offline with its reconciled lockfile locked.
    pub fn check_locked_offline(&self) -> Result<()> {
        self.with_locked_target(|target_dir| self.check_locked_offline_in(target_dir))
    }

    fn check_locked_offline_in(&self, target_dir: &Path) -> Result<()> {
        let arguments = self.configured_arguments("check", target_dir, false);
        let output = self.cargo(arguments)?;
        require_success("locked offline launcher check", &output)
    }

    /// Builds and executes the generated launcher offline with its reconciled lockfile locked.
    pub fn run_locked_offline(&self) -> Result<()> {
        self.with_locked_target(|target_dir| self.run_locked_offline_in(target_dir))
    }

    fn run_locked_offline_in(&self, target_dir: &Path) -> Result<()> {
        let arguments = self.configured_arguments("run", target_dir, false);
        let output = self.cargo(arguments)?;
        require_success("locked offline launcher execution", &output)
    }

    fn with_locked_target<T>(&self, operation: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        self.workspace.with_locked_target(|target_dir| {
            self.configured_files.validate()?;
            operation(target_dir)
        })
    }

    /// Builds configured arguments for one locked, offline launcher Cargo operation.
    fn configured_arguments(
        &self,
        operation: &str,
        target_directory: &Path,
        json_diagnostics: bool,
    ) -> Vec<OsString> {
        let mut arguments = Vec::new();
        if let Some(toolchain) = &self.federate.toolchain {
            arguments.push(format!("+{toolchain}").into());
        }
        arguments.extend([
            OsString::from(operation),
            OsString::from("--manifest-path"),
            self.manifest_path.as_os_str().to_owned(),
            OsString::from("--locked"),
            OsString::from("--offline"),
        ]);
        if json_diagnostics {
            arguments.push(OsString::from("--message-format=json-render-diagnostics"));
        }
        arguments.extend([
            OsString::from("--target-dir"),
            target_directory.as_os_str().to_owned(),
        ]);
        if let Some(target_json) = &self.federate.target_json {
            arguments.extend([
                OsString::from("--target"),
                configured_path_argument(target_json),
            ]);
        } else if let Some(target) = &self.federate.target {
            arguments.extend([OsString::from("--target"), OsString::from(target)]);
        } else {
            arguments.extend([
                OsString::from("--target"),
                OsString::from(target_lexicon::HOST.to_string()),
            ]);
        }
        if let Some(profile) = &self.federate.profile {
            arguments.extend([OsString::from("--profile"), OsString::from(profile)]);
        }
        if let Some(cargo_config) = &self.federate.cargo_config {
            arguments.extend([
                OsString::from("--config"),
                configured_path_argument(cargo_config),
            ]);
        }
        arguments
    }

    /// Runs one Cargo command against this generated manifest with compatibility inputs set.
    fn cargo(&self, arguments: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<Output> {
        let mut command = launcher_command(
            &self.cargo_program,
            &self.application_workspace,
            &self.compile_inputs,
            arguments,
            &self.compiler_wrapper,
            &self.compiler_wrappers,
        );
        self.output.configure(&mut command);
        let output = command
            .output()
            .context("failed to start generated Cargo command")?;
        self.output.forward_cargo_stderr(&output)?;
        Ok(output)
    }
}

/// Creates one launcher Cargo process with the selected executable and compatibility environment.
fn launcher_command(
    cargo_program: &OsStr,
    directory: &Path,
    compile_inputs: &[(String, String)],
    arguments: impl IntoIterator<Item = impl AsRef<OsStr>>,
    wrapper: &Path,
    configured: &crate::facet::CompilerWrappers,
) -> Command {
    let mut command = Command::new(cargo_program);
    command
        .current_dir(directory)
        .args(arguments)
        .envs(compile_inputs.iter().map(|(key, value)| (key, value)));
    crate::facet::Facet::Payload.configure(&mut command, wrapper, configured);
    command
}

/// Preserves a configured Cargo path as a native command-line argument.
fn configured_path_argument(path: &Path) -> OsString {
    path.as_os_str().to_owned()
}

/// Builds the configured Cargo arguments used to reconcile the generated lockfile.
fn configured_metadata_arguments(
    federate: &ResolvedFederate,
    manifest_path: &Path,
) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if let Some(toolchain) = &federate.toolchain {
        arguments.push(format!("+{toolchain}").into());
    }
    arguments.extend([
        OsString::from("metadata"),
        OsString::from("--manifest-path"),
        manifest_path.as_os_str().to_owned(),
        OsString::from("--format-version"),
        OsString::from("1"),
        OsString::from("--offline"),
    ]);
    if let Some(cargo_config) = &federate.cargo_config {
        arguments.extend([
            OsString::from("--config"),
            configured_path_argument(cargo_config),
        ]);
    }
    arguments
}

/// Collects Cargo-rendered compiler diagnostics from JSON message output.
pub(crate) fn rendered_compiler_diagnostics(stdout: &[u8]) -> Result<String> {
    let mut diagnostics = String::new();
    for message in Message::parse_stream(stdout) {
        match message.context("failed to parse generated Cargo build message")? {
            Message::CompilerMessage(message) => {
                if let Some(rendered) = message.message.rendered {
                    diagnostics.push_str(&rendered);
                }
            }
            Message::TextLine(line) => {
                bail!("generated Cargo build emitted non-JSON output: {line}");
            }
            _ => {}
        }
    }
    Ok(diagnostics)
}

/// Generates an isolated static Rust launcher for one Federate in a named deployment.
pub fn generate_launcher(
    workspace: impl AsRef<Path>,
    deployment_name: &str,
    federate_id: &str,
) -> Result<GeneratedLauncher> {
    let output = crate::CommandOutput::silent();
    let analyzed = analyze(workspace, deployment_name, &output)?;
    generate_analyzed_launcher(&analyzed, federate_id, &output)
}

/// Generates an isolated static Rust launcher from already completed deployment analysis.
pub(crate) fn generate_analyzed_launcher(
    analyzed: &AnalyzedDeployment,
    federate_id: &str,
    output: &crate::CommandOutput,
) -> Result<GeneratedLauncher> {
    let coordination = generated_coordination(analyzed)?;
    let federates = analyzed.compiled.federates();
    let (federate_index, federate) = federates
        .iter()
        .find(|(_, federate)| federate.id().as_str() == federate_id)
        .ok_or_else(|| {
            anyhow!(
                "deployment '{}' has no Federate '{federate_id}'",
                analyzed.resolved.deployment_name()
            )
        })?;
    let capabilities = launcher_capabilities(federate.runtime().as_str()).map_err(|_| {
        anyhow!(
            "Federate '{federate_id}' selects unsupported runtime '{}'",
            federate.runtime()
        )
    })?;
    let mut configuration = analyzed
        .resolved
        .deployment()
        .federates
        .get(federate_id)
        .cloned()
        .ok_or_else(|| {
            anyhow!("deployment has no resolved configuration for Federate '{federate_id}'")
        })?;
    let configured_files = ConfiguredFiles::new(&configuration)?;
    configured_files.apply_canonical_paths(&mut configuration);

    let slice = analyzed.compiled.federate_slice(federate_index)?;
    let distributed = coordination.is_some();
    let aliases = payload_aliases(&analyzed.resolved, &analyzed.driver, slice.enclaves())?;
    let manifest = render_manifest(&analyzed.resolved, &aliases, distributed, capabilities)?;
    let execution = analyzed
        .resolved
        .deployment()
        .execution
        .clone()
        .unwrap_or_default();
    let source = rust::render_launcher(
        &analyzed.driver,
        &slice,
        &aliases,
        &execution,
        rust::render_tracing_init(
            analyzed.resolved.deployment().tracing,
            configuration.bounded_tracing,
        ),
        coordination,
        capabilities,
    )?;
    let compile_inputs = payload_compile_inputs(&analyzed.resolved, &analyzed.driver, &aliases)?;
    prepare_launcher(
        analyzed,
        configuration,
        configured_files,
        manifest,
        source,
        compile_inputs,
        output,
    )
}

/// Reuses locked Cargo workspace validation for either kind of compiled executable.
#[allow(clippy::too_many_arguments, reason = "one generated workspace request")]
fn prepare_launcher(
    analyzed: &AnalyzedDeployment,
    configuration: ResolvedFederate,
    configured_files: ConfiguredFiles,
    manifest: String,
    source: String,
    compile_inputs: Vec<(String, String)>,
    output: &crate::CommandOutput,
) -> Result<GeneratedLauncher> {
    let application_workspace = analyzed
        .resolved
        .lockfile()
        .path
        .parent()
        .expect("canonical workspace lockfile has a parent")
        .to_path_buf();
    let cargo_program = generated_cargo_program();
    let compiler_wrapper = crate::driver::prepare_compiler_wrapper(&analyzed.resolved, output)?;
    let compiler_wrappers = crate::facet::CompilerWrappers::resolve(
        &application_workspace,
        configuration.cargo_config.as_deref(),
    )?;
    let identity = launcher_request_identity(
        manifest.as_bytes(),
        source.as_bytes(),
        &analyzed.resolved.lockfile().digest,
        &compile_inputs,
        &configuration,
        &configured_files,
        &cargo_program,
    )?;
    let request = GeneratedWorkspaceRequest {
        role: GeneratedRole::Launcher,
        identity,
        manifest: manifest.as_bytes(),
        source: source.as_bytes(),
        source_lockfile: &analyzed.resolved.lockfile().path,
        source_lock_digest: &analyzed.resolved.lockfile().digest,
    };
    let (workspace, package_id) = resolve_generated_workspace(
        analyzed.resolved.target_directory(),
        &request,
        |directory| {
            reconcile_launcher_lock(
                directory,
                &configuration,
                &compile_inputs,
                &application_workspace,
                &cargo_program,
                &compiler_wrapper,
                &compiler_wrappers,
                output,
            )
        },
        |directory| {
            validate_launcher_graph(
                directory,
                &configuration,
                &compile_inputs,
                &analyzed.resolved,
                &cargo_program,
                &compiler_wrapper,
                &compiler_wrappers,
                output,
            )
        },
    )?;
    let source_path = workspace.directory().join("src/main.rs");
    let lockfile_path = workspace.directory().join("Cargo.lock");
    let manifest_path = workspace.directory().join("Cargo.toml");

    Ok(GeneratedLauncher {
        workspace,
        application_workspace,
        cargo_program,
        compiler_wrapper,
        compiler_wrappers,
        output: *output,
        package_id,
        manifest_path,
        source_path,
        lockfile_path,
        compile_inputs,
        configured_files,
        federate: configuration,
    })
}

/// Computes the canonical identity of one launcher request from all Cargo-relevant inputs.
fn launcher_request_identity(
    manifest: &[u8],
    source: &[u8],
    source_lock_digest: &[u8; 32],
    compile_inputs: &[(String, String)],
    federate: &ResolvedFederate,
    configured_files: &ConfiguredFiles,
    cargo_program: &OsStr,
) -> Result<RequestIdentity> {
    let mut inputs = compile_inputs.to_vec();
    inputs.sort();

    let mut identity = RequestIdentityBuilder::new(GeneratedRole::Launcher);
    identity.field("facet", Some(b"payload"));
    identity.field("facet-wrapper", Some(include_bytes!("../facet_rustc.rs")));
    identity.field("manifest", Some(manifest));
    identity.field("source", Some(source));
    identity.field("source-lock-digest", Some(source_lock_digest));
    for (key, value) in inputs {
        identity.field("compile-input-key", Some(key.as_bytes()));
        identity.field("compile-input-value", Some(value.as_bytes()));
    }
    let effective_target = federate.target_json.is_none().then(|| {
        federate
            .target
            .clone()
            .unwrap_or_else(|| target_lexicon::HOST.to_string())
    });
    identity.field("target", effective_target.as_deref().map(str::as_bytes));
    identity.field("profile", federate.profile.as_deref().map(str::as_bytes));
    identity.field(
        "toolchain",
        federate.toolchain.as_deref().map(str::as_bytes),
    );
    for (label, file) in [
        ("target-json", &configured_files.target_json),
        ("cargo-config", &configured_files.cargo_config),
    ] {
        identity.field(
            &format!("{label}-path"),
            file.as_ref()
                .map(|(path, _)| path.as_os_str().as_encoded_bytes()),
        );
        identity.field(label, file.as_ref().map(|(_, bytes)| bytes.as_slice()));
    }
    identity.field("cargo-program", Some(cargo_program.as_encoded_bytes()));
    Ok(identity.finish())
}

/// Canonicalizes and reads an optional configured file for lossless request identity encoding.
fn configured_file(path: Option<&Path>, description: &str) -> Result<Option<(PathBuf, Vec<u8>)>> {
    path.map(|path| {
        let path = fs::canonicalize(path)
            .with_context(|| format!("failed to canonicalize {description} {}", path.display()))?;
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read {description} {}", path.display()))?;
        Ok((path, bytes))
    })
    .transpose()
}

/// Reconciles the copied source lockfile for one generated launcher without network access.
#[allow(
    clippy::too_many_arguments,
    reason = "launcher Cargo context is forwarded without lossy repacking"
)]
fn reconcile_launcher_lock(
    directory: &Path,
    federate: &ResolvedFederate,
    compile_inputs: &[(String, String)],
    application_workspace: &Path,
    cargo_program: &OsStr,
    wrapper: &Path,
    configured: &crate::facet::CompilerWrappers,
    progress: &crate::CommandOutput,
) -> Result<()> {
    let arguments = configured_metadata_arguments(federate, &directory.join("Cargo.toml"));
    let mut command = launcher_command(
        cargo_program,
        application_workspace,
        compile_inputs,
        arguments,
        wrapper,
        configured,
    );
    progress.configure(&mut command);
    let output = command
        .output()
        .context("failed to start generated Cargo metadata reconciliation")?;
    progress.forward_cargo_stderr(&output)?;
    require_success("lock reconciliation", &output)
}

/// Verifies the locked graph uses only source packages and controlled launcher dependencies.
#[allow(
    clippy::too_many_arguments,
    reason = "launcher Cargo context and graph expectations are independently validated"
)]
fn validate_launcher_graph(
    directory: &Path,
    federate: &ResolvedFederate,
    compile_inputs: &[(String, String)],
    resolved: &ResolvedWorkspace,
    cargo_program: &OsStr,
    wrapper: &Path,
    configured: &crate::facet::CompilerWrappers,
    progress: &crate::CommandOutput,
) -> Result<PackageId> {
    let capabilities = launcher_capabilities(federate.runtime.as_str())?;
    let application_workspace = resolved
        .lockfile()
        .path
        .parent()
        .expect("canonical workspace lockfile has a parent");
    let mut arguments = configured_metadata_arguments(federate, &directory.join("Cargo.toml"));
    arguments.push(OsString::from("--locked"));
    let mut command = launcher_command(
        cargo_program,
        application_workspace,
        compile_inputs,
        arguments,
        wrapper,
        configured,
    );
    progress.configure(&mut command);
    let output = command
        .output()
        .context("failed to start locked generated Cargo metadata validation")?;
    progress.forward_cargo_stderr(&output)?;
    require_success("locked metadata verification", &output)?;
    let metadata: Metadata = serde_json::from_slice(&output.stdout)
        .context("failed to decode generated Cargo metadata")?;
    let root = metadata
        .root_package()
        .ok_or_else(|| anyhow!("generated launcher metadata has no root package"))?;
    let graph = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| anyhow!("generated launcher metadata has no resolve graph"))?;
    let root_node = graph
        .nodes
        .iter()
        .find(|node| node.id == root.id)
        .expect("Cargo resolve graph contains its root");
    let direct_package = |name: &str| {
        root_node
            .deps
            .iter()
            .map(|dependency| &dependency.pkg)
            .find(|package| {
                metadata
                    .packages
                    .iter()
                    .any(|candidate| candidate.id == **package && candidate.name == name)
            })
    };
    // The generated manifest owns these direct dependencies and therefore their complete closures.
    let mut launcher_dependencies = BTreeSet::new();
    let mut pending = Vec::new();
    if capabilities.hosted {
        let launcher_support = direct_package("boomerang_util").ok_or_else(|| {
            anyhow!("generated hosted launcher does not depend directly on boomerang_util")
        })?;
        pending.push(launcher_support.clone());
    }
    if let Some(wire) = direct_package("boomerang_federated") {
        pending.push(wire.clone());
    }
    if let Some(backend) = direct_package("boomerang_central_rti") {
        pending.push(backend.clone());
    }
    while let Some(package) = pending.pop() {
        if !launcher_dependencies.insert(package.clone()) {
            continue;
        }
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == package)
            .expect("Cargo resolve graph contains every dependency");
        pending.extend(node.deps.iter().map(|dependency| dependency.pkg.clone()));
    }
    for node in graph.nodes.iter().filter(|node| node.id != root.id) {
        let id = node.id.to_string();
        if !resolved.locked_package_ids().contains(&id) && !launcher_dependencies.contains(&node.id)
        {
            bail!("generated launcher package {id} was absent from source metadata");
        }
    }
    Ok(root.id.clone())
}

/// Maps selected implementation package identities to deterministic generated crate aliases.
fn payload_aliases(
    resolved: &ResolvedWorkspace,
    driver: &DriverOutput,
    enclaves: &[boomerang_builder::compiler::OwnedEnclaveImage],
) -> Result<BTreeMap<String, String>> {
    let required = enclaves
        .iter()
        .flat_map(|enclave| enclave.required_bindings().iter())
        .map(binding_implementation)
        .collect::<BTreeSet<_>>();
    let mut aliases = BTreeMap::new();
    let mut package_aliases = BTreeMap::new();
    for binding in driver.bindings() {
        let implementation = binding.implementation().as_str();
        if required.contains(implementation) && !aliases.contains_key(implementation) {
            let (package, selection) =
                resolved.implementation(implementation).ok_or_else(|| {
                    anyhow!("selected implementation '{implementation}' was not resolved")
                })?;
            let next = format!("implementation_{}", package_aliases.len());
            let alias = package_aliases.entry(package.name.clone()).or_insert(next);
            aliases.insert(implementation.to_owned(), selection.exported_path(alias));
        }
    }
    if aliases.len() != required.len() {
        bail!("compiled Federate contains an implementation absent from descriptor output");
    }
    Ok(aliases)
}

/// Returns the selected implementation identity carried by a required payload binding.
fn binding_implementation(binding: &boomerang_builder::compiler::RequiredBinding) -> &str {
    use boomerang_builder::compiler::RequiredBinding;
    match binding {
        RequiredBinding::State { implementation, .. }
        | RequiredBinding::Reaction { implementation, .. }
        | RequiredBinding::Port { implementation, .. }
        | RequiredBinding::Action { implementation, .. } => implementation.as_str(),
    }
}

fn selected_payload_features(
    bindings: &BTreeMap<String, crate::manifest::Binding>,
    aliases: &BTreeMap<String, String>,
    package: &str,
) -> Vec<String> {
    let mut features = bindings
        .values()
        .filter(|binding| {
            binding.package == package && aliases.contains_key(&binding.implementation_id())
        })
        .flat_map(|binding| binding.features.iter().cloned())
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    features
}

/// Renders the standalone launcher manifest with runtime, tracing, and selected payload packages.
fn render_manifest(
    resolved: &ResolvedWorkspace,
    aliases: &BTreeMap<String, String>,
    distributed: bool,
    capabilities: LauncherCapabilities,
) -> Result<String> {
    let mut dependencies = BTreeMap::new();
    dependencies.insert(
        String::from("boomerang_runtime"),
        dependency(resolved.runtime(), false, Vec::new())?,
    );
    dependencies.insert(
        String::from("boomerang_federated"),
        runtime_sibling_dependency(resolved.runtime(), "boomerang_federated", Vec::new())?,
    );
    dependencies.insert(
        String::from("tinymap"),
        dependency(resolved.table_store(), false, Vec::new())?,
    );
    if capabilities.hosted {
        dependencies.insert(
            String::from("boomerang_util"),
            runtime_sibling_dependency(
                resolved.runtime(),
                "boomerang_util",
                vec![String::from(match resolved.deployment().tracing {
                    crate::manifest::TracingBackend::Off => "launcher",
                    crate::manifest::TracingBackend::Bounded => "bounded-tracing",
                    crate::manifest::TracingBackend::Hosted => "hosted-tracing",
                })],
            )?,
        );
    }
    if distributed {
        dependencies.insert(
            String::from("boomerang_central_rti"),
            runtime_sibling_dependency(
                resolved.runtime(),
                "boomerang_central_rti",
                if capabilities.hosted
                    && resolved.deployment().tracing == crate::manifest::TracingBackend::Bounded
                {
                    vec![String::from("bounded-tracing")]
                } else {
                    Vec::new()
                },
            )?,
        );
    }
    for (implementation, path) in aliases {
        let (package, _) = resolved
            .implementation(implementation)
            .expect("payload alias requires resolved selection");
        let mut features =
            selected_payload_features(&resolved.deployment().bindings, aliases, &package.name);
        features.sort();
        features.dedup();
        // Generated aliases contain a validated crate identifier followed by an
        // optional validated component path. One dependency per Cargo package.
        let alias = path.split("::").next().expect("generated crate alias");
        dependencies.insert(alias.to_owned(), dependency(package, false, features)?);
    }
    let package = toml::Table::from_iter([
        ("name".into(), "boomerang-static-launcher".into()),
        ("version".into(), "0.0.0".into()),
        ("edition".into(), "2021".into()),
        ("publish".into(), false.into()),
    ]);
    toml::to_string(&toml::Table::from_iter([
        ("package".into(), package.into()),
        (
            "dependencies".into(),
            dependencies.into_iter().collect::<toml::Table>().into(),
        ),
        ("workspace".into(), toml::Table::new().into()),
    ]))
    .map_err(anyhow::Error::from)
}

/// Computes the exact environment consumed while expanding selected payload facets.
fn payload_compile_inputs(
    resolved: &ResolvedWorkspace,
    driver: &DriverOutput,
    aliases: &BTreeMap<String, String>,
) -> Result<Vec<(String, String)>> {
    let mut inputs = vec![(
        PAYLOAD_MACRO_ABI_COMPILE_INPUT.to_owned(),
        boomerang_runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI.to_string(),
    )];
    let mut component_inputs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for binding in driver
        .bindings()
        .iter()
        .filter(|binding| aliases.contains_key(binding.implementation().as_str()))
    {
        let (package, selection) = resolved
            .implementation(binding.implementation().as_str())
            .expect("descriptor implementation package is resolved");
        let manifest_dir = package
            .manifest_path
            .parent()
            .expect("package manifest has a parent");
        let manifest_dir = fs::canonicalize(manifest_dir)?;
        let manifest_dir = manifest_dir
            .to_str()
            .ok_or_else(|| anyhow!("payload package path is not valid UTF-8"))?;
        let descriptor = binding.descriptor();
        let fingerprint = descriptor.descriptor_fingerprint_input().fingerprint();
        let fingerprint = hex(&fingerprint.to_bytes());
        for reactor in descriptor
            .reactor_slots()
            .iter()
            .filter(|reactor| reactor.parent.is_none())
        {
            if selection.component.is_some() {
                let library = package
                    .lib_target
                    .as_deref()
                    .expect("selected package library");
                let key =
                    boomerang_runtime::binding::component_payload_fingerprint_compile_inputs_key(
                        manifest_dir,
                        descriptor.contract_id().as_str(),
                        descriptor.contract_version(),
                        &reactor.id.to_string(),
                    );
                let module = selection.exported_path(library).replace("r#", "");
                component_inputs
                    .entry(key)
                    .or_default()
                    .insert(module, fingerprint.clone());
            } else {
                let key = payload_fingerprint_compile_input_key(
                    manifest_dir,
                    descriptor.contract_id().as_str(),
                    descriptor.contract_version(),
                    &reactor.id.to_string(),
                );
                inputs.push((key, fingerprint.clone()));
            }
        }
    }
    inputs.extend(component_inputs.into_iter().map(|(key, records)| {
        (
            key,
            records
                .into_iter()
                .map(|(module, fingerprint)| format!("{module}={fingerprint}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }));
    Ok(inputs)
}

/// Encodes bytes as canonical lowercase hexadecimal text.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing into a String cannot fail");
    }
    output
}

/// Converts unsuccessful generated Cargo execution into a diagnostic preserving stderr.
fn require_success(phase: &'static str, output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let mut diagnostics = rendered_compiler_diagnostics(output.stdout.as_slice())?;
    if !diagnostics.is_empty() && !diagnostics.ends_with('\n') {
        diagnostics.push('\n');
    }
    diagnostics.push_str(&String::from_utf8_lossy(&output.stderr));
    bail!("generated launcher {phase} failed:\n{}", diagnostics)
}

/// Bundle protocol identity for the canonical framed channel used by hosted deployments.
pub(crate) const HOSTED_PROTOCOL: &str = "boomerang.canonical.v1";

/// Validates the selected hosted projection before writing any generated source.
fn validate_coordination(analyzed: &AnalyzedDeployment) -> Result<bool> {
    use boomerang_runtime::image::*;
    for federate in analyzed.compiled.federates().values() {
        launcher_capabilities(federate.runtime().as_str())?;
    }
    analyzed.compiled.with_coordination(|projection| {
        let image = match projection {
            CoordinationProjection::Local => return Ok(false),
            CoordinationProjection::CentralRti(image) => image,
        };
        for (key, _) in image.routes().iter() {
            if image.route_transport_capability(key) != "tcp"
                || image.route_codec_capability(key) != "serde-json"
                || image.route_transport_policy(key) != TransportPolicy::ReliableOrderedFramed
                || image.route_codec_policy(key) != CodecPolicy::CanonicalBounded
                || image.route_failure_policy(key) != BoundaryFailurePolicy::PropagateStop
                || image.route_timing_policy(key) != TimingPolicy::BestEffort
                || image.route_security_policy(key) != SecurityPolicy::None
            {
                bail!(
                    "unsupported generated central-rti boundary configuration for '{}'",
                    image.route_boundary(key).as_str()
                );
            }
        }
        for (key, _) in analyzed.compiled.federates().iter() {
            if image.member_recovery_policy(key) != RecoveryPolicy::FailStop {
                bail!("unsupported generated central-rti recovery policy");
            }
        }
        Ok(true)
    })
}

/// Hashes shared boundary contracts and canonical coordination semantics with a protocol domain.
pub(crate) fn coordination_identity(analyzed: &AnalyzedDeployment) -> Result<Option<blake3::Hash>> {
    if !validate_coordination(analyzed)? {
        return Ok(None);
    }
    fingerprints::coordination(&analyzed.compiled, analyzed.driver.topology()).map(Some)
}

/// Computes the local image claim from its actual typed Federate slice.
pub(crate) fn federate_image_fingerprint(
    analyzed: &AnalyzedDeployment,
    federate: boomerang_runtime::image::FederateIndex,
) -> Result<blake3::Hash> {
    fingerprints::federate_image(
        &analyzed.compiled.federate_slice(federate)?,
        analyzed.driver.bindings(),
    )
}

/// Records the baseline canonical protocol independently of the current hosted transport.
pub(crate) fn wire_profile(
    analyzed: &AnalyzedDeployment,
) -> Result<Option<crate::bundle::WireProfileDocument>> {
    if !validate_coordination(analyzed)? {
        return Ok(None);
    }
    use boomerang_federated::wire::*;
    Ok(Some(crate::bundle::WireProfileDocument {
        protocol: PROTOCOL_VERSION,
        codec: CODEC_VERSION,
        max_payload_bytes: MAX_PAYLOAD_BYTES,
        mapping: fingerprints::mapping(&analyzed.compiled)?
            .to_hex()
            .to_string(),
    }))
}

/// Emits the shared immutable projection, portable admission contract, and codec profile.
fn generated_coordination(
    analyzed: &AnalyzedDeployment,
) -> Result<Option<proc_macro2::TokenStream>> {
    let Some(identity) = coordination_identity(analyzed)? else {
        return Ok(None);
    };
    let image = rti::render_coordination(&analyzed.compiled)?;
    let bytes = identity.as_bytes().iter();
    let mapping = fingerprints::mapping(&analyzed.compiled)?;
    let mapping_bytes = mapping.as_bytes().iter();
    Ok(Some(quote::quote! {
        #image
        /// Shared semantic compatibility claim; local image and artifact claims are distinct.
        pub const WIRE_COORDINATION_FINGERPRINT: boomerang_federated::wire::CoordinationFingerprint =
            boomerang_federated::wire::CoordinationFingerprint::new([#(#bytes),*]);
        const COORDINATION_IDENTITY: boomerang_central_rti::compiled::CoordinationIdentity =
            WIRE_COORDINATION_FINGERPRINT;
        /// Exact dense table mapping shared by the closed roster.
        pub const WIRE_MAPPING: [u8; 32] = [#(#mapping_bytes),*];
        /// Baseline canonical codec with the declared route profile's encoded-message bound.
        pub type WirePayloadCodec<T> = boomerang_federated::wire::PostcardCodec<T, {boomerang_federated::wire::MAX_PAYLOAD_BYTES}>;
        /// Binds portable admission directly to the generated typed RTI image.
        pub fn wire_contract() -> boomerang_federated::wire::Contract<'static, FederateIndex, RtiRouteIndex, RtiRouteImage<'static>> {
            boomerang_federated::wire::Contract::new(WIRE_COORDINATION_FINGERPRINT, WIRE_MAPPING,
                COORDINATION_MEMBERS, *COORDINATION_IMAGE.routes(), |route| (route.source(), route.target()))
        }
        /// Creates the exact baseline handshake for a member of the generated roster.
        pub fn wire_handshake(member: FederateIndex) -> Option<boomerang_federated::wire::Handshake<'static>> {
            use boomerang_federated::wire::*;
            COORDINATION_MEMBERS.get(member).map(|member| Handshake {
                protocol: PROTOCOL_VERSION, codec: CODEC_VERSION, coordination: WIRE_COORDINATION_FINGERPRINT,
                epoch: 0, incarnation: 0, mapping: WIRE_MAPPING, member,
            })
        }
    }))
}

/// Generates the payload-free coordinator only when canonical lowering selects central RTI.
pub(crate) fn generate_analyzed_rti(
    analyzed: &AnalyzedDeployment,
    output: &crate::CommandOutput,
) -> Result<Option<GeneratedLauncher>> {
    let Some(coordination) = generated_coordination(analyzed)? else {
        return Ok(None);
    };
    let selected = analyzed.resolved.deployment().rti.as_ref();
    let configuration = ResolvedFederate {
        bounded_tracing: analyzed
            .resolved
            .deployment()
            .bounded_tracing_limits(selected.and_then(|rti| rti.bounded_tracing.as_ref())),
        groups: Vec::new(),
        target: selected.map(|rti| rti.target.clone()),
        toolchain: None,
        profile: selected.and_then(|rti| rti.profile.clone()),
        runtime: String::from("std"),
        recovery: boomerang_runtime::image::RecoveryPolicy::FailStop,
        target_json: None,
        cargo_config: None,
    };
    let aliases = BTreeMap::new();
    let manifest = render_manifest(
        &analyzed.resolved,
        &aliases,
        true,
        LauncherCapabilities { hosted: true },
    )?;
    let init_tracing = rust::render_tracing_init(
        analyzed.resolved.deployment().tracing,
        configuration.bounded_tracing,
    );
    let source = rust::format_rust(quote::quote! {
        use boomerang_runtime::image::*;
        use tinymap::{TinyMapView, SliceRange};
        #coordination
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            use std::io::Write;
            #init_tracing
            let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS)?;
            let rti = boomerang_central_rti::compiled::CompiledRti::from_image(view, COORDINATION_IDENTITY)?;
            let bind = std::env::var("BOOMERANG_RTI_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());
            let listener = std::net::TcpListener::bind(bind)?;
            if let Ok(ready) = std::env::var("BOOMERANG_RTI_READY_ADDRESS") {
                let ready: std::net::SocketAddr = ready.parse()?;
                let mut ready = std::net::TcpStream::connect_timeout(&ready, std::time::Duration::from_secs(10))?;
                ready.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;
                writeln!(ready, "BOOMERANG_RTI_READY_V1 {}", listener.local_addr()?)?;
            } else {
                println!("BOOMERANG_RTI_READY_V1 {}", listener.local_addr()?);
            }
            boomerang_central_rti::compiled::hosted::Server::new(listener, rti, wire_contract(), std::time::Duration::from_secs(10))?.serve()?;
            Ok(())
        }
    })?;
    let configured_files = ConfiguredFiles::new(&configuration)?;
    prepare_launcher(
        analyzed,
        configuration,
        configured_files,
        manifest,
        source,
        Vec::new(),
        output,
    )
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::{
        configured_metadata_arguments, configured_path_argument, launcher_capabilities,
        launcher_command, launcher_request_identity, rendered_compiler_diagnostics,
        selected_payload_features, ConfiguredFiles,
    };
    use crate::{manifest::Binding, RecoveryPolicy, ResolvedFederate};
    use std::{collections::BTreeMap, ffi::OsStr, path::Path};

    #[test]
    fn payload_features_include_only_named_components_selected_for_this_federate() {
        let bindings = BTreeMap::from([
            (
                "local".into(),
                Binding {
                    package: "components".into(),
                    component: Some("local".into()),
                    features: vec!["local-target".into()],
                },
            ),
            (
                "remote".into(),
                Binding {
                    package: "components".into(),
                    component: Some("remote".into()),
                    features: vec!["remote-target".into()],
                },
            ),
        ]);
        let aliases =
            BTreeMap::from([("components::local".into(), "implementation_0::local".into())]);
        assert_eq!(
            selected_payload_features(&bindings, &aliases, "components"),
            ["local-target"]
        );
    }

    #[test]
    fn launcher_capabilities_depend_only_on_the_runtime_backend() {
        assert!(launcher_capabilities("std").unwrap().hosted);
        assert!(launcher_capabilities("embedded").is_err());
    }

    #[test]
    fn metadata_reconciliation_preserves_federate_toolchain_and_cargo_config() {
        let federate = ResolvedFederate {
            bounded_tracing: None,
            groups: Vec::new(),
            target: None,
            toolchain: Some(String::from("nightly-test")),
            profile: None,
            runtime: String::from("std"),
            recovery: RecoveryPolicy::FailStop,
            target_json: None,
            cargo_config: Some(std::path::PathBuf::from("/tmp/cargo-config.toml")),
        };
        let arguments = configured_metadata_arguments(&federate, Path::new("Cargo.toml"));
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "+nightly-test",
                "metadata",
                "--manifest-path",
                "Cargo.toml",
                "--format-version",
                "1",
                "--offline",
                "--config",
                "/tmp/cargo-config.toml"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn configured_path_argument_preserves_native_bytes() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/cargo-config-\x80.toml".to_vec(),
        ));
        assert_eq!(configured_path_argument(&path), path.as_os_str());
    }

    #[test]
    fn launcher_request_identity_tracks_configured_files_and_cargo_program() {
        let [target_a, target_b, config_a, config_b] =
            std::array::from_fn(|_| tempfile::NamedTempFile::new().unwrap());
        for path in [target_a.path(), target_b.path()] {
            std::fs::write(path, b"{}\n").unwrap();
        }
        for path in [config_a.path(), config_b.path()] {
            std::fs::write(path, b"[net]\noffline = true\n").unwrap();
        }
        let inputs = vec![(String::from("COMPATIBILITY"), String::from("fixed"))];
        let identity = |target_json: &Path, cargo_config: &Path, cargo| {
            let federate = ResolvedFederate {
                bounded_tracing: None,
                groups: Vec::new(),
                target: None,
                toolchain: None,
                profile: None,
                runtime: String::from("std"),
                recovery: RecoveryPolicy::FailStop,
                target_json: Some(target_json.to_path_buf()),
                cargo_config: Some(cargo_config.to_path_buf()),
            };
            let configured_files = ConfiguredFiles::new(&federate).unwrap();
            launcher_request_identity(
                b"manifest",
                b"source",
                &[7; 32],
                &inputs,
                &federate,
                &configured_files,
                OsStr::new(cargo),
            )
            .unwrap()
        };
        let first = identity(target_a.path(), config_a.path(), "cargo");

        std::fs::write(config_a.path(), b"[net]\noffline = false\n").unwrap();
        assert_ne!(first, identity(target_a.path(), config_a.path(), "cargo"));
        std::fs::write(config_a.path(), b"[net]\noffline = true\n").unwrap();
        assert_ne!(first, identity(target_b.path(), config_a.path(), "cargo"));
        assert_ne!(first, identity(target_a.path(), config_b.path(), "cargo"));
        assert_ne!(
            first,
            identity(target_a.path(), config_a.path(), "custom-cargo")
        );
    }

    #[test]
    fn launcher_request_identity_normalizes_an_implicit_host_target() {
        let identity = |target: Option<String>| {
            let federate = ResolvedFederate {
                bounded_tracing: None,
                groups: Vec::new(),
                target,
                toolchain: None,
                profile: None,
                runtime: String::from("std"),
                recovery: RecoveryPolicy::FailStop,
                target_json: None,
                cargo_config: None,
            };
            let configured_files = ConfiguredFiles::new(&federate).unwrap();
            launcher_request_identity(
                b"manifest",
                b"source",
                &[7; 32],
                &[],
                &federate,
                &configured_files,
                OsStr::new("cargo"),
            )
            .unwrap()
        };

        assert_eq!(
            identity(None),
            identity(Some(target_lexicon::HOST.to_string()))
        );
    }

    #[test]
    fn rendered_compiler_diagnostics_preserve_cargo_json_output() {
        let output = br#"{"reason":"compiler-message","package_id":"test","manifest_path":"/tmp/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/tmp/lib.rs","edition":"2021","doc":false,"doctest":false,"test":false},"message":{"message":"intentional target payload build failure","code":null,"level":"error","spans":[],"children":[],"rendered":"error: intentional target payload build failure\\n"}}
"#;
        assert!(rendered_compiler_diagnostics(output)
            .unwrap()
            .contains("intentional target payload build failure"));
    }

    #[test]
    fn rendered_compiler_diagnostics_reject_malformed_cargo_json() {
        assert!(rendered_compiler_diagnostics(b"this is not Cargo JSON\n").is_err());
    }

    #[test]
    fn generated_cargo_uses_the_runtime_override() {
        let command = launcher_command(
            OsStr::new("custom-cargo"),
            Path::new("."),
            &[],
            std::iter::empty::<&OsStr>(),
            Path::new("facet-wrapper"),
            &crate::facet::CompilerWrappers::default(),
        );
        assert_eq!(command.get_program(), "custom-cargo");
    }
}
