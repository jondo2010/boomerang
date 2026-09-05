//! Persistent, content-addressed workspaces for generated Cargo crates.
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use crate::bundle::rename_noreplace;
use anyhow::{bail, ensure, Context, Result};
use cargo_metadata::{Artifact, PackageId, TargetKind};
use fs2::FileExt;
const GENERATED_CACHE_SCHEMA: u32 = 1;
/// The purpose of a generated Cargo workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GeneratedRole {
    /// Host-only executable that emits descriptor data for deployment analysis.
    Descriptor,
    /// Target executable that runs a fully lowered deployment image.
    Launcher,
}
impl GeneratedRole {
    fn directory_name(self) -> &'static str {
        match self {
            Self::Descriptor => "descriptor",
            Self::Launcher => "launcher",
        }
    }
}
/// Content identity of one generated-workspace request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestIdentity(blake3::Hash);
impl RequestIdentity {
    fn lowercase_hex(self) -> String {
        self.0.to_hex().to_string()
    }
    fn short_target_name(self, role: GeneratedRole) -> String {
        let prefix = match role {
            GeneratedRole::Descriptor => 'd',
            GeneratedRole::Launcher => 'l',
        };
        format!("{prefix}{}", &self.lowercase_hex()[..32])
    }
}
/// Canonically encodes the inputs to a generated-workspace request.
pub(crate) struct RequestIdentityBuilder(blake3::Hasher);
impl RequestIdentityBuilder {
    /// Starts an identity with the cache schema and generated-workspace role.
    pub(crate) fn new(role: GeneratedRole) -> Self {
        let mut builder = Self(blake3::Hasher::new());
        builder.field("schema", Some(&GENERATED_CACHE_SCHEMA.to_be_bytes()));
        builder.field("role", Some(role.directory_name().as_bytes()));
        builder
    }
    /// Appends one length-delimited optional request field using the canonical cache encoding.
    pub(crate) fn field(&mut self, label: &str, value: Option<&[u8]>) {
        let label_length = u64::try_from(label.len()).expect("request label length exceeds u64");
        self.0.update(&label_length.to_be_bytes());
        self.0.update(label.as_bytes());
        self.0.update(&[u8::from(value.is_some())]);
        if let Some(value) = value {
            let value_length = u64::try_from(value.len()).expect("request value too long");
            self.0.update(&value_length.to_be_bytes());
            self.0.update(value);
        }
    }
    /// Finishes canonical encoding and returns the resulting content identity.
    pub(crate) fn finish(self) -> RequestIdentity {
        RequestIdentity(self.0.finalize())
    }
}
/// Rendered sources and source-lock identity needed to prepare one generated workspace.
pub(crate) struct GeneratedWorkspaceRequest<'a> {
    /// Distinguishes descriptor and launcher cache namespaces.
    pub(crate) role: GeneratedRole,
    /// Complete content identity derived from every cache-relevant input.
    pub(crate) identity: RequestIdentity,
    /// Exact generated `Cargo.toml` bytes to publish on a cache miss.
    pub(crate) manifest: &'a [u8],
    /// Exact generated `src/main.rs` bytes to publish on a cache miss.
    pub(crate) source: &'a [u8],
    /// Canonical source-workspace lockfile copied before reconciliation on a cache miss.
    pub(crate) source_lockfile: &'a Path,
    /// BLAKE3 digest of the source lockfile retained in the published cache marker.
    pub(crate) source_lock_digest: &'a [u8; 32],
}
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
/// Serialized integrity record for one immutable generated workspace.
struct CacheRecord {
    /// Cache-format version used to reject incompatible records.
    schema: u32,
    /// Generated-workspace role whose namespace owns this record.
    role: GeneratedRole,
    /// Full lowercase request digest naming the workspace.
    request: String,
    /// BLAKE3 digest of the canonical generated manifest.
    manifest_hash: String,
    /// BLAKE3 digest of the canonical generated source.
    source_hash: String,
    /// BLAKE3 digest of the reconciled generated lockfile.
    generated_lock_hash: String,
    /// BLAKE3 digest of the source-workspace lock snapshot.
    source_lock_hash: String,
    /// Role-prefixed short target-directory locator.
    target_locator: String,
}
#[derive(Debug)]
/// Independently retained bytes and record used for later validation.
struct WorkspaceExpectations {
    /// Canonical generated manifest bytes.
    manifest: Box<[u8]>,
    /// Canonical generated source bytes.
    source: Box<[u8]>,
    /// Exact serialized integrity values expected on disk.
    record: CacheRecord,
}
impl WorkspaceExpectations {
    fn new(request: &GeneratedWorkspaceRequest<'_>, generated_lock_hash: String) -> Self {
        Self {
            manifest: request.manifest.into(),
            source: request.source.into(),
            record: CacheRecord {
                schema: GENERATED_CACHE_SCHEMA,
                role: request.role,
                request: request.identity.lowercase_hex(),
                manifest_hash: blake3::hash(request.manifest).to_string(),
                source_hash: blake3::hash(request.source).to_string(),
                generated_lock_hash,
                source_lock_hash: blake3::Hash::from(*request.source_lock_digest).to_string(),
                target_locator: request.identity.short_target_name(request.role),
            },
        }
    }
}
/// Filesystem locations that form one validated generated Cargo workspace.
#[derive(Debug)]
pub(crate) struct GeneratedWorkspace {
    /// Canonical immutable generated-workspace directory.
    directory: PathBuf,
    /// Canonical caller-selected target root.
    target_anchor: PathBuf,
    /// Independently retained integrity expectations.
    expectations: WorkspaceExpectations,
}
impl GeneratedWorkspace {
    /// Revalidates a published workspace and its locked Cargo graph.
    fn validate_published<T>(&self, validate_graph: &impl Fn(&Path) -> Result<T>) -> Result<T> {
        require_real_directory(&self.directory, "generated workspace")?;
        validate_canonical_containment(&self.target_anchor, &self.directory, "workspace")?;
        validate_workspace_contents(&self.directory, &self.expectations)?;
        validate_graph(self.directory())
    }
    /// Returns the published cache directory.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    /// Returns the generated Cargo manifest path.
    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.directory.join("Cargo.toml")
    }
    fn target_directory(&self) -> PathBuf {
        let target_locator = &self.expectations.record.target_locator;
        self.target_anchor.join("b").join(target_locator)
    }
    /// Serializes target-directory use and revalidates the published cache before an operation.
    pub(crate) fn with_locked_target<T>(
        &self,
        operation: impl FnOnce(&Path) -> Result<T>,
    ) -> Result<T> {
        validate_workspace_contents(&self.directory, &self.expectations)?;
        let target = self.target_directory();
        self.prepare_short_target(&target)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(target.join(".boomerang-request"))
            .context("failed to open generated target lock")?;
        lock.lock_exclusive()
            .context("failed to lock generated target")?;
        self.validate_target_marker(&target)?;
        validate_workspace_contents(&self.directory, &self.expectations)?;
        operation(&target)
    }
    fn prepare_short_target(&self, target: &Path) -> Result<()> {
        let parent = prepare_managed_directories(&self.target_anchor, &["b"])?;
        if target
            .try_exists()
            .context("failed to inspect generated target")?
        {
            return self.validate_target_marker(target);
        }
        let marker = &self.expectations.record;
        let bytes = serde_json::to_vec(marker).context("failed to encode target marker")?;
        publish_staged(
            &parent,
            target,
            |dir| {
                fs::write(dir.join(".boomerang-request"), &bytes)
                    .context("failed to write target marker")
            },
            |directory, ()| {
                let marker_path = directory.join(".boomerang-request");
                validate_exact_directory(directory, &[".boomerang-request"], "target staging")?;
                if read_cache_record(&marker_path, "generated target marker")? != *marker {
                    bail!("invalid target marker");
                }
                Ok(())
            },
        )?;
        self.validate_target_marker(target)
    }
    fn validate_target_marker(&self, target: &Path) -> Result<()> {
        require_real_directory(target, "generated target directory")?;
        validate_canonical_containment(&self.target_anchor, target, "generated target directory")?;
        let marker = read_cache_record(
            &target.join(".boomerang-request"),
            "generated target marker",
        )?;
        let expected = &self.expectations.record;
        if marker.request != expected.request {
            bail!("target locator collision");
        }
        ensure!(marker == *expected, "generated target marker is invalid");
        Ok(())
    }
}
/// Validates the exact workspace tree and all retained canonical contents.
fn validate_workspace_contents(
    directory: &Path,
    expectations: &WorkspaceExpectations,
) -> Result<()> {
    let entries = ["Cargo.toml", "Cargo.lock", "cache.json", "src"];
    validate_exact_directory(directory, &entries, "generated workspace")?;
    validate_exact_directory(&directory.join("src"), &["main.rs"], "generated workspace")?;
    let manifest = directory.join("Cargo.toml");
    let source = directory.join("src/main.rs");
    let lockfile = directory.join("Cargo.lock");
    let record = directory.join("cache.json");
    validate_canonical_file(&manifest, &expectations.manifest, "generated manifest")?;
    validate_canonical_file(&source, &expectations.source, "generated source")?;
    let expected = &expectations.record;
    if hash_file(&lockfile, "generated lockfile")? != expected.generated_lock_hash {
        bail!("generated lockfile changed");
    }
    let actual = read_cache_record(&record, "generated cache record")?;
    ensure!(actual == *expected, "generated cache record changed");
    Ok(())
}
fn hash_file(path: &Path, description: &str) -> Result<String> {
    Ok(blake3::hash(&read_regular_file(path, description)?).to_string())
}
/// Reads one regular source lockfile snapshot and verifies its retained digest.
fn read_verified_source_lock(path: &Path, expected_digest: &[u8; 32]) -> Result<Vec<u8>> {
    let bytes = read_regular_file(path, "source workspace lockfile")?;
    (blake3::hash(&bytes).as_bytes() == expected_digest)
        .then_some(bytes)
        .context("source lock digest changed")
}
/// Reads a file only after rejecting links, reparse points, and non-files.
fn read_regular_file(path: &Path, description: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata_is_reparse_point(&metadata),
        "{description} is not a real regular file"
    );
    fs::read(path).with_context(|| format!("failed to read {description} {}", path.display()))
}
fn require_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_dir() && !metadata_is_reparse_point(&metadata),
        "{description} is not a real directory"
    );
    Ok(())
}

