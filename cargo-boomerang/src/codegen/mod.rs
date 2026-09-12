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

mod rust;

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
) -> Command {
    let mut command = Command::new(cargo_program);
    command
        .current_dir(directory)
        .args(arguments)
        .envs(compile_inputs.iter().map(|(key, value)| (key, value)));
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
    let distributed = federates.len() > 1;
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
        distributed,
        capabilities,
    )?;
    let compile_inputs = payload_compile_inputs(&analyzed.resolved, &analyzed.driver, &aliases)?;
    let application_workspace = analyzed
        .resolved
        .lockfile()
        .path
        .parent()
        .expect("canonical workspace lockfile has a parent")
        .to_path_buf();
    let cargo_program = generated_cargo_program();
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
                output,
            )
        },
        |directory| {
            validate_launcher_graph(
                directory,
                &configuration,
                &compile_inputs,
                &aliases,
                &analyzed.resolved,
                capabilities,
                &cargo_program,
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
fn reconcile_launcher_lock(
    directory: &Path,
    federate: &ResolvedFederate,
    compile_inputs: &[(String, String)],
    application_workspace: &Path,
    cargo_program: &OsStr,
    progress: &crate::CommandOutput,
) -> Result<()> {
    let arguments = configured_metadata_arguments(federate, &directory.join("Cargo.toml"));
    let mut command = launcher_command(
        cargo_program,
        application_workspace,
        compile_inputs,
        arguments,
    );
    progress.configure(&mut command);
    let output = command
        .output()
        .context("failed to start generated Cargo metadata reconciliation")?;
    progress.forward_cargo_stderr(&output)?;
    require_success("lock reconciliation", &output)
}

/// Verifies the locked graph uses only source packages and controlled launcher dependencies.
fn validate_launcher_graph(
    directory: &Path,
    federate: &ResolvedFederate,
    compile_inputs: &[(String, String)],
    aliases: &BTreeMap<String, String>,
    resolved: &ResolvedWorkspace,
    capabilities: LauncherCapabilities,
    cargo_program: &OsStr,
    progress: &crate::CommandOutput,
) -> Result<PackageId> {
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
    let implementation_ids = resolved
        .deployment()
        .bindings
        .values()
        .map(|binding| {
            &resolved
                .package(&binding.package)
                .expect("resolved implementation package is retained")
                .id
        })
        .collect::<BTreeSet<_>>();
    let selected_ids = aliases
        .keys()
        .map(|implementation| {
            &resolved
                .package(implementation)
                .expect("selected implementation package is retained")
                .id
        })
        .collect::<BTreeSet<_>>();
    for node in graph.nodes.iter().filter(|node| node.id != root.id) {
        let id = node.id.to_string();
        if node
            .features
            .iter()
            .any(|feature| *feature == "__boomerang_payload")
            && implementation_ids.contains(&node.id)
            && !selected_ids.contains(&node.id)
        {
            bail!("unselected implementation package {id} activates reserved payload facet");
        }
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
    for binding in driver.bindings() {
        let implementation = binding.implementation().as_str();
        if required.contains(implementation) && !aliases.contains_key(implementation) {
            let alias = format!("implementation_{}", aliases.len());
            if resolved.package(implementation).is_none() {
                bail!("selected implementation package '{implementation}' was not resolved");
            }
            aliases.insert(implementation.to_owned(), alias);
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
        String::from("tinymap"),
        dependency(resolved.table_store(), false, Vec::new())?,
    );
    if capabilities.hosted {
        dependencies.insert(
            String::from("boomerang_util"),
            runtime_sibling_dependency(
                resolved.runtime(),
                "boomerang_util",
                vec![String::from("launcher")],
            )?,
        );
    }
    if distributed {
        dependencies.insert(
            String::from("boomerang_central_rti"),
            runtime_sibling_dependency(resolved.runtime(), "boomerang_central_rti", Vec::new())?,
        );
    }
    for (implementation, alias) in aliases {
        let package = resolved
            .package(implementation)
            .expect("payload alias requires a resolved package");
        let mut features = resolved
            .deployment()
            .bindings
            .values()
            .filter(|binding| binding.package == *implementation)
            .flat_map(|binding| binding.features.iter().cloned())
            .collect::<Vec<_>>();
        features.push(String::from("__boomerang_payload"));
        features.sort();
        features.dedup();
        dependencies.insert(alias.clone(), dependency(package, false, features)?);
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
    for binding in driver
        .bindings()
        .iter()
        .filter(|binding| aliases.contains_key(binding.implementation().as_str()))
    {
        let package = resolved
            .package(binding.implementation().as_str())
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
            let key = payload_fingerprint_compile_input_key(
                manifest_dir,
                descriptor.contract_id().as_str(),
                descriptor.contract_version(),
                &reactor.id.to_string(),
            );
            inputs.push((key, fingerprint.clone()));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        configured_metadata_arguments, configured_path_argument, launcher_capabilities,
        launcher_command, launcher_request_identity, rendered_compiler_diagnostics,
        ConfiguredFiles,
    };
    use crate::{RecoveryPolicy, ResolvedFederate};
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn launcher_capabilities_depend_only_on_the_runtime_backend() {
        assert!(launcher_capabilities("std").unwrap().hosted);
        assert!(launcher_capabilities("embedded").is_err());
    }

    #[test]
    fn metadata_reconciliation_preserves_federate_toolchain_and_cargo_config() {
        let federate = ResolvedFederate {
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
        );
        assert_eq!(command.get_program(), "custom-cargo");
    }
}
