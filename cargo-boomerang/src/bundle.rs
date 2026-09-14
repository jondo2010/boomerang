//! Verified construction and failure-atomic publication of deployment bundles.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, BufReader, Read, Seek, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::check::{ResourceReport, COMPILER_SCHEMA};

/// Current public deployment-document schema.
pub(crate) const DEPLOYMENT_SCHEMA: u32 = 1;

/// Stable domain separator for schema-v1 deployment fingerprint inputs.
const DEPLOYMENT_FINGERPRINT_DOMAIN_V1: &str = "boomerang.deployment.v1";

/// Complete schema-v1 deployment document published with one artifact bundle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentDocument {
    /// Deployment-document schema version.
    pub(crate) schema: u32,
    /// Compiler image schema included in the semantic fingerprint.
    pub(crate) compiler_schema: u32,
    /// Selected deployment name.
    pub(crate) deployment: String,
    /// Bundle reproducibility fingerprint naming its directory, not a peer compatibility claim.
    pub(crate) fingerprint: String,
    /// Lowercase BLAKE3 hash of compact canonical topology JSON.
    pub(crate) topology_hash: String,
    /// Lowercase BLAKE3 hash of the source workspace lockfile.
    pub(crate) source_lock_hash: String,
    /// Lowercase BLAKE3 hash of the reconciled generated lockfile.
    pub(crate) generated_lock_hash: String,
    /// Lowercase BLAKE3 hash of generated Rust launcher source.
    pub(crate) generated_source_hash: String,
    /// Selected component implementations and compatibility descriptors.
    pub(crate) bindings: Vec<BindingDocument>,
    /// Built Federates in compiler identity order.
    pub(crate) federates: Vec<FederateDocument>,
    /// Separate compiled central RTI, absent for local coordination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rti: Option<RtiDocument>,
    /// Deployment execution policy embedded in every generated launcher.
    pub(crate) execution: ExecutionPolicyDocument,
    /// Canonical statically computed resource bounds.
    pub(crate) resources: ResourceReport,
    /// Selected coordination backend and protocol identity.
    pub(crate) coordination: CoordinationDocument,
    /// Generated source files retained for audit and reproduction.
    pub(crate) generated: Vec<FileRecord>,
    /// Executable files ready for deployment.
    pub(crate) artifacts: Vec<FileRecord>,
}

/// Canonical semantic input for schema-v1 deployment fingerprints.
#[derive(Serialize)]
struct FingerprintInputV1<'a> {
    /// Stable domain separator for schema-v1 deployment fingerprints.
    domain: &'static str,
    /// Deployment-document schema version.
    schema: u32,
    /// Canonical compiler-image schema version.
    compiler_schema: u32,
    /// Lowercase BLAKE3 hash of compact canonical topology JSON.
    topology_hash: &'a str,
    /// Selected implementation bindings in canonical driver order.
    bindings: &'a [BindingDocument],
    /// Lowercase BLAKE3 hash of the source workspace lockfile.
    source_lock_hash: &'a str,
    /// Lowercase BLAKE3 hash of the reconciled generated lockfile.
    generated_lock_hash: &'a str,
    /// Lowercase BLAKE3 hash of generated Rust launcher source.
    generated_source_hash: &'a str,
    /// Federate target and runtime selections in compiler identity order.
    federates: &'a [FederateDocument],
    /// RTI compilation selection, excluding publication paths and bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    rti: Option<RtiFingerprintInput<'a>>,
    /// Deployment execution policy embedded in generated source.
    execution: &'a ExecutionPolicyDocument,
    /// Canonical static resource projection.
    resources: &'a ResourceReport,
    /// Selected coordination backend and protocol identity.
    coordination: &'a CoordinationDocument,
}

/// Path-neutral RTI build inputs participating in the deployment fingerprint.
#[derive(Serialize)]
struct RtiFingerprintInput<'a> {
    target: &'a str,
    profile: Option<&'a str>,
}

impl<'a> From<&'a RtiDocument> for RtiFingerprintInput<'a> {
    fn from(rti: &'a RtiDocument) -> Self {
        Self {
            target: &rti.target,
            profile: rti.profile.as_deref(),
        }
    }
}

/// Computes the canonical semantic fingerprint for one deployment document.
pub(crate) fn deployment_fingerprint(document: &DeploymentDocument) -> Result<String> {
    let input = FingerprintInputV1 {
        domain: DEPLOYMENT_FINGERPRINT_DOMAIN_V1,
        schema: document.schema,
        compiler_schema: document.compiler_schema,
        topology_hash: &document.topology_hash,
        bindings: &document.bindings,
        source_lock_hash: &document.source_lock_hash,
        generated_lock_hash: &document.generated_lock_hash,
        generated_source_hash: &document.generated_source_hash,
        federates: &document.federates,
        rti: document.rti.as_ref().map(RtiFingerprintInput::from),
        execution: &document.execution,
        resources: &document.resources,
        coordination: &document.coordination,
    };
    let bytes = serde_json::to_vec(&input)
        .context("failed to serialize canonical deployment fingerprint input")?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// One selected component binding recorded in the deployment document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BindingDocument {
    /// Stable component-instance identity.
    pub(crate) component: String,
    /// Exact selected Cargo package identity and features.
    pub(crate) package: PackageDocument,
    /// Host-verified descriptor compatibility identity.
    pub(crate) descriptor: DescriptorDocument,
}

/// Path-neutral Cargo package identity recorded for a selected binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackageDocument {
    /// Cargo package name.
    pub(crate) name: String,
    /// Exact Cargo package version.
    pub(crate) version: String,
    /// Cargo source identity, absent for local path packages.
    pub(crate) source: Option<String>,
    /// Canonically sorted manifest-selected features.
    pub(crate) features: Vec<String>,
}

/// Descriptor identity proving the selected payload contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescriptorDocument {
    /// Stable component-instance identity.
    pub(crate) component: String,
    /// Selected implementation package name.
    pub(crate) package: String,
    /// Stable external contract identity.
    pub(crate) contract: String,
    /// Stable external contract version.
    pub(crate) contract_version: u64,
    /// Lowercase host-computed descriptor fingerprint.
    pub(crate) fingerprint: String,
    /// Descriptor macro ABI version.
    pub(crate) macro_abi: u32,
}

/// Target and runtime identity for one built Federate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FederateDocument {
    /// Identity of this Federate scheduler image, direct bindings, and local bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) image_fingerprint: Option<String>,
    /// Stable Federate identity.
    pub(crate) id: String,
    /// Canonically sorted placement groups assigned to this Federate.
    pub(crate) groups: Vec<String>,
    /// Selected Rust compilation target.
    pub(crate) target: String,
    /// Optional Rust toolchain selector.
    pub(crate) toolchain: Option<String>,
    /// Optional Cargo profile selector.
    pub(crate) profile: Option<String>,
    /// Selected runtime backend.
    pub(crate) runtime: String,
    /// Hash of a custom target JSON file, when selected.
    pub(crate) target_json_hash: Option<String>,
    /// Hash of a custom Cargo configuration file, when selected.
    pub(crate) cargo_config_hash: Option<String>,
}

/// Normalized deployment execution policy embedded literally in generated launchers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionPolicyDocument {
    /// Whether logical execution bypasses wall-clock synchronization.
    pub(crate) fast_forward: bool,
    /// Whether schedulers remain alive without pending events.
    pub(crate) keep_alive: bool,
    /// Optional logical horizon in nanoseconds.
    pub(crate) logical_horizon_nanos: Option<u64>,
}

