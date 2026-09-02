//! Verified construction and failure-atomic publication of deployment bundles.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::check::ResourceReport;

/// Current public deployment-document schema.
pub(crate) const DEPLOYMENT_SCHEMA: u32 = 1;

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
    /// Lowercase BLAKE3 deployment fingerprint naming the bundle directory.
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
    /// Runtime settings embedded in every generated launcher.
    pub(crate) runtime_configuration: RuntimeConfigurationDocument,
    /// Canonical statically computed resource bounds.
    pub(crate) resources: ResourceReport,
    /// Selected coordination backend and protocol identity.
    pub(crate) coordination: CoordinationDocument,
    /// Generated source files retained for audit and reproduction.
    pub(crate) generated: Vec<FileRecord>,
    /// Executable files ready for deployment.
    pub(crate) artifacts: Vec<FileRecord>,
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

/// Runtime configuration embedded literally in generated launchers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeConfigurationDocument {
    /// Whether logical execution bypasses wall-clock synchronization.
    pub(crate) fast_forward: bool,
    /// Whether schedulers remain alive without pending events.
    pub(crate) keep_alive: bool,
    /// Maximum physical events buffered by each runtime environment.
    pub(crate) physical_event_q_size: usize,
    /// Optional runtime timeout in nanoseconds.
    pub(crate) timeout_nanos: Option<u64>,
}

/// Coordination selection recorded for external deployment tooling.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoordinationDocument {
    /// Selected backend identity, including `local` for one Federate.
    pub(crate) backend: String,
    /// Versioned protocol identity, absent for local coordination.
    pub(crate) protocol: Option<String>,
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

/// Stages, verifies, and atomically publishes an immutable deployment bundle.
pub(crate) fn publish_bundle(
    target_directory: &Path,
    mut document: DeploymentDocument,
    source: BundleSource<'_>,
) -> Result<PathBuf> {
    validate_segment(&document.deployment, "deployment")?;
    validate_fingerprint(&document.fingerprint)?;
    validate_segment(source.federate, "Federate")?;
    let executable_name = source
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("generated executable name is not valid UTF-8"))?;
    validate_segment(executable_name, "executable")?;

    let parent = target_directory
        .join("boomerang")
        .join(&document.deployment);
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{}.staging-", document.fingerprint))
        .tempdir_in(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;

    let generated = format!("generated/{}", source.federate);
    document.generated = vec![
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
    ];

    document.artifacts = vec![stage_file(
        staging.path(),
        source.federate,
        source.executable,
        &format!("artifacts/{}/{executable_name}", source.federate),
    )?];

    write_document(staging.path(), &document)?;
    let decoded = read_document(staging.path())?;
    if decoded != document {
        bail!("staged deployment document changed during serialization");
    }
    validate_bundle(staging.path(), &decoded)?;

    let final_directory = parent.join(&document.fingerprint);
    match rename_noreplace(staging.path(), &final_directory) {
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
    }
}

/// Atomically renames a directory only when the destination does not exist.
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
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
    use std::os::windows::ffi::OsStrExt as _;

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
    Ok(FileRecord {
        federate: federate.to_owned(),
        path: relative.to_owned(),
        blake3: expected,
    })
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

