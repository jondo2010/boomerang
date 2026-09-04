//! Cached static launcher generation for one compiled Federate.

mod rust;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{anyhow, bail, Context, Result};
use boomerang_runtime::binding::{
    payload_fingerprint_compile_input_key, PAYLOAD_MACRO_ABI_COMPILE_INPUT,
};
use cargo_metadata::Message;

use crate::{
    bundle::rename_noreplace,
    check::{analyze, AnalyzedDeployment},
    generated::dependency,
    DriverOutput, ResolvedFederate, ResolvedWorkspace,
};

/// Cache format version included in both the path and canonical cache identity.
const GENERATED_LAUNCHER_CACHE_VERSION: &str = "1";

/// A persistent Cargo crate containing one generated static Federate launcher.
pub struct GeneratedLauncher {
    /// Stable generated-workspace cache directory under the resolved Cargo target tree.
    directory: PathBuf,
    /// Path to the generated Cargo manifest.
    manifest_path: PathBuf,
    /// Path to the generated Rust executable source.
    source_path: PathBuf,
    /// Path to the copied source workspace lockfile.
    lockfile_path: PathBuf,
    /// Host-verified payload compatibility inputs passed to Cargo.
    compile_inputs: Vec<(String, String)>,
    /// Target and Cargo configuration selected for this Federate.
    federate: ResolvedFederate,
}

/// Successful offline build artifact for a generated static Federate launcher.
///
/// The executable path is stored in the persistent generated-workspace cache.
pub struct BuiltLauncher {
    executable_path: PathBuf,
}

impl BuiltLauncher {
    /// Returns the canonical executable produced by the generated launcher build.
    pub fn executable_path(&self) -> &Path {
        &self.executable_path
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
        let target_dir = self.directory.join("target");
        let target_dir_argument = target_dir
            .to_str()
            .ok_or_else(|| anyhow!("generated target path is not valid UTF-8"))?;
        let mut arguments = Vec::new();
        if let Some(toolchain) = &self.federate.toolchain {
            arguments.push(format!("+{toolchain}").into());
        }
        arguments.extend([
            OsString::from("build"),
            OsString::from("--manifest-path"),
            self.manifest_path.as_os_str().to_owned(),
            OsString::from("--locked"),
            OsString::from("--offline"),
            OsString::from("--message-format=json-render-diagnostics"),
            OsString::from("--target-dir"),
            OsString::from(target_dir_argument),
        ]);
        if let Some(target_json) = &self.federate.target_json {
            arguments.extend([
                OsString::from("--target"),
                configured_path_argument(target_json, "configured target JSON")?,
            ]);
        } else if let Some(target) = &self.federate.target {
            arguments.extend([OsString::from("--target"), OsString::from(target)]);
        }
        if let Some(profile) = &self.federate.profile {
            arguments.extend([OsString::from("--profile"), OsString::from(profile)]);
        }
        if let Some(cargo_config) = &self.federate.cargo_config {
            arguments.extend([
                OsString::from("--config"),
                configured_path_argument(cargo_config, "configured Cargo configuration")?,
            ]);
        }

        let output = self.cargo(arguments)?;
        require_success("locked offline launcher build", &output)?;
        let canonical_target_dir = fs::canonicalize(&target_dir)
            .with_context(|| format!("failed to canonicalize {}", target_dir.display()))?;
        let mut executable_paths = BTreeSet::new();
        for message in Message::parse_stream(output.stdout.as_slice()) {
            let artifact = match message.context("failed to parse generated Cargo build message")? {
                Message::CompilerArtifact(artifact) => artifact,
                Message::TextLine(line) => {
                    bail!("generated Cargo build emitted non-JSON output: {line}");
                }
                _ => continue,
            };
            if !same_manifest_identity(&self.manifest_path, artifact.manifest_path.as_std_path())?
                || !artifact
                    .target
                    .kind
                    .contains(&cargo_metadata::TargetKind::Bin)
            {
                continue;
            }
            let executable = artifact.executable.ok_or_else(|| {
                anyhow!("generated launcher build emitted a binary without an executable")
            })?;
            let executable = fs::canonicalize(executable.as_std_path()).with_context(|| {
                format!("failed to canonicalize generated executable {executable}")
            })?;
            if !fs::metadata(&executable)
                .with_context(|| {
                    format!(
                        "failed to inspect generated executable {}",
                        executable.display()
                    )
                })?
                .is_file()
            {
                bail!(
                    "generated executable {} is not a regular file",
                    executable.display()
                );
            }
            if !executable.starts_with(&canonical_target_dir) {
                bail!(
                    "generated executable {} is outside isolated target directory {}",
                    executable.display(),
                    canonical_target_dir.display()
                );
            }
            executable_paths.insert(executable);
        }
        if executable_paths.len() != 1 {
            bail!(
                "generated launcher build produced {} executable artifacts; expected exactly one",
                executable_paths.len()
            );
        }
        let executable_path = executable_paths
            .into_iter()
            .next()
            .expect("exactly one generated executable was required");
        Ok(BuiltLauncher { executable_path })
    }