/// Coordination selection recorded for external deployment tooling.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoordinationDocument {
    /// Selected backend identity, including `local` for one Federate.
    pub(crate) backend: String,
    /// Versioned protocol identity, absent for local coordination.
    pub(crate) protocol: Option<String>,
    /// Shared coordination compatibility fingerprint embedded in participating launchers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity: Option<String>,
    /// Portable canonical protocol profile reserved for the framed transport adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) wire: Option<WireProfileDocument>,
}

/// Explicit baseline wire profile and dense-mapping claim shared by every route.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireProfileDocument {
    /// Portable frame protocol version.
    pub(crate) protocol: u16,
    /// Canonical wire codec profile version.
    pub(crate) codec: u16,
    /// Inclusive maximum encoded payload bytes, independent of scheduler storage bounds.
    pub(crate) max_payload_bytes: usize,
    /// Digest of the exact canonical dense mapping.
    pub(crate) mapping: String,
}

/// Compilation identity and files owned by the central RTI, never by a Federate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RtiDocument {
    /// Rust target selected for the hosted RTI executable.
    pub(crate) target: String,
    /// Optional Cargo profile selector.
    pub(crate) profile: Option<String>,
    /// Generated RTI workspace files retained for reproduction.
    pub(crate) generated: Vec<RtiFileRecord>,
    /// Exactly one executable once the bundle is published.
    pub(crate) artifact: Option<RtiFileRecord>,
}

/// One file owned by the RTI; ownership follows from the enclosing RTI document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RtiFileRecord {
    /// Normalized bundle-relative path in the dedicated RTI namespace.
    path: String,
    /// BLAKE3 hash of the published bytes.
    blake3: String,
}

/// One normalized bundle-relative file path and exact content hash.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileRecord {
    /// Federate that owns the generated or executable file.
    federate: String,
    /// `/`-normalized path relative to the fingerprint directory.
    path: String,
    /// Lowercase BLAKE3 hash of the exact published bytes.
    blake3: String,
}

/// Temporary generated files and executable consumed by bundle publication.
pub(crate) struct BundleSource<'a> {
    /// Stable Federate identity used as a safe directory component.
    pub(crate) federate: &'a str,
    /// Generated Cargo manifest.
    pub(crate) manifest: &'a Path,
    /// Reconciled generated Cargo lockfile.
    pub(crate) lockfile: &'a Path,
    /// Generated static-launcher Rust source.
    pub(crate) source: &'a Path,
    /// Canonical executable emitted by Cargo.
    pub(crate) executable: &'a Path,
}

/// Temporary generated workspace and executable owned by the central RTI.
pub(crate) struct RtiBundleSource<'a> {
    /// Generated RTI Cargo manifest.
    pub(crate) manifest: &'a Path,
    /// Reconciled RTI Cargo lockfile.
    pub(crate) lockfile: &'a Path,
    /// Generated RTI Rust source.
    pub(crate) source: &'a Path,
    /// RTI executable emitted by Cargo.
    pub(crate) executable: &'a Path,
}

/// Stable paths produced by one successful immutable bundle publication.
#[derive(Debug)]
pub(crate) struct PublishedBundle {
    /// Published deployment manifest returned by the public build API.
    manifest: PathBuf,
    /// Canonically ordered executable artifacts in the published bundle.
    executables: Vec<PublishedExecutable>,
    /// Separate central RTI executable, if this deployment uses it.
    rti_executable: Option<PathBuf>,
}

impl PublishedBundle {
    /// Returns the canonically ordered published executable artifacts.
    pub(crate) fn executables(&self) -> &[PublishedExecutable] {
        &self.executables
    }

    /// Returns the separately owned central RTI executable.
    pub(crate) fn rti_executable(&self) -> Option<&Path> {
        self.rti_executable.as_deref()
    }

    /// Consumes the publication result and returns its deployment manifest.
    pub(crate) fn into_manifest(self) -> PathBuf {
        self.manifest
    }
}

/// One federate-owned executable in an immutable published bundle.
#[derive(Debug)]
pub(crate) struct PublishedExecutable {
    /// Stable federate identity owning this executable.
    federate: String,
    /// Absolute path to the published executable bytes.
    path: PathBuf,
}

impl PublishedExecutable {
    /// Returns the stable identity of the owning Federate.
    pub(crate) fn federate(&self) -> &str {
        &self.federate
    }

    /// Returns the absolute path to the published executable.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Verified document and executable selected from a published deployment bundle.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct PublishedArtifact {
    /// Complete validated deployment document.
    pub(crate) document: DeploymentDocument,
    /// Open regular executable selected from the published artifact record.
    pub(crate) executable: File,
    /// Expected BLAKE3 digest for the selected executable bytes.
    pub(crate) executable_hash: String,
}

/// All opened executables from one validated immutable deployment bundle.
#[derive(Debug)]
pub(crate) struct PublishedArtifacts {
    /// Complete validated deployment document.
    pub(crate) document: DeploymentDocument,
    /// Federate artifacts in compiler identity order.
    pub(crate) federates: Vec<LoadedFederateArtifact>,
    /// Separately owned RTI executable, present only for central coordination.
    pub(crate) rti: Option<LoadedRtiArtifact>,
}

/// Opened Federate executable and the digest expected by execution staging.
#[derive(Debug)]
pub(crate) struct LoadedFederateArtifact {
    /// Stable identity of this artifact's Federate.
    pub(crate) federate: String,
    /// Open regular executable with verified bytes.
    pub(crate) executable: File,
    /// Expected BLAKE3 digest, retained for execution staging verification.
    pub(crate) executable_hash: String,
}

/// Opened RTI executable and the digest expected by execution staging.
#[derive(Debug)]
pub(crate) struct LoadedRtiArtifact {
    /// Open regular executable with verified bytes.
    pub(crate) executable: File,
    /// Expected BLAKE3 digest, retained for execution staging verification.
    pub(crate) executable_hash: String,
}

