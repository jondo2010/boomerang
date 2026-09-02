//! Ephemeral static launcher generation for one compiled Federate.

mod rust;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{anyhow, bail, Context, Result};
use boomerang_runtime::binding::{
    payload_fingerprint_compile_input_key, PAYLOAD_MACRO_ABI_COMPILE_INPUT,
};

use crate::{check::analyze, generated::dependency, DriverOutput, ResolvedWorkspace};

/// An owned temporary Cargo crate containing one generated static Federate launcher.
pub struct GeneratedLauncher {
    /// Temporary directory whose lifetime owns every generated launcher file.
    directory: tempfile::TempDir,
    /// Path to the generated Cargo manifest.
    manifest_path: PathBuf,
    /// Path to the generated Rust executable source.
    source_path: PathBuf,
    /// Host-verified payload compatibility inputs passed to Cargo.
    compile_inputs: Vec<(String, String)>,
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

    /// Reconciles the copied lockfile offline, then checks the launcher with it locked.
    pub fn check_locked_offline(&self) -> Result<()> {
        self.reconcile_lockfile()?;
        let target_dir = self.directory.path().join("target");
        let output = self.cargo([
            "check",
            "--locked",
            "--offline",
            "--target-dir",
            target_dir
                .to_str()
                .ok_or_else(|| anyhow!("generated target path is not valid UTF-8"))?,
        ])?;
        require_success("locked offline launcher check", output)
    }

    /// Builds and executes the generated launcher offline with its copied lockfile.
    pub fn run_locked_offline(&self) -> Result<()> {
        self.reconcile_lockfile()?;
        let target_dir = self.directory.path().join("target");
        let output = self.cargo([
            "run",
            "--locked",
            "--offline",
            "--target-dir",
            target_dir
                .to_str()
                .ok_or_else(|| anyhow!("generated target path is not valid UTF-8"))?,
        ])?;
        require_success("locked offline launcher execution", output)
    }

    /// Reconciles the generated crate into its copied workspace lockfile offline.
    fn reconcile_lockfile(&self) -> Result<()> {
        let metadata = self.cargo(["metadata", "--format-version", "1", "--offline"])?;
        require_success("lock reconciliation", metadata)
    }

    /// Runs one Cargo command against this generated manifest with compatibility inputs set.
    fn cargo<const N: usize>(&self, arguments: [&str; N]) -> Result<Output> {
        let (subcommand, arguments) = arguments
            .split_first()
            .expect("generated Cargo command requires a subcommand");
        let mut command = Command::new(cargo_program(std::env::var_os("CARGO")));
        command
            .current_dir(self.directory.path())
            .arg(subcommand)
            .arg("--manifest-path")
            .arg(&self.manifest_path)
            .args(arguments)
            .envs(self.compile_inputs.iter().map(|(key, value)| (key, value)));
        command
            .output()
            .context("failed to start generated Cargo command")
    }
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
    let federates = analyzed.compiled.federates();
    let (federate_index, federate) = federates
        .iter()
        .enumerate()
        .find(|(_, federate)| federate.id().as_str() == federate_id)
        .ok_or_else(|| anyhow!("deployment '{deployment_name}' has no Federate '{federate_id}'"))?;
    if federates.len() != 1 {
        bail!("static launcher generation currently supports one local Federate");
    }
    if federate.runtime().as_str() != "std" {
        bail!(
            "Federate '{federate_id}' selects unsupported runtime '{}'",
            federate.runtime()
        );
    }

    let aliases = payload_aliases(&analyzed.resolved, &analyzed.driver, federate)?;
    let manifest = render_manifest(&analyzed.resolved, &aliases)?;
    let source = rust::render_launcher(
        &analyzed.driver,
        &analyzed.compiled,
        federate_index,
        &aliases,
    )?;
    let compile_inputs = payload_compile_inputs(&analyzed.resolved, &analyzed.driver)?;

    let parent = analyzed
        .resolved
        .target_directory()
        .join("boomerang/generated-launcher");
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let directory = tempfile::Builder::new()
        .prefix(&format!("{deployment_name}-{federate_id}-"))
        .tempdir_in(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let manifest_path = directory.path().join("Cargo.toml");
    let source_dir = directory.path().join("src");
    fs::create_dir(&source_dir)
        .with_context(|| format!("failed to prepare {}", source_dir.display()))?;
    let source_path = source_dir.join("main.rs");
    fs::write(&manifest_path, manifest)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    fs::write(&source_path, source)
        .with_context(|| format!("failed to write {}", source_path.display()))?;
    fs::copy(
        analyzed.resolved.lockfile().path.as_path(),
        directory.path().join("Cargo.lock"),
    )
    .context("failed to copy source workspace lockfile")?;

    Ok(GeneratedLauncher {
        directory,
        manifest_path,
        source_path,
        compile_inputs,
    })
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
fn require_success(phase: &'static str, output: Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "generated launcher {phase} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(test)]
mod tests {
    use super::cargo_program;

    #[test]
    fn generated_cargo_uses_the_runtime_override() {
        assert_eq!(cargo_program(Some("custom-cargo".into())), "custom-cargo");
        assert_eq!(cargo_program(None), "cargo");
    }
}