    /// Checks the generated launcher offline with its reconciled lockfile locked.
    pub fn check_locked_offline(&self) -> Result<()> {
        let target_dir = self.directory.join("target");
        let output = self.cargo(vec![
            OsString::from("check"),
            OsString::from("--manifest-path"),
            self.manifest_path.as_os_str().to_owned(),
            OsString::from("--locked"),
            OsString::from("--offline"),
            OsString::from("--target-dir"),
            target_dir
                .to_str()
                .ok_or_else(|| anyhow!("generated target path is not valid UTF-8"))?
                .into(),
        ])?;
        require_success("locked offline launcher check", &output)
    }

    /// Builds and executes the generated launcher offline with its reconciled lockfile locked.
    pub fn run_locked_offline(&self) -> Result<()> {
        let target_dir = self.directory.join("target");
        let output = self.cargo(vec![
            OsString::from("run"),
            OsString::from("--manifest-path"),
            self.manifest_path.as_os_str().to_owned(),
            OsString::from("--locked"),
            OsString::from("--offline"),
            OsString::from("--target-dir"),
            target_dir
                .to_str()
                .ok_or_else(|| anyhow!("generated target path is not valid UTF-8"))?
                .into(),
        ])?;
        require_success("locked offline launcher execution", &output)
    }

    /// Reconciles the generated crate into its copied workspace lockfile offline.
    fn reconcile_lockfile(&self) -> Result<()> {
        let metadata = self.cargo(configured_metadata_arguments(
            &self.federate,
            &self.manifest_path,
        )?)?;
        require_success("lock reconciliation", &metadata)
    }

    /// Runs one Cargo command against this generated manifest with compatibility inputs set.
    fn cargo(&self, arguments: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<Output> {
        let mut command = Command::new(cargo_program(std::env::var_os("CARGO")));
        command
            .current_dir(&self.directory)
            .args(arguments)
            .envs(self.compile_inputs.iter().map(|(key, value)| (key, value)));
        command
            .output()
            .context("failed to start generated Cargo command")
    }
}

/// Converts a configured Cargo path to a UTF-8 command-line argument.
fn configured_path_argument(path: &Path, description: &str) -> Result<OsString> {
    path.to_str()
        .map(OsString::from)
        .ok_or_else(|| anyhow!("{description} path {} is not valid UTF-8", path.display()))
}

/// Builds the configured Cargo arguments used to reconcile the generated lockfile.
fn configured_metadata_arguments(
    federate: &ResolvedFederate,
    manifest_path: &Path,
) -> Result<Vec<OsString>> {
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
            configured_path_argument(cargo_config, "configured Cargo configuration")?,
        ]);
    }
    Ok(arguments)
}

/// Compares manifest paths after resolving native aliases and symlinks.
fn same_manifest_identity(expected: &Path, reported: &Path) -> Result<bool> {
    let expected = fs::canonicalize(expected).with_context(|| {
        format!(
            "failed to canonicalize generated manifest {}",
            expected.display()
        )
    })?;
    let reported = fs::canonicalize(reported).with_context(|| {
        format!(
            "failed to canonicalize Cargo-reported manifest {}",
            reported.display()
        )
    })?;
    Ok(expected == reported)
}