/// Loads and opens every executable after verifying the complete bundle.
pub(crate) fn load_published_artifacts(manifest: &Path) -> Result<PublishedArtifacts> {
    if manifest.file_name().and_then(|name| name.to_str()) != Some("deployment.json") {
        bail!("published artifact manifest must be named deployment.json");
    }
    let bundle = manifest
        .parent()
        .ok_or_else(|| anyhow!("published artifact manifest has no parent directory"))?;
    let document = read_document(bundle)?;
    validate_bundle(bundle, &document)?;
    if deployment_fingerprint(&document)? != document.fingerprint {
        bail!("deployment semantic fingerprint mismatch");
    }
    let directory_name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("published artifact directory name is not valid UTF-8"))?;
    if directory_name != document.fingerprint {
        bail!("published artifact directory name does not match deployment fingerprint");
    }
    // Artifact records may arrive in any order; execution follows compiler identity order.
    let federates = document
        .federates
        .iter()
        .map(|federate| {
            let artifact = document
                .artifacts
                .iter()
                .find(|artifact| artifact.federate == federate.id)
                .expect("validated Federate has exactly one artifact");
            Ok(LoadedFederateArtifact {
                federate: federate.id.clone(),
                executable: open_verified_artifact(bundle, &artifact.path, &artifact.blake3)?,
                executable_hash: artifact.blake3.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let rti = document
        .rti
        .as_ref()
        .map(|rti| {
            let artifact = rti
                .artifact
                .as_ref()
                .expect("validated RTI has an artifact");
            Ok::<_, anyhow::Error>(LoadedRtiArtifact {
                executable: open_verified_artifact(bundle, &artifact.path, &artifact.blake3)?,
                executable_hash: artifact.blake3.clone(),
            })
        })
        .transpose()?;
    Ok(PublishedArtifacts {
        document,
        federates,
        rti,
    })
}

/// Loads one immutable, semantically validated local deployment artifact.
#[cfg(test)]
pub(crate) fn load_published_artifact(manifest: &Path) -> Result<PublishedArtifact> {
    let PublishedArtifacts {
        document,
        mut federates,
        rti,
    } = load_published_artifacts(manifest)?;
    if federates.len() != 1 || rti.is_some() {
        bail!("single-artifact loader requires one local Federate");
    }
    let artifact = federates.pop().expect("checked one Federate artifact");
    Ok(PublishedArtifact {
        document,
        executable: artifact.executable,
        executable_hash: artifact.executable_hash,
    })
}

/// Rechecks bytes on the opened handle to close the validation-to-open race.
fn open_verified_artifact(bundle: &Path, relative: &str, expected: &str) -> Result<File> {
    let path = join_normalized(bundle, relative)?;
    let mut file = open_published_artifact(&path)?;
    if hash_open_file(&mut file)? != expected {
        bail!("bundle hash mismatch for {relative}");
    }
    file.rewind()
        .context("failed to rewind verified artifact")?;
    Ok(file)
}

/// Opens one regular artifact selected from an already validated published bundle.
pub(crate) fn open_published_artifact(path: &Path) -> Result<File> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular published executable", path.display());
    }
    Ok(file)
}

/// Stages, verifies, and atomically publishes an immutable deployment bundle.
#[cfg(test)]
pub(crate) fn publish_bundle(
    target_directory: &Path,
    document: DeploymentDocument,
    sources: &[BundleSource<'_>],
) -> Result<PublishedBundle> {
    publish_bundle_with_rti(target_directory, document, sources, None)
}

/// Publishes Federate workspaces and an optional separately owned central RTI.
pub(crate) fn publish_bundle_with_rti(
    target_directory: &Path,
    mut document: DeploymentDocument,
    sources: &[BundleSource<'_>],
    rti_source: Option<&RtiBundleSource<'_>>,
) -> Result<PublishedBundle> {
    validate_coordination(&document)?;
    if document.rti.is_some() != rti_source.is_some() {
        bail!("RTI source must be present exactly when central RTI metadata is present");
    }
    validate_segment(&document.deployment, "deployment")?;
    validate_fingerprint(&document.fingerprint)?;
    if sources.len() != document.federates.len() {
        bail!("bundle source count does not match compiled Federate count");
    }
    for (federate, source) in document.federates.iter().zip(sources) {
        if federate.id != source.federate {
            bail!("bundle sources are not in canonical Federate order");
        }
        validate_segment(source.federate, "Federate")?;
        let executable_name = source
            .executable
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("generated executable name is not valid UTF-8"))?;
        validate_segment(executable_name, "executable")?;
    }

    let parent = target_directory
        .join("boomerang")
        .join(&document.deployment);
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{}.staging-", document.fingerprint))
        .tempdir_in(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;

    document.generated.clear();
    document.artifacts.clear();
    for source in sources {
        let generated = federate_directory("generated", source.federate, document.rti.is_some());
        document.generated.extend([
            stage_file(
                staging.path(),
                source.federate,
                source.manifest,
                &format!("{generated}/Cargo.toml"),
            )?,
            stage_file(
                staging.path(),
                source.federate,
                source.lockfile,
                &format!("{generated}/Cargo.lock"),
            )?,
            stage_file(
                staging.path(),
                source.federate,
                source.source,
                &format!("{generated}/src/main.rs"),
            )?,
        ]);
        let executable_name = source
            .executable
            .file_name()
            .and_then(|name| name.to_str())
            .expect("validated executable name remains UTF-8");
        document.artifacts.push(stage_file(
            staging.path(),
            source.federate,
            source.executable,
            &format!(
                "{}/{executable_name}",
                federate_directory("artifacts", source.federate, document.rti.is_some())
            ),
        )?);
    }

    if let (Some(rti), Some(source)) = (&mut document.rti, rti_source) {
        rti.generated = [
            (source.manifest, "generated/rti/Cargo.toml"),
            (source.lockfile, "generated/rti/Cargo.lock"),
            (source.source, "generated/rti/src/main.rs"),
        ]
        .into_iter()
        .map(|(source, path)| stage_rti_file(staging.path(), source, path))
        .collect::<Result<Vec<_>>>()?;
        let name = source
            .executable
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("RTI executable name is not valid UTF-8"))?;
        validate_segment(name, "RTI executable")?;
        rti.artifact = Some(stage_rti_file(
            staging.path(),
            source.executable,
            &format!("artifacts/rti/{name}"),
        )?);
    }
    write_document(staging.path(), &document)?;
    let decoded = read_document(staging.path())?;
    if decoded != document {
        bail!("staged deployment document changed during serialization");
    }
    validate_bundle(staging.path(), &decoded)?;

    let final_directory = parent.join(&document.fingerprint);
    let manifest = match rename_noreplace(staging.path(), &final_directory) {
        Ok(()) => accept_existing(&final_directory, &document),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            accept_existing(&final_directory, &document)
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to publish deployment bundle {} to {}",
                staging.path().display(),
                final_directory.display()
            )
        }),
    }?;
    let bundle = manifest
        .parent()
        .expect("published deployment manifest has a bundle parent");
    let executables = document
        .artifacts
        .into_iter()
        .map(|artifact| {
            Ok(PublishedExecutable {
                federate: artifact.federate,
                path: join_normalized(bundle, &artifact.path)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let rti_executable = document
        .rti
        .and_then(|rti| rti.artifact)
        .map(|artifact| join_normalized(bundle, &artifact.path))
        .transpose()?;
    Ok(PublishedBundle {
        manifest,
        executables,
        rti_executable,
    })
}

/// Atomically renames a directory only when the destination does not exist.
pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    let result = platform_rename_noreplace(source, destination);
    match result {
        Err(error) if fs::symlink_metadata(destination).is_ok() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination exists: {error}"),
        )),
        other => other,
    }
}

/// Calls Linux `renameat2` with `RENAME_NOREPLACE` and maps `errno` through `last_os_error`.
#[cfg(target_os = "linux")]
fn platform_rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt as _;

    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;
    extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }

    let source = c_path(source, source.as_os_str().as_bytes())?;
    let destination = c_path(destination, destination.as_os_str().as_bytes())?;
    // SAFETY: both C strings are NUL-terminated owned buffers that outlive this call. The flags
    // request an atomic rename that fails when the destination exists; no pointers are retained.
    let status = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    os_status(status == 0)
}

/// Calls macOS `renamex_np` with `RENAME_EXCL` and maps `errno` through `last_os_error`.
#[cfg(target_os = "macos")]
fn platform_rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt as _;

    const RENAME_EXCL: c_uint = 0x0000_0004;
    extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }

    let source = c_path(source, source.as_os_str().as_bytes())?;
    let destination = c_path(destination, destination.as_os_str().as_bytes())?;
    // SAFETY: both C strings are NUL-terminated owned buffers that outlive this call. The exclusive
    // flag forbids replacement atomically, and the OS does not retain either pointer.
    let status = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    os_status(status == 0)
}