/// Validates schema identity, safe unique paths, and every referenced file hash.
fn validate_bundle(bundle: &Path, document: &DeploymentDocument) -> Result<()> {
    if document.schema != DEPLOYMENT_SCHEMA {
        bail!("unsupported deployment schema {}", document.schema);
    }
    validate_fingerprint(&document.fingerprint)?;
    validate_segment(&document.deployment, "deployment")?;
    let metadata = fs::symlink_metadata(bundle)
        .with_context(|| format!("failed to inspect {}", bundle.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular bundle directory", bundle.display());
    }
    let canonical_bundle = fs::canonicalize(bundle)
        .with_context(|| format!("failed to canonicalize {}", bundle.display()))?;
    let mut paths = BTreeSet::new();
    let mut expected_files = BTreeSet::new();
    expected_files.insert(PathBuf::from("deployment.json"));
    let mut expected_directories = BTreeSet::new();
    for (category, records) in [
        ("generated", document.generated.as_slice()),
        ("artifacts", document.artifacts.as_slice()),
    ] {
        if records.is_empty() {
            bail!("deployment document has no {category} file records");
        }
        for record in records {
            validate_record(category, record)?;
            if !paths.insert(record.path.as_str()) {
                bail!("duplicate bundle path {}", record.path);
            }
            let relative = join_normalized(Path::new(""), &record.path)?;
            expected_files.insert(relative.clone());
            for directory in relative.ancestors().skip(1) {
                if directory.as_os_str().is_empty() {
                    break;
                }
                expected_directories.insert(directory.to_path_buf());
            }
            let path = bundle.join(relative);
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("failed to inspect {}", path.display()))?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                bail!("{} is not a regular bundle file", path.display());
            }
            let canonical_path = fs::canonicalize(&path)
                .with_context(|| format!("failed to canonicalize {}", path.display()))?;
            if !canonical_path.starts_with(&canonical_bundle) {
                bail!(
                    "bundle path {} escapes its fingerprint directory",
                    record.path
                );
            }
            let actual = hash_file(&path)?;
            if actual != record.blake3 {
                bail!("bundle hash mismatch for {}", record.path);
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
fn validate_record(category: &str, record: &FileRecord) -> Result<()> {
    validate_segment(&record.federate, "Federate")?;
    if !is_lower_hex(&record.blake3, 64) {
        bail!("invalid BLAKE3 hash for {}", record.path);
    }
    let parts = record.path.split('/').collect::<Vec<_>>();
    if parts.len() < 3 || parts[0] != category || parts[1] != record.federate {
        bail!(
            "{} path {} does not belong to Federate {}",
            category,
            record.path,
            record.federate
        );
    }
    join_normalized(Path::new("."), &record.path)?;
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
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_document() -> DeploymentDocument {
        serde_json::from_value(serde_json::json!({
            "schema": 1,
            "compiler_schema": 1,
            "deployment": "test",
            "fingerprint": "00".repeat(32),
            "topology_hash": "11".repeat(32),
            "source_lock_hash": "22".repeat(32),
            "generated_lock_hash": "33".repeat(32),
            "generated_source_hash": "44".repeat(32),
            "bindings": [],
            "federates": [],
            "runtime_configuration": {
                "fast_forward": false,
                "keep_alive": false,
                "physical_event_q_size": 1024,
                "timeout_nanos": null
            },
            "resources": { "federates": [] },
            "coordination": { "backend": "local", "protocol": null },
            "generated": [],
            "artifacts": []
        }))
        .unwrap()
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
        let destination = target.path().join("boomerang/test").join("00".repeat(32));
        fs::create_dir_all(&destination).unwrap();

        let error = publish_bundle(
            target.path(),
            sample_document(),
            BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            },
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
        let first = publish_bundle(
            target.path(),
            document.clone(),
            BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            },
        )
        .unwrap();
        let bundle = first.parent().unwrap();
        let manifest_before = fs::read(&first).unwrap();
        let artifact = bundle.join("artifacts/host/launcher");
        let artifact_before = fs::read(&artifact).unwrap();

        let second = publish_bundle(
            target.path(),
            document,
            BundleSource {
                federate: "host",
                manifest: &manifest,
                lockfile: &lockfile,
                source: &source,
                executable: &executable,
            },
        )
        .unwrap();

        assert_eq!(second, first);
        assert_eq!(fs::read(&first).unwrap(), manifest_before);
        assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        let published = read_document(bundle).unwrap();
        validate_bundle(bundle, &published).unwrap();
        let staging_prefix = format!(".{}.staging-", "00".repeat(32));
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
        nested["runtime_configuration"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<DeploymentDocument>(nested).is_err());
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
}