/// Collects Cargo-rendered compiler diagnostics from JSON message output.
fn rendered_compiler_diagnostics(stdout: &[u8]) -> Result<String> {
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

/// Selects Cargo from the runtime environment with the conventional executable fallback.
fn cargo_program(configured: Option<OsString>) -> OsString {
    configured.unwrap_or_else(|| OsString::from("cargo"))
}

/// Generates an isolated static Rust launcher for one Federate in a named deployment.
pub fn generate_launcher(
    workspace: impl AsRef<Path>,
    deployment_name: &str,
    federate_id: &str,
) -> Result<GeneratedLauncher> {
    let analyzed = analyze(workspace, deployment_name)?;
    generate_analyzed_launcher(&analyzed, federate_id)
}

/// Generates an isolated static Rust launcher from already completed deployment analysis.
pub(crate) fn generate_analyzed_launcher(
    analyzed: &AnalyzedDeployment,
    federate_id: &str,
) -> Result<GeneratedLauncher> {
    let federates = analyzed.compiled.federates();
    let (federate_index, federate) = federates
        .iter()
        .enumerate()
        .find(|(_, federate)| federate.id().as_str() == federate_id)
        .ok_or_else(|| {
            anyhow!(
                "deployment '{}' has no Federate '{federate_id}'",
                analyzed.resolved.deployment_name()
            )
        })?;
    if federates.len() != 1 {
        bail!("static launcher generation currently supports one local Federate");
    }
    if federate.runtime().as_str() != "std" {
        bail!(
            "Federate '{federate_id}' selects unsupported runtime '{}'",
            federate.runtime()
        );
    }
    let configuration = analyzed
        .resolved
        .deployment()
        .federates
        .get(federate_id)
        .cloned()
        .ok_or_else(|| {
            anyhow!("deployment has no resolved configuration for Federate '{federate_id}'")
        })?;

    let aliases = payload_aliases(&analyzed.resolved, &analyzed.driver, federate)?;
    let manifest = render_manifest(&analyzed.resolved, &aliases)?;
    let execution = analyzed
        .resolved
        .deployment()
        .execution
        .clone()
        .unwrap_or_default();
    let source = rust::render_launcher(
        &analyzed.driver,
        &analyzed.compiled,
        federate_index,
        &aliases,
        &execution,
    )?;
    let compile_inputs = payload_compile_inputs(&analyzed.resolved, &analyzed.driver)?;

    let parent = analyzed
        .resolved
        .target_directory()
        .join("boomerang/generated-launcher")
        .join(GENERATED_LAUNCHER_CACHE_VERSION);
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let manifest_path = staging.path().join("Cargo.toml");
    let source_dir = staging.path().join("src");
    fs::create_dir(&source_dir)
        .with_context(|| format!("failed to prepare {}", source_dir.display()))?;
    let source_path = source_dir.join("main.rs");
    fs::write(&manifest_path, &manifest)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    fs::write(&source_path, &source)
        .with_context(|| format!("failed to write {}", source_path.display()))?;
    let lockfile_path = staging.path().join("Cargo.lock");
    fs::copy(analyzed.resolved.lockfile().path.as_path(), &lockfile_path)
        .context("failed to copy source workspace lockfile")?;

    let candidate = GeneratedLauncher {
        directory: staging.path().to_path_buf(),
        manifest_path,
        source_path,
        lockfile_path,
        compile_inputs: compile_inputs.clone(),
        federate: configuration.clone(),
    };
    candidate.reconcile_lockfile()?;
    let reconciled_lockfile = fs::read(candidate.lockfile_path()).with_context(|| {
        format!(
            "failed to read reconciled generated lockfile {}",
            candidate.lockfile_path().display()
        )
    })?;
    validate_launcher_cache_entry(
        staging.path(),
        manifest.as_bytes(),
        source.as_bytes(),
        &reconciled_lockfile,
    )?;
    let identity = launcher_cache_identity(
        manifest.as_bytes(),
        source.as_bytes(),
        &reconciled_lockfile,
        &compile_inputs,
        &configuration,
    )?;
    let directory = parent.join(identity);
    match rename_noreplace(staging.path(), &directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to publish generated launcher cache {} to {}",
                    staging.path().display(),
                    directory.display()
                )
            });
        }
    }
    validate_launcher_cache_entry(
        &directory,
        manifest.as_bytes(),
        source.as_bytes(),
        &reconciled_lockfile,
    )
    .with_context(|| {
        format!(
            "generated launcher cache {} is invalid",
            directory.display()
        )
    })?;
    let manifest_path = directory.join("Cargo.toml");
    let source_path = directory.join("src/main.rs");
    let lockfile_path = directory.join("Cargo.lock");

    Ok(GeneratedLauncher {
        directory,
        manifest_path,
        source_path,
        lockfile_path,
        compile_inputs,
        federate: configuration,
    })
}