/// Calls Windows `MoveFileExW` without replacement and maps `GetLastError` via `last_os_error`.
#[cfg(windows)]
fn platform_rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }

    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    // SAFETY: both UTF-16 buffers are NUL-terminated and live through the call. Zero flags omit
    // MOVEFILE_REPLACE_EXISTING, so the atomic move cannot overwrite an existing destination.
    let status = unsafe { move_file_ex_w(source.as_ptr(), destination.as_ptr(), 0) };
    os_status(status != 0)
}

/// Reports unsupported publication platforms instead of weakening no-replace semantics.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace directory rename is unsupported on this platform",
    ))
}

/// Converts a Unix path byte string to a checked C string.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_path(path: &Path, bytes: &[u8]) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path contains an interior NUL: {}", path.display()),
        )
    })
}

/// Converts a Windows path to a checked NUL-terminated UTF-16 buffer.
#[cfg(windows)]
fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt as _;

    let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path contains an interior NUL: {}", path.display()),
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

/// Converts a platform success predicate to `io::Result` using the current OS error.
fn os_status(succeeded: bool) -> io::Result<()> {
    if succeeded {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Copies one source file and proves the staged bytes match its recorded hash.
fn stage_file(staging: &Path, federate: &str, source: &Path, relative: &str) -> Result<FileRecord> {
    let blake3 = stage_bytes(staging, source, relative)?;
    Ok(FileRecord {
        federate: federate.to_owned(),
        path: relative.to_owned(),
        blake3,
    })
}

/// Stages RTI bytes without inventing a Federate owner.
fn stage_rti_file(staging: &Path, source: &Path, relative: &str) -> Result<RtiFileRecord> {
    let blake3 = stage_bytes(staging, source, relative)?;
    Ok(RtiFileRecord {
        path: relative.to_owned(),
        blake3,
    })
}

/// Copies and verifies one file into the unpublished bundle.
fn stage_bytes(staging: &Path, source: &Path, relative: &str) -> Result<String> {
    let expected = hash_file(source)?;
    let destination = join_normalized(staging, relative)?;
    let destination_parent = destination
        .parent()
        .expect("validated relative file path has a parent");
    fs::create_dir_all(destination_parent)
        .with_context(|| format!("failed to prepare {}", destination_parent.display()))?;
    fs::copy(source, &destination).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    let actual = hash_file(&destination)?;
    if actual != expected {
        bail!("staged file {} changed while copying", source.display());
    }
    Ok(expected)
}

/// Writes a durable, newline-terminated deployment document into staging.
fn write_document(staging: &Path, document: &DeploymentDocument) -> Result<()> {
    let path = staging.join("deployment.json");
    let mut file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, document)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// Reads one complete deployment document without following a final symlink.
fn read_document(bundle: &Path) -> Result<DeploymentDocument> {
    let path = bundle.join("deployment.json");
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular deployment document", path.display());
    }
    let file = File::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("failed to decode {}", path.display()))
}

/// Accepts an existing immutable bundle only when it exactly matches the candidate.
fn accept_existing(final_directory: &Path, candidate: &DeploymentDocument) -> Result<PathBuf> {
    let existing = read_document(final_directory)
        .and_then(|document| {
            validate_bundle(final_directory, &document)?;
            Ok(document)
        })
        .map_err(|error| {
            anyhow!(
                "deployment bundle conflict at {}: existing bundle is invalid: {error:#}",
                final_directory.display()
            )
        })?;
    if existing != *candidate {
        bail!(
            "deployment bundle conflict at {}: existing document differs from candidate",
            final_directory.display()
        );
    }
    Ok(final_directory.join("deployment.json"))
}

/// Keeps RTI paths separate even when a real Federate is named `rti`.
fn federate_directory(category: &str, federate: &str, central: bool) -> String {
    if central {
        format!("{category}/federates/{federate}")
    } else {
        format!("{category}/{federate}")
    }
}

/// Validates the association between coordination identity and RTI ownership.
fn validate_coordination(document: &DeploymentDocument) -> Result<()> {
    match document.coordination.backend.as_str() {
        "local" => {
            if document.rti.is_some()
                || document.coordination.identity.is_some()
                || document.coordination.protocol.is_some()
                || document.coordination.wire.is_some()
            {
                bail!("local coordination has unexpected RTI or protocol identity");
            }
        }
        "central-rti" => {
            let rti = document
                .rti
                .as_ref()
                .context("central coordination is missing RTI metadata")?;
            if !document
                .coordination
                .identity
                .as_deref()
                .is_some_and(|identity| is_lower_hex(identity, 64))
            {
                bail!("central coordination requires a valid RTI deployment identity");
            }
            if !document
                .coordination
                .protocol
                .as_deref()
                .is_some_and(|protocol| !protocol.is_empty())
            {
                bail!("central coordination requires an RTI protocol identity");
            }
            if let Some(wire) = &document.coordination.wire {
                use boomerang_federated::wire::{
                    CODEC_VERSION, MAX_PAYLOAD_BYTES, PROTOCOL_VERSION,
                };
                if wire.protocol != PROTOCOL_VERSION
                    || wire.codec != CODEC_VERSION
                    || wire.max_payload_bytes != MAX_PAYLOAD_BYTES
                    || !is_lower_hex(&wire.mapping, 64)
                {
                    bail!("unsupported portable wire profile");
                }
            }
            if rti.target.is_empty() {
                bail!("RTI compilation target must not be empty");
            }
            if let Some(profile) = &rti.profile {
                validate_segment(profile, "RTI profile")?;
            }
        }
        backend => bail!("unsupported deployment coordination backend {backend}"),
    }
    Ok(())
}