/// Validates and privately copies a raw Cargo artifact while its target lock is held.
pub(crate) fn copy_private_artifact(
    path: &Path,
    target: &Path,
) -> Result<(tempfile::TempDir, PathBuf)> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect Cargo artifact {}", path.display()))?;
    ensure!(
        path.is_absolute() && metadata.is_file() && !metadata_is_reparse_point(&metadata),
        "Cargo artifact {} is not a raw regular file",
        path.display()
    );
    let target = fs::canonicalize(target).context("failed to canonicalize locked target")?;
    let artifact = validate_canonical_containment(&target, path, "Cargo artifact")?;
    let private = tempfile::Builder::new()
        .prefix(".boomerang-artifact-")
        .tempdir_in(target)
        .context("failed to prepare private artifact directory")?;
    let filename = artifact
        .file_name()
        .context("Cargo artifact has no file name")?;
    let destination = private.path().join(filename);
    let source_hash = hash_file(&artifact, "Cargo artifact")?;
    fs::copy(&artifact, &destination)
        .with_context(|| format!("failed to copy Cargo artifact {}", artifact.display()))?;
    ensure!(
        source_hash == hash_file(&destination, "private artifact")?,
        "private copy differs from Cargo artifact"
    );
    Ok((private, destination))
}

/// Selects only a Cargo artifact with the exact generated package, manifest, and binary identity.
pub(crate) fn artifact_matches(
    artifact: &Artifact,
    package: &PackageId,
    manifest: &Path,
    binary: &str,
) -> Result<bool> {
    let same_package = artifact.package_id == *package;
    let same_manifest = fs::canonicalize(artifact.manifest_path.as_std_path())
        .context("failed to canonicalize Cargo artifact manifest")?
        == fs::canonicalize(manifest).context("failed to canonicalize generated manifest")?;
    if !same_package && !same_manifest {
        return Ok(false);
    }
    ensure!(
        same_package
            && same_manifest
            && artifact.target.name == binary
            && artifact.target.kind == [TargetKind::Bin],
        "generated Cargo artifact identity mismatch"
    );
    Ok(true)
}
fn read_cache_record(path: &Path, description: &str) -> Result<CacheRecord> {
    serde_json::from_slice(&read_regular_file(path, description)?)
        .with_context(|| format!("failed to decode {}", path.display()))
}
fn validate_exact_directory(directory: &Path, expected: &[&str], description: &str) -> Result<()> {
    require_real_directory(directory, description)?;
    let entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read {description} {}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<BTreeSet<_>>>()
        .with_context(|| format!("failed to read entry in {}", directory.display()))?;
    let expected = BTreeSet::from_iter(expected.iter().copied().map(OsString::from));
    ensure!(entries == expected, "unexpected {description} entry set");
    Ok(())
}
fn validate_canonical_file(path: &Path, expected: &[u8], description: &str) -> Result<()> {
    ensure!(
        read_regular_file(path, description)? == expected,
        "{description} differs from its canonical input"
    );
    Ok(())
}
const STAGING_OWNER_MARKER: &str = ".boomerang-staging-owner";
const UNSAFE_CLEANUP: &str = "unsafe staging cleanup failed and residue was retained";
/// Validates the reserved name, direct canonical parent, and exact owner proof for cleanup.
fn validate_staging_ownership(
    staging: &Path,
    parent: &Path,
    marker: &Path,
    owner: &[u8],
) -> Result<bool> {
    ensure!(
        owner.starts_with(b".staging-"),
        "invalid staging owner name"
    );
    let external = marker.parent() == Some(parent);
    let exists = match fs::symlink_metadata(staging) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound && external => false,
        Err(error) => return Err(error).context("failed to inspect generated staging path"),
    };
    if exists {
        require_real_directory(staging, "generated staging directory")?;
        let canonical = validate_canonical_containment(parent, staging, "staging")?;
        ensure!(canonical.parent() == Some(parent), "staging escaped parent");
    }
    ensure!(
        read_regular_file(marker, "staging owner proof")? == owner,
        "staging owner proof changed"
    );
    Ok(exists)
}
/// Populates, validates, and atomically publishes one staging directory with safe cleanup.
fn publish_staged<T>(
    parent: &Path,
    destination: &Path,
    populate: impl FnOnce(&Path) -> Result<T>,
    validate: impl FnOnce(&Path, &T) -> Result<()>,
) -> Result<T> {
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(parent)
        .context("failed to prepare generated staging")?
        .keep();
    let owner = staging
        .file_name()
        .context("staging path has no name")?
        .as_encoded_bytes();
    let mut marker = staging.join(STAGING_OWNER_MARKER);
    fs::write(&marker, owner).context("unsafe staging cleanup unavailable; residue retained")?;
    let result = (|| {
        let value = populate(&staging)?;
        validate_staging_ownership(&staging, parent, &marker, owner)?;
        let external_marker = staging.with_extension("owner");
        rename_noreplace(&marker, &external_marker)
            .context("failed to remove staging owner marker before publication")?;
        marker = external_marker;
        validate(&staging, &value)?;
        match rename_noreplace(&staging, destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("failed to publish generated cache entry"),
        }
        Ok(value)
    })();
    let cleanup: Result<()> = (|| {
        if validate_staging_ownership(&staging, parent, &marker, owner)? {
            fs::remove_dir_all(&staging).context("failed to remove generated staging directory")?;
        }
        if marker.parent() == Some(parent) {
            fs::remove_file(&marker).context("failed to remove staging owner proof")?;
        }
        Ok(())
    })();
    cleanup.with_context(|| match &result {
        Ok(_) => UNSAFE_CLEANUP.to_owned(),
        Err(error) => format!("{error:#}; {UNSAFE_CLEANUP}"),
    })?;
    result
}
/// Creates each managed component while preserving canonical direct ancestry.
fn prepare_managed_directories(anchor: &Path, components: &[&str]) -> Result<PathBuf> {
    require_real_directory(anchor, "target anchor")?;
    let mut current = anchor.to_path_buf();
    for component in components {
        let child = current.join(component);
        match fs::create_dir(&child) {
            Ok(()) => {}
            Err(error) => ensure!(error.kind() == io::ErrorKind::AlreadyExists, "{error}"),
        }
        require_real_directory(&child, "managed path component")?;
        let canonical = validate_canonical_containment(anchor, &child, "managed path component")?;
        ensure!(
            canonical.parent() == Some(current.as_path()),
            "managed path escaped"
        );
        current = canonical;
    }
    Ok(current)
}
/// Canonicalizes a path and rejects escape from its trusted anchor.
fn validate_canonical_containment(
    anchor: &Path,
    path: &Path,
    description: &str,
) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).context("failed to canonicalize managed path")?;
    ensure!(
        canonical.starts_with(anchor),
        "{description} escapes target anchor"
    );
    Ok(canonical)
}
#[cfg(windows)]
fn windows_file_attributes_are_reparse(attributes: u32) -> bool {
    attributes & 0x400 != 0
}
#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    windows_file_attributes_are_reparse(metadata.file_attributes())
}
#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}
/// Resolves or atomically publishes a generated workspace for one exact request.
pub(crate) fn resolve_generated_workspace<T>(
    target_directory: &Path,
    request: &GeneratedWorkspaceRequest<'_>,
    reconcile: impl FnOnce(&Path) -> Result<()>,
    validate_graph: impl Fn(&Path) -> Result<T>,
) -> Result<(GeneratedWorkspace, T)> {
    let source_lock =
        read_verified_source_lock(request.source_lockfile, request.source_lock_digest)?;
    let identity = request.identity.lowercase_hex();
    fs::create_dir_all(target_directory).context("failed to prepare target anchor")?;
    require_real_directory(target_directory, "target anchor")?;
    let target_anchor =
        fs::canonicalize(target_directory).context("failed to canonicalize target anchor")?;
    let role = request.role.directory_name();
    let parent =
        prepare_managed_directories(&target_anchor, &["boomerang", "generated", "v1", role])?;
    let final_directory = parent.join(&identity);
    if final_directory
        .try_exists()
        .context("failed to inspect workspace")?
    {
        let lock_hash = hash_file(&final_directory.join("Cargo.lock"), "generated lockfile")?;
        let expectations = WorkspaceExpectations::new(request, lock_hash);
        let workspace = GeneratedWorkspace {
            directory: final_directory,
            target_anchor,
            expectations,
        };
        let graph = workspace.validate_published(&validate_graph)?;
        return Ok((workspace, graph));
    }
    let expectations = publish_staged(
        &parent,
        &final_directory,
        |directory| {
            let source_directory = directory.join("src");
            fs::create_dir(&source_directory)
                .context("failed to prepare generated source directory")?;
            fs::write(directory.join("Cargo.toml"), request.manifest)
                .context("failed to write generated Cargo.toml")?;
            fs::write(source_directory.join("main.rs"), request.source)
                .context("failed to write generated src/main.rs")?;
            fs::write(directory.join("Cargo.lock"), source_lock)
                .context("failed to seed generated Cargo.lock")?;
            reconcile(directory)?;
            validate_graph(directory)?;
            let lock_hash = hash_file(&directory.join("Cargo.lock"), "generated lockfile")?;
            let expectations = WorkspaceExpectations::new(request, lock_hash);
            let cache_record = serde_json::to_vec(&expectations.record)
                .context("failed to encode generated workspace cache record")?;
            fs::write(directory.join("cache.json"), cache_record)
                .context("failed to write generated cache.json")?;
            Ok(expectations)
        },
        validate_workspace_contents,
    )?;
    let workspace = GeneratedWorkspace {
        directory: final_directory,
        target_anchor,
        expectations,
    };
    let graph = workspace.validate_published(&validate_graph)?;
    Ok((workspace, graph))
}
/// Selects the Cargo executable while leaving invocation details to the caller.
pub(crate) fn generated_cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::{fs, path::Path, sync::Barrier};
    const LOCK: &[u8] = b"version = 4\n";
    const MANIFEST: &[u8] = b"[package]\nname = \"generated-test\"\nversion = \"0.0.0\"\n";
    const SOURCE: &[u8] = b"fn main() {}\n";
    struct CacheFixture {
        target: tempfile::TempDir,
        outside: tempfile::TempDir,
        source_lockfile: tempfile::NamedTempFile,
        source_lock_digest: [u8; 32],
    }
    impl CacheFixture {
        fn new() -> Self {
            let source_lockfile = tempfile::NamedTempFile::new().unwrap();
            fs::write(source_lockfile.path(), LOCK).unwrap();
            Self {
                target: tempfile::tempdir().unwrap(),
                outside: tempfile::tempdir().unwrap(),
                source_lockfile,
                source_lock_digest: *blake3::hash(LOCK).as_bytes(),
            }
        }
        fn request(&self) -> GeneratedWorkspaceRequest<'_> {
            GeneratedWorkspaceRequest {
                role: GeneratedRole::Descriptor,
                identity: RequestIdentity(blake3::hash(b"cache fixture")),
                manifest: MANIFEST,
                source: SOURCE,
                source_lockfile: self.source_lockfile.path(),
                source_lock_digest: &self.source_lock_digest,
            }
        }
        fn resolve(&self) -> Result<GeneratedWorkspace> {
            self.resolve_with(|_| Ok(()))
        }
        fn resolve_with(
            &self,
            reconcile: impl FnOnce(&Path) -> Result<()>,
        ) -> Result<GeneratedWorkspace> {
            resolve_generated_workspace(self.target.path(), &self.request(), reconcile, |_| Ok(()))
                .map(|(workspace, ())| workspace)
        }
        fn published() -> (Self, GeneratedWorkspace) {
            let fixture = Self::new();
            let workspace = fixture.resolve().unwrap();
            (fixture, workspace)
        }
    }
    fn expect_error<T: std::fmt::Debug>(result: Result<T>, expected: &str) -> String {
        let error = result.unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        error
    }
    #[test]
    fn workspace_validation_rejects_mutated_contents_and_entries() {
        let (fixture, workspace) = CacheFixture::published();
        fs::write(workspace.directory().join("src/main.rs"), b"changed").unwrap();
        expect_error(
            fixture.resolve(),
            "generated source differs from its canonical input",
        );

        let (fixture, workspace) = CacheFixture::published();
        fs::write(workspace.directory().join("unexpected"), b"x").unwrap();
        expect_error(fixture.resolve(), "unexpected generated workspace entry");
    }
    #[test]
    fn locked_target_rejects_coordinated_source_and_record_mutation() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        let changed = b"fn main() { panic!() }\n";
        fs::write(workspace.directory().join("src/main.rs"), changed).unwrap();
        let marker = workspace.directory().join("cache.json");
        let mut record: CacheRecord = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        record.source_hash = blake3::hash(changed).to_string();
        fs::write(marker, serde_json::to_vec(&record).unwrap()).unwrap();
        expect_error(
            workspace.with_locked_target(|_| -> Result<()> { panic!("target operation ran") }),
            "generated source differs from its canonical input",
        );
    }
    #[test]
    fn staging_failure_is_atomic_and_cleanup_is_ownership_safe() {
        let fixture = CacheFixture::new();
        expect_error(
            fixture.resolve_with(|_| anyhow::bail!("intentional reconciliation failure")),
            "intentional reconciliation failure",
        );
        let published = fixture
            .target
            .path()
            .join("boomerang/generated/v1/descriptor")
            .join(fixture.request().identity.lowercase_hex());
        assert!(!published.exists());

        let fixture = CacheFixture::new();
        let mut retained = PathBuf::new();
        expect_error(
            fixture.resolve_with(|directory| {
                fs::rename(directory, directory.with_extension("owned"))?;
                fs::create_dir(directory)?;
                fs::write(directory.join(STAGING_OWNER_MARKER), b"replacement")?;
                fs::write(directory.join("victim"), b"retain")?;
                retained = directory.to_path_buf();
                anyhow::bail!("forced reconciliation failure")
            }),
            UNSAFE_CLEANUP,
        );
        assert_eq!(fs::read(retained.join("victim")).unwrap(), b"retain");

        let fixture = CacheFixture::new();
        fs::create_dir_all(fixture.target.path().join("boomerang/.staging-crash")).unwrap();
        assert!(fixture.resolve().is_ok());
    }
    #[test]
    fn cargo_artifact_identity_fails_closed_on_partial_matches() {
        let manifest = tempfile::NamedTempFile::new().unwrap();
        let other = tempfile::NamedTempFile::new().unwrap();
        let expected = "path+file:///expected#generated@0.0.0";
        let package = PackageId {
            repr: expected.into(),
        };
        let artifact = |package: &str, manifest: &Path, binary: &str| -> Artifact {
            serde_json::from_str(&format!(
                r#"{{"package_id":{package:?},"manifest_path":{manifest:?},"target":{{"name":{binary:?},"kind":["bin"],"src_path":{manifest:?}}},"profile":{{"opt_level":"0","debug_assertions":false,"overflow_checks":false,"test":false}},"features":[],"filenames":[],"executable":null,"fresh":false}}"#,
                manifest = manifest.to_str().unwrap(),
            ))
            .unwrap()
        };
        for artifact in [
            artifact(
                "path+file:///wrong#wrong@0.0.0",
                manifest.path(),
                "generated",
            ),
            artifact(expected, other.path(), "generated"),
            artifact(expected, manifest.path(), "wrong"),
        ] {
            assert!(artifact_matches(&artifact, &package, manifest.path(), "generated").is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn artifact_and_managed_paths_reject_symlinks() {
        let fixture = CacheFixture::new();
        let artifact = fixture.target.path().join("artifact");
        let link = fixture.target.path().join("redirected");
        fs::write(&artifact, b"binary").unwrap();
        std::os::unix::fs::symlink(artifact, &link).unwrap();
        assert!(copy_private_artifact(&link, fixture.target.path()).is_err());

        let link = fixture.target.path().join("boomerang");
        std::os::unix::fs::symlink(fixture.outside.path(), link).unwrap();
        expect_error(fixture.resolve(), "is not a real directory");
    }
    #[cfg(unix)]
    #[test]
    fn source_lock_snapshot_survives_atomic_path_replacement() {
        let fixture = CacheFixture::new();
        let path = fixture.source_lockfile.path();
        let snapshot = read_verified_source_lock(path, &fixture.source_lock_digest).unwrap();
        let replacement = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
        fs::write(replacement.path(), b"replacement lock").unwrap();
        fs::rename(replacement.path(), path).unwrap();
        assert_eq!(
            (snapshot, fs::read(path).unwrap()),
            (LOCK.to_vec(), b"replacement lock".to_vec())
        );
    }
    #[cfg(windows)]
    #[test]
    fn linked_managed_parent_and_reparse_attributes_are_rejected() {
        assert!(windows_file_attributes_are_reparse(0x400));
        assert!(!windows_file_attributes_are_reparse(0));
        let fixture = CacheFixture::new();
        let generated = fixture.target.path().join("boomerang");
        match std::os::windows::fs::symlink_dir(fixture.outside.path(), generated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("failed to create directory link: {error}"),
        }
        expect_error(fixture.resolve(), "is not a real directory");
    }
    #[test]
    fn short_target_marker_rejects_collisions_and_recreates_removals() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        let marker_path = workspace.target_directory().join(".boomerang-request");
        let mut marker = read_cache_record(&marker_path, "target marker").unwrap();
        marker.request = "f".repeat(64);
        fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        expect_error(
            workspace.with_locked_target(|_| Ok(())),
            "target locator collision",
        );

        let (_fixture, workspace) = CacheFixture::published();
        let target = workspace.target_directory();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        fs::remove_dir_all(&target).unwrap();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        assert!(target.join(".boomerang-request").is_file());
    }
    #[test]
    fn concurrent_identical_publishers_reuse_one_workspace() {
        let fixture = CacheFixture::new();
        let barrier = Barrier::new(2);
        std::thread::scope(|scope| {
            let resolve = || {
                barrier.wait();
                fixture.resolve().unwrap().directory().to_path_buf()
            };
            let [first, second] = [scope.spawn(resolve), scope.spawn(resolve)];
            assert_eq!(first.join().unwrap(), second.join().unwrap());
        });
    }
}