/// Computes the versioned canonical identity of one reconciled launcher workspace.
fn launcher_cache_identity(
    manifest: &[u8],
    source: &[u8],
    lockfile: &[u8],
    compile_inputs: &[(String, String)],
    federate: &ResolvedFederate,
) -> Result<String> {
    let target_json_hash =
        configured_content_hash(federate.target_json.as_deref(), "configured target JSON")?;
    let cargo_config_hash = configured_content_hash(
        federate.cargo_config.as_deref(),
        "configured Cargo configuration",
    )?;
    let mut inputs = compile_inputs.to_vec();
    inputs.sort();

    let mut hasher = blake3::Hasher::new();
    digest_cache_field(
        &mut hasher,
        "cache-version",
        Some(GENERATED_LAUNCHER_CACHE_VERSION.as_bytes()),
    );
    digest_cache_field(&mut hasher, "manifest", Some(manifest));
    digest_cache_field(&mut hasher, "source", Some(source));
    digest_cache_field(&mut hasher, "lockfile", Some(lockfile));
    let input_count = u64::try_from(inputs.len())
        .expect("compile input count must fit the canonical u64 representation")
        .to_be_bytes();
    digest_cache_field(&mut hasher, "compile-input-count", Some(&input_count));
    for (key, value) in inputs {
        digest_cache_field(&mut hasher, "compile-input-key", Some(key.as_bytes()));
        digest_cache_field(&mut hasher, "compile-input-value", Some(value.as_bytes()));
    }
    digest_cache_field(
        &mut hasher,
        "target",
        federate.target.as_deref().map(str::as_bytes),
    );
    digest_cache_field(
        &mut hasher,
        "profile",
        federate.profile.as_deref().map(str::as_bytes),
    );
    digest_cache_field(
        &mut hasher,
        "toolchain",
        federate.toolchain.as_deref().map(str::as_bytes),
    );
    digest_cache_field(
        &mut hasher,
        "target-json-hash",
        target_json_hash
            .as_ref()
            .map(|hash| hash.as_bytes().as_ref()),
    );
    digest_cache_field(
        &mut hasher,
        "cargo-config-hash",
        cargo_config_hash
            .as_ref()
            .map(|hash| hash.as_bytes().as_ref()),
    );
    Ok(hasher.finalize().to_hex().to_string())
}

/// Appends one labelled optional value using a presence tag and big-endian lengths.
fn digest_cache_field(hasher: &mut blake3::Hasher, label: &str, value: Option<&[u8]>) {
    let label_length = u64::try_from(label.len())
        .expect("cache identity label length must fit u64")
        .to_be_bytes();
    hasher.update(&label_length);
    hasher.update(label.as_bytes());
    match value {
        Some(value) => {
            hasher.update(&[1]);
            let value_length = u64::try_from(value.len())
                .expect("cache identity value length must fit u64")
                .to_be_bytes();
            hasher.update(&value_length);
            hasher.update(value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

/// Hashes an optional configured file's current bytes for cache identity selection.
fn configured_content_hash(path: Option<&Path>, description: &str) -> Result<Option<blake3::Hash>> {
    path.map(|path| {
        fs::read(path)
            .with_context(|| format!("failed to read {description} {}", path.display()))
            .map(|bytes| blake3::hash(&bytes))
    })
    .transpose()
}

/// Validates the canonical generated files before a cache entry is selected or reused.
fn validate_launcher_cache_entry(
    directory: &Path,
    manifest: &[u8],
    source: &[u8],
    lockfile: &[u8],
) -> Result<()> {
    validate_cache_directory(directory, "generated launcher cache")?;
    validate_cache_directory(
        &directory.join("src"),
        "generated launcher source directory",
    )?;
    let target = directory.join("target");
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => bail!(
            "generated launcher target directory {} is not a directory",
            target.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect generated launcher target directory {}",
                    target.display()
                )
            });
        }
    }
    validate_cache_file(
        &directory.join("Cargo.toml"),
        manifest,
        "generated manifest",
    )?;
    validate_cache_file(&directory.join("src/main.rs"), source, "generated source")?;
    validate_cache_file(
        &directory.join("Cargo.lock"),
        lockfile,
        "reconciled generated lockfile",
    )
}