/// Validates schema identity, safe unique paths, and every referenced file hash.
fn validate_bundle(bundle: &Path, document: &DeploymentDocument) -> Result<()> {
    if document.schema != DEPLOYMENT_SCHEMA {
        bail!("unsupported deployment schema {}", document.schema);
    }
    if document.compiler_schema != COMPILER_SCHEMA {
        bail!("unsupported compiler schema {}", document.compiler_schema);
    }
    validate_fingerprint(&document.fingerprint)?;
    validate_segment(&document.deployment, "deployment")?;
    validate_coordination(document)?;
    let mut federates = BTreeSet::new();
    for federate in &document.federates {
        validate_segment(&federate.id, "Federate")?;
        if let Some(identity) = &federate.image_fingerprint {
            validate_fingerprint(identity)?;
        }
        if !federates.insert(federate.id.as_str()) {
            bail!("duplicate Federate record {}", federate.id);
        }
    }
    if federates.is_empty() {
        bail!("deployment document has no Federate records");
    }
    let metadata = fs::symlink_metadata(bundle)
        .with_context(|| format!("failed to inspect {}", bundle.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular bundle directory", bundle.display());
    }
    let canonical_bundle = fs::canonicalize(bundle)
        .with_context(|| format!("failed to canonicalize {}", bundle.display()))?;
    let mut paths = BTreeSet::new();
    let mut artifact_owners = BTreeSet::new();
    let mut files = Vec::new();
    for (category, records) in [
        ("generated", document.generated.as_slice()),
        ("artifacts", document.artifacts.as_slice()),
    ] {
        if records.is_empty() {
            bail!("deployment document has no {category} file records");
        }
        for record in records {
            validate_record(category, record, document.rti.is_some())?;
            if !federates.contains(record.federate.as_str()) {
                bail!(
                    "{category} file is owned by unknown Federate {}",
                    record.federate
                );
            }
            if category == "artifacts" && !artifact_owners.insert(record.federate.as_str()) {
                bail!(
                    "Federate {} owns multiple artifact records",
                    record.federate
                );
            }
            files.push((record.path.as_str(), record.blake3.as_str()));
        }
    }
    if artifact_owners != federates {
        bail!("each compiled Federate must own exactly one artifact record");
    }
    if let Some(rti) = &document.rti {
        let artifact = rti
            .artifact
            .as_ref()
            .context("central RTI is missing its artifact record")?;
        for (category, record) in rti
            .generated
            .iter()
            .map(|record| ("generated", record))
            .chain(std::iter::once(("artifacts", artifact)))
        {
            validate_file_record(category, "rti", &record.path, &record.blake3)?;
            files.push((record.path.as_str(), record.blake3.as_str()));
        }
    }
    let mut expected_files = BTreeSet::from([PathBuf::from("deployment.json")]);
    let mut expected_directories = BTreeSet::new();
    for (relative, digest) in files {
        if !paths.insert(relative) {
            bail!("duplicate bundle path {relative}");
        }
        let relative_path = join_normalized(Path::new(""), relative)?;
        for directory in relative_path.ancestors().skip(1) {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(directory.to_path_buf());
        }
        let path = bundle.join(&relative_path);
        expected_files.insert(relative_path);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("{} is not a regular bundle file", path.display());
        }
        let canonical_path = fs::canonicalize(&path)
            .with_context(|| format!("failed to canonicalize {}", path.display()))?;
        if !canonical_path.starts_with(&canonical_bundle) {
            bail!("bundle path {relative} escapes its fingerprint directory");
        }
        if hash_file(&path)? != digest {
            bail!("bundle hash mismatch for {relative}");
        }
    }
    for federate in federates {
        let directory = federate_directory("generated", federate, document.rti.is_some());
        for relative in ["Cargo.toml", "Cargo.lock", "src/main.rs"] {
            let path = format!("{directory}/{relative}");
            if !paths.contains(path.as_str()) {
                bail!("Federate {federate} is missing generated workspace record {path}");
            }
        }
    }
    if document.rti.is_some() {
        for relative in ["Cargo.toml", "Cargo.lock", "src/main.rs"] {
            let path = format!("generated/rti/{relative}");
            if !paths.contains(path.as_str()) {
                bail!("RTI is missing generated workspace record {path}");
            }
        }
    }
    validate_bundle_tree(bundle, &expected_files, &expected_directories)?;
    Ok(())
}

/// Validates that the bundle contains exactly the documented files and their directories.
fn validate_bundle_tree(
    bundle: &Path,
    expected_files: &BTreeSet<PathBuf>,
    expected_directories: &BTreeSet<PathBuf>,
) -> Result<()> {
    let mut actual_files = BTreeSet::new();
    let mut actual_directories = BTreeSet::new();
    validate_bundle_directory(
        bundle,
        Path::new(""),
        expected_files,
        expected_directories,
        &mut actual_files,
        &mut actual_directories,
    )?;
    if let Some(path) = expected_files.difference(&actual_files).next() {
        bail!("bundle is missing file {}", path.display());
    }
    if let Some(path) = expected_directories.difference(&actual_directories).next() {
        bail!("bundle is missing directory {}", path.display());
    }
    Ok(())
}

fn validate_bundle_directory(
    bundle: &Path,
    relative: &Path,
    expected_files: &BTreeSet<PathBuf>,
    expected_directories: &BTreeSet<PathBuf>,
    actual_files: &mut BTreeSet<PathBuf>,
    actual_directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let directory = bundle.join(relative);
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("failed to read bundle directory {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", directory.display()))?;
        let name = entry.file_name().into_string().map_err(|name| {
            anyhow!(
                "bundle entry {} is not valid UTF-8",
                directory.join(name).display()
            )
        })?;
        let child = relative.join(name);
        let metadata = fs::symlink_metadata(entry.path())
            .with_context(|| format!("failed to inspect bundle entry {}", child.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            bail!("bundle contains symbolic link {}", child.display());
        }
        if file_type.is_file() {
            if !expected_files.contains(&child) {
                bail!("bundle contains unreferenced file {}", child.display());
            }
            actual_files.insert(child);
        } else if file_type.is_dir() {
            if !expected_directories.contains(&child) {
                bail!("bundle contains unreferenced directory {}", child.display());
            }
            actual_directories.insert(child.clone());
            validate_bundle_directory(
                bundle,
                &child,
                expected_files,
                expected_directories,
                actual_files,
                actual_directories,
            )?;
        } else {
            bail!("bundle contains unsupported entry {}", child.display());
        }
    }
    Ok(())
}

/// Validates the category, owner, normalized path, and digest of one file record.
fn validate_record(category: &str, record: &FileRecord, central: bool) -> Result<()> {
    validate_segment(&record.federate, "Federate")?;
    let owner = if central {
        format!("federates/{}", record.federate)
    } else {
        record.federate.to_owned()
    };
    validate_file_record(category, &owner, &record.path, &record.blake3)
}

/// Validates exact role-specific path namespaces before any filesystem access.
fn validate_file_record(category: &str, owner: &str, path: &str, digest: &str) -> Result<()> {
    if !is_lower_hex(digest, 64) {
        bail!("invalid BLAKE3 hash for {path}");
    }
    if !path.starts_with(&format!("{category}/{owner}/")) {
        bail!("{category} path {path} does not belong to {owner}");
    }
    join_normalized(Path::new("."), path)?;
    Ok(())
}

/// Joins a portable `/`-normalized relative path after rejecting traversal.
fn join_normalized(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    if relative.is_empty() || relative.starts_with('/') || relative.contains('\\') {
        bail!("unsafe bundle-relative path {relative:?}");
    }
    for segment in relative.split('/') {
        validate_segment(segment, "bundle path")?;
        path.push(segment);
    }
    Ok(path)
}