/// Requires a cache path to be a real directory rather than a symlink or other file type.
fn validate_cache_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    if !metadata.file_type().is_dir() {
        bail!("{description} {} is not a directory", path.display());
    }
    Ok(())
}

/// Requires a generated cache file to be regular and byte-identical to its canonical input.
fn validate_cache_file(path: &Path, expected: &[u8], description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{description} {} is not a regular file", path.display());
    }
    let actual = fs::read(path)
        .with_context(|| format!("failed to read {description} {}", path.display()))?;
    if actual != expected {
        bail!(
            "{description} {} does not match canonical input",
            path.display()
        );
    }
    Ok(())
}

/// Maps selected implementation package identities to deterministic generated crate aliases.
fn payload_aliases(
    resolved: &ResolvedWorkspace,
    driver: &DriverOutput,
    federate: &boomerang_builder::compiler::OwnedFederateImage,
) -> Result<BTreeMap<String, String>> {
    let required = federate
        .enclaves()
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

/// Renders the standalone launcher manifest with only runtime and selected payload packages.
fn render_manifest(
    resolved: &ResolvedWorkspace,
    aliases: &BTreeMap<String, String>,
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
) -> Result<Vec<(String, String)>> {
    let mut inputs = vec![(
        PAYLOAD_MACRO_ABI_COMPILE_INPUT.to_owned(),
        boomerang_runtime::binding::COMPONENT_DESCRIPTOR_MACRO_ABI.to_string(),
    )];
    for binding in driver.bindings() {
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
        cargo_program, configured_metadata_arguments, rendered_compiler_diagnostics,
        same_manifest_identity, validate_launcher_cache_entry,
    };
    use crate::ResolvedFederate;
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn launcher_cache_rejects_symlinked_target_directory() {
        let cache = tempfile::tempdir().unwrap();
        fs::create_dir(cache.path().join("src")).unwrap();
        let manifest = b"manifest";
        let source = b"source";
        let lockfile = b"lockfile";
        fs::write(cache.path().join("Cargo.toml"), manifest).unwrap();
        fs::write(cache.path().join("src/main.rs"), source).unwrap();
        fs::write(cache.path().join("Cargo.lock"), lockfile).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), cache.path().join("target")).unwrap();

        let error = validate_launcher_cache_entry(cache.path(), manifest, source, lockfile)
            .expect_err("symlinked cache target must be rejected");

        assert!(error.to_string().contains("target directory"), "{error:#}");
    }

    #[test]
    fn metadata_reconciliation_preserves_federate_toolchain_and_cargo_config() {
        let federate = ResolvedFederate {
            groups: Vec::new(),
            target: None,
            toolchain: Some(String::from("nightly-test")),
            profile: None,
            runtime: String::from("std"),
            target_json: None,
            cargo_config: Some(std::path::PathBuf::from("/tmp/cargo-config.toml")),
        };
        let arguments = configured_metadata_arguments(&federate, Path::new("Cargo.toml")).unwrap();
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

    #[test]
    fn generated_manifest_identity_accepts_canonical_path() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"test\"\n").unwrap();

        let canonical = std::fs::canonicalize(&manifest).unwrap();
        assert!(same_manifest_identity(&manifest, &canonical).unwrap());
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
        assert_eq!(cargo_program(Some("custom-cargo".into())), "custom-cargo");
        assert_eq!(cargo_program(None), "cargo");
    }
}