/// Rejects empty, traversal, or path-separated identifiers.
fn validate_segment(value: &str, description: &str) -> Result<()> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.ends_with(['.', ' '])
        || value.chars().any(|character| {
            character <= '\u{1f}'
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
        || windows_reserved_component(value)
    {
        bail!("unsafe {description} path component {value:?}");
    }
    Ok(())
}

/// Returns whether a component has a DOS device basename reserved by Windows.
fn windows_reserved_component(value: &str) -> bool {
    let basename = value.split('.').next().unwrap_or_default();
    let basename = basename.to_ascii_uppercase();
    matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || basename
            .strip_prefix("COM")
            .or_else(|| basename.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
}

/// Validates the lowercase hexadecimal deployment fingerprint.
fn validate_fingerprint(fingerprint: &str) -> Result<()> {
    if !is_lower_hex(fingerprint, 64) {
        bail!("invalid deployment fingerprint {fingerprint:?}");
    }
    Ok(())
}

/// Returns whether text has the requested lowercase hexadecimal width.
fn is_lower_hex(value: &str, width: usize) -> bool {
    value.len() == width
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Hashes exact file bytes as lowercase BLAKE3 text.
fn hash_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    hash_open_file(&mut file)
}

/// Computes a BLAKE3 digest from the bytes held by an open file handle.
fn hash_open_file(file: &mut File) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .context("failed to read opened artifact")?;
        if read == 0 {
            return Ok(hasher.finalize().to_hex().to_string());
        }
        hasher.update(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layered claims must be accepted and retained independently in bundle metadata.
    #[test]
    fn bundle_retains_federate_image_claim() {
        let mut legacy = sample_document();
        legacy.federates[0].image_fingerprint = None;
        let mut value = serde_json::to_value(&legacy).unwrap();
        assert!(value["federates"][0].get("image_fingerprint").is_none());
        let decoded: DeploymentDocument = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            deployment_fingerprint(&legacy).unwrap(),
            deployment_fingerprint(&decoded).unwrap()
        );
        value["federates"][0]["image_fingerprint"] = serde_json::json!("ab".repeat(32));
        let decoded: DeploymentDocument = serde_json::from_value(value).unwrap();
        assert_eq!(
            decoded.federates[0].image_fingerprint.as_deref(),
            Some("ab".repeat(32).as_str())
        );
        assert_ne!(
            deployment_fingerprint(&legacy).unwrap(),
            deployment_fingerprint(&decoded).unwrap()
        );
        assert_eq!(legacy.coordination, decoded.coordination);
    }

    fn sample_document() -> DeploymentDocument {
        let mut document: DeploymentDocument = serde_json::from_value(serde_json::json!({
            "schema": 1,
            "compiler_schema": 1,
            "deployment": "test",
            "fingerprint": "",
            "topology_hash": "11".repeat(32),
            "source_lock_hash": "22".repeat(32),
            "generated_lock_hash": "33".repeat(32),
            "generated_source_hash": "44".repeat(32),
            "bindings": [],
            "federates": [{
                "id": "host",
                "image_fingerprint": "66".repeat(32),
                "groups": [],
                "target": target_lexicon::HOST.to_string(),
                "toolchain": null,
                "profile": null,
                "runtime": "std",
                "target_json_hash": null,
                "cargo_config_hash": null
            }],
            "execution": {
                "fast_forward": false,
                "keep_alive": false,
                "logical_horizon_nanos": null
            },
            "resources": { "federates": [] },
            "coordination": { "backend": "local", "protocol": null },
            "generated": [],
            "artifacts": []
        }))
        .unwrap();
        document.fingerprint = deployment_fingerprint(&document).unwrap();
        document
    }

    fn central_document() -> DeploymentDocument {
        let mut document = sample_document();
        document.federates[0].id = "rti".into();
        document.coordination = CoordinationDocument {
            backend: "central-rti".into(),
            protocol: Some("boomerang.coordination.v1".into()),
            identity: Some("55".repeat(32)),
            wire: None,
        };
        document.rti = Some(RtiDocument {
            target: target_lexicon::HOST.to_string(),
            profile: None,
            generated: Vec::new(),
            artifact: None,
        });
        document.fingerprint = deployment_fingerprint(&document).unwrap();
        document
    }

    /// Changing the RTI target/profile or shared handshake identity invalidates bundle reuse.
    #[test]
    fn central_rti_compilation_and_coordination_affect_fingerprint() {
        let document = central_document();
        let original = deployment_fingerprint(&document).unwrap();
        for field in ["target", "profile", "identity", "protocol"] {
            let mut changed = document.clone();
            match field {
                "target" => changed.rti.as_mut().unwrap().target = "other-target".into(),
                "profile" => changed.rti.as_mut().unwrap().profile = Some("release".into()),
                "identity" => changed.coordination.identity = Some("66".repeat(32)),
                "protocol" => changed.coordination.protocol = Some("other-protocol".into()),
                _ => unreachable!(),
            }
            assert_ne!(
                original,
                deployment_fingerprint(&changed).unwrap(),
                "{field}"
            );
        }
    }

    /// A central deployment cannot accidentally publish only its Federate artifacts.
    #[test]
    fn central_rti_publication_requires_separate_source() {
        let target = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let [manifest, lockfile, source, executable] = write_sample_source_files(inputs.path());
        let error = publish_bundle(
            target.path(),
            central_document(),
            &[BundleSource {
                federate: "rti",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            }],
        )
        .unwrap_err();
        assert!(error.to_string().contains("RTI source"), "{error:#}");
    }

    fn publish_central_fixture(target: &Path) -> PublishedBundle {
        let inputs = tempfile::tempdir().unwrap();
        let [manifest, lockfile, source, executable] = write_sample_source_files(inputs.path());
        let rti_executable = inputs.path().join("rti-launcher");
        fs::write(&rti_executable, b"RTI fixture").unwrap();
        let mut document = central_document();
        let mut sensor = document.federates[0].clone();
        sensor.id = "sensor".into();
        document.federates.push(sensor);
        document.fingerprint = deployment_fingerprint(&document).unwrap();
        let sources = ["rti", "sensor"].map(|federate| BundleSource {
            federate,
            manifest: &manifest,
            lockfile: &lockfile,
            source: &source,
            executable: &executable,
        });
        publish_bundle_with_rti(
            target,
            document,
            &sources,
            Some(&RtiBundleSource {
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &rti_executable,
            }),
        )
        .unwrap()
    }

    /// RTI and real Federate rti bytes survive publication, reuse and opened-handle loading.
    #[test]
    fn central_rti_bundle_roundtrip_preserves_distinct_executables() {
        let target = tempfile::tempdir().unwrap();
        let published = publish_central_fixture(target.path());
        assert_eq!(published.executables().len(), 2);
        assert_eq!(published.executables()[0].federate(), "rti");
        assert!(published.executables()[0]
            .path()
            .ends_with("artifacts/federates/rti/launcher"));
        assert!(published
            .rti_executable()
            .unwrap()
            .ends_with("artifacts/rti/rti-launcher"));
        let manifest = published.into_manifest();
        assert_eq!(
            publish_central_fixture(target.path()).into_manifest(),
            manifest
        );
        let mut loaded = load_published_artifacts(&manifest).unwrap();
        assert_eq!(loaded.document.federates.len(), 2);
        assert_eq!(
            loaded
                .federates
                .iter()
                .map(|artifact| artifact.federate.as_str())
                .collect::<Vec<_>>(),
            ["rti", "sensor"]
        );
        for artifact in &mut loaded.federates {
            let mut contents = String::new();
            artifact.executable.read_to_string(&mut contents).unwrap();
            assert_eq!(contents, "fixture");
            assert_eq!(
                artifact.executable_hash,
                blake3::hash(b"fixture").to_hex().as_str()
            );
        }
        let rti = loaded.rti.as_mut().unwrap();
        let mut contents = String::new();
        rti.executable.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "RTI fixture");
        assert_eq!(
            rti.executable_hash,
            blake3::hash(b"RTI fixture").to_hex().as_str()
        );
        assert_eq!(loaded.document.rti.as_ref().unwrap().generated.len(), 3);
        assert!(load_published_artifact(&manifest).is_err());
    }

    /// Every RTI record is checked before any process can be launched.
    #[test]
    fn central_rti_loader_rejects_missing_wrong_duplicate_and_tampered_records() {
        let cases = [
            ("missing metadata", "missing RTI metadata"),
            ("missing executable record", "missing its artifact record"),
            ("wrong artifact role", "does not belong to rti"),
            ("duplicate generated", "duplicate bundle path"),
            ("missing generated", "generated workspace record"),
            ("unexpected rti", "unexpected RTI"),
            ("bad identity", "valid RTI deployment identity"),
            ("bad protocol", "RTI protocol identity"),
            ("changed target", "semantic fingerprint mismatch"),
            ("tampered executable", "bundle hash mismatch"),
            ("absent executable", "failed to inspect"),
        ];
        for (case, expected) in cases {
            let target = tempfile::tempdir().unwrap();
            let manifest = publish_central_fixture(target.path()).into_manifest();
            let bundle = manifest.parent().unwrap();
            let mut document = read_document(bundle).unwrap();
            match case {
                "missing metadata" => document.rti = None,
                "missing executable record" => document.rti.as_mut().unwrap().artifact = None,
                "wrong artifact role" => {
                    document
                        .rti
                        .as_mut()
                        .unwrap()
                        .artifact
                        .as_mut()
                        .unwrap()
                        .path = "artifacts/federates/rti/launcher".into()
                }
                "duplicate generated" => {
                    let rti = document.rti.as_mut().unwrap();
                    rti.generated.push(rti.generated[0].clone());
                }
                "missing generated" => {
                    let record = document.rti.as_mut().unwrap().generated.remove(0);
                    fs::remove_file(bundle.join(record.path)).unwrap();
                }
                "unexpected rti" => document.coordination.backend = "local".into(),
                "bad identity" => document.coordination.identity = Some("invalid".into()),
                "bad protocol" => document.coordination.protocol = None,
                "changed target" => document.rti.as_mut().unwrap().target = "other-target".into(),
                "tampered executable" => {
                    let path = &document
                        .rti
                        .as_ref()
                        .unwrap()
                        .artifact
                        .as_ref()
                        .unwrap()
                        .path;
                    fs::write(bundle.join(path), b"tampered").unwrap();
                }
                "absent executable" => {
                    let path = &document
                        .rti
                        .as_ref()
                        .unwrap()
                        .artifact
                        .as_ref()
                        .unwrap()
                        .path;
                    fs::remove_file(bundle.join(path)).unwrap();
                }
                _ => unreachable!(),
            }
            write_document(bundle, &document).unwrap();
            let error = load_published_artifacts(&manifest).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{case}: {error:#}");
        }
    }

    fn write_sample_bundle(root: &Path) -> DeploymentDocument {
        let mut document = sample_document();
        let contents = b"fixture";
        let digest = blake3::hash(contents).to_hex().to_string();
        document.generated = [
            "generated/host/Cargo.toml",
            "generated/host/Cargo.lock",
            "generated/host/src/main.rs",
        ]
        .into_iter()
        .map(|path| FileRecord {
            federate: String::from("host"),
            path: String::from(path),
            blake3: digest.clone(),
        })
        .collect();
        document.artifacts = vec![FileRecord {
            federate: String::from("host"),
            path: String::from("artifacts/host/launcher"),
            blake3: digest,
        }];
        for record in document.generated.iter().chain(&document.artifacts) {
            let path = join_normalized(root, &record.path).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        write_document(root, &document).unwrap();
        document
    }

    fn write_sample_source_files(root: &Path) -> [PathBuf; 4] {
        let files = [
            root.join("Cargo.toml"),
            root.join("Cargo.lock"),
            root.join("main.rs"),
            root.join("launcher"),
        ];
        for path in &files {
            fs::write(path, b"fixture").unwrap();
        }
        files
    }

    #[test]
    fn portable_components_reject_windows_prefixes_and_reserved_names() {
        for unsafe_component in [
            "C:",
            "CON",
            "nul.txt",
            "COM¹",
            "com¹",
            "COM².txt",
            "com².TXT",
            "LPT³",
            "lpt³",
            "trailing.",
            "trailing ",
            "bad?name",
        ] {
            assert!(
                validate_segment(unsafe_component, "test").is_err(),
                "accepted unsafe portable component {unsafe_component:?}"
            );
        }
        validate_segment("host-a_1", "test").unwrap();
    }

    #[test]
    fn no_replace_rename_preserves_an_existing_empty_destination() {
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("source");
        let destination = parent.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("source-marker"), b"source").unwrap();
        fs::create_dir(&destination).unwrap();

        let error = rename_noreplace(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(source.join("source-marker").is_file());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    }

    #[test]
    fn publication_conflicts_without_overwriting_an_existing_empty_destination() {
        let target = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let [manifest, lockfile, source, executable] = write_sample_source_files(inputs.path());
        let document = sample_document();
        let destination = target
            .path()
            .join("boomerang/test")
            .join(&document.fingerprint);
        fs::create_dir_all(&destination).unwrap();

        let error = publish_bundle(
            target.path(),
            document,
            &[BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains("conflict"), "{error:#}");
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    }

    #[test]
    fn publication_reuses_a_complete_identical_candidate_without_staging_remnants() {
        let target = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let [manifest, lockfile, source, executable] = write_sample_source_files(inputs.path());
        let document = sample_document();
        let fingerprint = document.fingerprint.clone();
        let first = publish_bundle(
            target.path(),
            document.clone(),
            &[BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            }],
        )
        .unwrap()
        .into_manifest();
        let bundle = first.parent().unwrap();
        let manifest_before = fs::read(&first).unwrap();
        let artifact = bundle.join("artifacts/host/launcher");
        let artifact_before = fs::read(&artifact).unwrap();

        let second = publish_bundle(
            target.path(),
            document,
            &[BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            }],
        )
        .unwrap()
        .into_manifest();

        assert_eq!(second, first);
        assert_eq!(fs::read(&first).unwrap(), manifest_before);
        assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        let mut loaded = load_published_artifact(&first).unwrap();
        assert_eq!(loaded.document, read_document(bundle).unwrap());
        let mut loaded_bytes = Vec::new();
        loaded.executable.read_to_end(&mut loaded_bytes).unwrap();
        assert_eq!(loaded_bytes, b"fixture");
        assert_eq!(
            loaded.executable_hash,
            blake3::hash(b"fixture").to_hex().as_str()
        );
        let staging_prefix = format!(".{fingerprint}.staging-");
        assert!(fs::read_dir(bundle.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&staging_prefix)));
    }

    #[test]
    fn deployment_schema_rejects_unknown_top_level_and_nested_fields() {
        let document = sample_document();
        let mut top_level = serde_json::to_value(&document).unwrap();
        top_level["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<DeploymentDocument>(top_level).is_err());

        let mut nested = serde_json::to_value(document).unwrap();
        nested["execution"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<DeploymentDocument>(nested).is_err());
    }

    #[test]
    fn fingerprint_input_serializes_the_v1_deployment_domain_first() {
        let document = sample_document();
        let input = FingerprintInputV1 {
            domain: DEPLOYMENT_FINGERPRINT_DOMAIN_V1,
            schema: document.schema,
            compiler_schema: document.compiler_schema,
            topology_hash: &document.topology_hash,
            bindings: &document.bindings,
            source_lock_hash: &document.source_lock_hash,
            generated_lock_hash: &document.generated_lock_hash,
            generated_source_hash: &document.generated_source_hash,
            federates: &document.federates,
            rti: document.rti.as_ref().map(RtiFingerprintInput::from),
            execution: &document.execution,
            resources: &document.resources,
            coordination: &document.coordination,
        };

        assert!(serde_json::to_string(&input)
            .unwrap()
            .starts_with(r#"{"domain":"boomerang.deployment.v1","schema":1"#));
    }

    #[test]
    fn bundle_validation_rejects_every_unreferenced_tree_entry() {
        let bundle = tempfile::tempdir().unwrap();
        let document = write_sample_bundle(bundle.path());
        validate_bundle(bundle.path(), &document).unwrap();

        let extra_file = bundle.path().join("extra-file");
        fs::write(&extra_file, b"extra").unwrap();
        assert!(validate_bundle(bundle.path(), &document).is_err());
        fs::remove_file(extra_file).unwrap();

        let extra_directory = bundle.path().join("extra-directory");
        fs::create_dir(&extra_directory).unwrap();
        assert!(validate_bundle(bundle.path(), &document).is_err());
        fs::remove_dir(extra_directory).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let extra_symlink = bundle.path().join("extra-symlink");
            symlink("deployment.json", &extra_symlink).unwrap();
            assert!(validate_bundle(bundle.path(), &document).is_err());
        }
    }

    #[test]
    fn published_loader_rejects_semantic_fingerprint_tampering() {
        let parent = tempfile::tempdir().unwrap();
        let document = sample_document();
        let bundle = parent.path().join(&document.fingerprint);
        fs::create_dir(&bundle).unwrap();
        let mut tampered = write_sample_bundle(&bundle);
        tampered.execution.keep_alive = true;
        write_document(&bundle, &tampered).unwrap();

        let error = load_published_artifact(&bundle.join("deployment.json")).unwrap_err();

        assert!(
            error.to_string().contains("fingerprint mismatch"),
            "{error:#}"
        );
    }

    #[test]
    fn published_loader_rejects_an_unsupported_compiler_schema() {
        let parent = tempfile::tempdir().unwrap();
        let staging = parent.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let mut document = write_sample_bundle(&staging);
        document.compiler_schema = 2;
        document.fingerprint = deployment_fingerprint(&document).unwrap();
        write_document(&staging, &document).unwrap();
        let bundle = parent.path().join(&document.fingerprint);
        fs::rename(&staging, &bundle).unwrap();

        let error = load_published_artifact(&bundle.join("deployment.json")).unwrap_err();

        assert!(
            error.to_string().contains("unsupported compiler schema 2"),
            "{error:#}"
        );
    }

    #[test]
    fn published_loader_rejects_a_wrong_fingerprint_directory_name() {
        let parent = tempfile::tempdir().unwrap();
        let bundle = parent.path().join("ff".repeat(32));
        fs::create_dir(&bundle).unwrap();
        write_sample_bundle(&bundle);

        let error = load_published_artifact(&bundle.join("deployment.json")).unwrap_err();

        assert!(error.to_string().contains("directory name"), "{error:#}");
    }

    /// The compatibility loader never silently discards artifacts in a collection.
    #[test]
    fn single_artifact_loader_rejects_a_collection() {
        let parent = tempfile::tempdir().unwrap();
        let mut document = sample_document();
        let mut sensor = document.federates[0].clone();
        sensor.id = String::from("sensor");
        document.federates.push(sensor);
        document.fingerprint = deployment_fingerprint(&document).unwrap();
        let bundle = parent.path().join(&document.fingerprint);
        fs::create_dir(&bundle).unwrap();
        let contents = b"fixture";
        let digest = blake3::hash(contents).to_hex().to_string();
        document.generated = ["host", "sensor"]
            .into_iter()
            .flat_map(|federate| {
                let digest = digest.clone();
                ["Cargo.toml", "Cargo.lock", "src/main.rs"]
                    .into_iter()
                    .map(move |path| FileRecord {
                        federate: federate.to_owned(),
                        path: format!("generated/{federate}/{path}"),
                        blake3: digest.clone(),
                    })
            })
            .collect();
        document.artifacts = ["host", "sensor"]
            .into_iter()
            .map(|federate| FileRecord {
                federate: federate.to_owned(),
                path: format!("artifacts/{federate}/launcher"),
                blake3: digest.clone(),
            })
            .collect();
        for record in document.generated.iter().chain(&document.artifacts) {
            let path = join_normalized(&bundle, &record.path).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        write_document(&bundle, &document).unwrap();

        let error = load_published_artifact(&bundle.join("deployment.json")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "single-artifact loader requires one local Federate"
        );
    }

    /// Rejects invalid ownership, ordering, and multiplicity at collection publication.
    #[test]
    fn collection_boundary_rejects_invalid_ownership_and_collisions() {
        let cases = [
            ("source count", "source count"),
            ("source order", "canonical Federate order"),
            ("unknown owner", "unknown Federate"),
            ("duplicate path", "duplicate bundle path"),
            ("missing generated workspace", "generated workspace record"),
            ("missing artifact", "exactly one artifact record"),
            ("multiple artifacts", "multiple artifact records"),
        ];
        let mut outcomes = Vec::new();
        for (case, expected) in cases {
            let result = if case.starts_with("source") {
                let target = tempfile::tempdir().unwrap();
                let inputs = tempfile::tempdir().unwrap();
                let [manifest, lockfile, source, executable] =
                    write_sample_source_files(inputs.path());
                let source = BundleSource {
                    federate: "sensor",
                    manifest: &manifest,
                    lockfile: &lockfile,
                    source: &source,
                    executable: &executable,
                };
                let sources = if case == "source count" {
                    &[][..]
                } else {
                    &[source]
                };
                publish_bundle(target.path(), sample_document(), sources).map(|_| ())
            } else {
                let bundle = tempfile::tempdir().unwrap();
                let mut document = write_sample_bundle(bundle.path());
                match case {
                    "unknown owner" => {
                        let path = "generated/sensor/Cargo.toml";
                        fs::create_dir_all(bundle.path().join("generated/sensor")).unwrap();
                        fs::rename(
                            bundle.path().join(&document.generated[0].path),
                            bundle.path().join(path),
                        )
                        .unwrap();
                        document.generated[0].federate = "sensor".into();
                        document.generated[0].path = path.into();
                    }
                    "duplicate path" => {
                        document.generated[1].path = document.generated[0].path.clone();
                    }
                    "missing generated workspace" => {
                        let missing = document.generated.remove(1);
                        fs::remove_file(bundle.path().join(missing.path)).unwrap();
                    }
                    "missing artifact" => {
                        let mut sensor = document.federates[0].clone();
                        sensor.id = "sensor".into();
                        document.federates.push(sensor);
                    }
                    "multiple artifacts" => {
                        let path = "artifacts/host/second";
                        fs::write(bundle.path().join(path), b"fixture").unwrap();
                        document.artifacts.push(FileRecord {
                            federate: "host".into(),
                            path: path.into(),
                            blake3: blake3::hash(b"fixture").to_hex().to_string(),
                        });
                    }
                    _ => unreachable!(),
                }
                validate_bundle(bundle.path(), &document)
            };
            outcomes.push((case, result.err().map(|error| error.to_string()), expected));
        }
        assert!(
            outcomes.iter().all(|(_, error, expected)| error
                .as_deref()
                .is_some_and(|error| error.contains(expected))),
            "{outcomes:#?}"
        );
    }
}
