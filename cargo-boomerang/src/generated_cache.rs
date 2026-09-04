//! Persistent, content-addressed workspaces for generated Cargo crates.

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use fs2::FileExt;

use crate::bundle::rename_noreplace;

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
struct CacheRecord {
    schema: u32,
    role: GeneratedRole,
    request: String,
    manifest_hash: String,
    source_hash: String,
    generated_lock_hash: String,
    source_lock_hash: String,
    target_locator: String,
}

#[derive(Debug)]
struct WorkspaceExpectations {
    manifest: Box<[u8]>,
    source: Box<[u8]>,
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

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct TargetMarker {
    schema: u32,
    role: GeneratedRole,
    request: String,
}

/// Filesystem locations that form one validated generated Cargo workspace.
#[derive(Debug)]
pub(crate) struct GeneratedWorkspace {
    directory: PathBuf,
    target_anchor: PathBuf,
    expectations: WorkspaceExpectations,
}

impl GeneratedWorkspace {
    /// Revalidates a published workspace and its locked Cargo graph.
    fn validate_published(&self, validate_graph: &impl Fn(&Path) -> Result<()>) -> Result<()> {
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
        let lock_path = target.join(".boomerang-request");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .context("failed to open generated target lock")?;
        lock.lock_exclusive()
            .context("failed to lock generated target")?;
        self.validate_target_marker(&target)?;
        validate_workspace_contents(&self.directory, &self.expectations)?;
        operation(&target)
    }

    fn prepare_short_target(&self, target: &Path) -> Result<()> {
        let parent = prepare_managed_directories(&self.target_anchor, &["b"])?;
        let target_exists = target
            .try_exists()
            .context("failed to inspect generated target")?;
        if target_exists {
            return self.validate_target_marker(target);
        }

        let staging = create_staging_directory(&parent, ".staging-target-")?;
        let record = &self.expectations.record;
        let marker = TargetMarker {
            schema: GENERATED_CACHE_SCHEMA,
            role: record.role,
            request: record.request.clone(),
        };
        fs::write(
            staging.path().join(".boomerang-request"),
            serde_json::to_vec(&marker).context("failed to encode generated target marker")?,
        )
        .context("failed to write generated target marker")?;
        validate_exact_directory(staging.path(), &[".boomerang-request"], "target staging")?;
        ensure!(
            read_target_marker(&staging.path().join(".boomerang-request"))? == marker,
            "generated target marker is invalid"
        );
        match rename_noreplace(staging.path(), target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("failed to publish generated target"),
        }
        self.validate_target_marker(target)
    }

    fn validate_target_marker(&self, target: &Path) -> Result<()> {
        require_real_directory(target, "generated target directory")?;
        validate_canonical_containment(&self.target_anchor, target, "generated target directory")?;
        let marker_path = target.join(".boomerang-request");
        let marker = read_target_marker(&marker_path)?;
        let expected = &self.expectations.record;
        ensure!(
            marker.request == expected.request,
            "target locator collision"
        );
        ensure!(
            marker.schema == GENERATED_CACHE_SCHEMA && marker.role == expected.role,
            "generated target marker is invalid"
        );
        Ok(())
    }
}

fn validate_workspace_contents(
    directory: &Path,
    expectations: &WorkspaceExpectations,
) -> Result<()> {
    let entries = ["Cargo.toml", "Cargo.lock", "cache.json", "src"];
    validate_exact_directory(directory, &entries, "generated workspace")?;
    validate_exact_directory(&directory.join("src"), &["main.rs"], "generated workspace")?;
    let manifest_path = directory.join("Cargo.toml");
    let source_path = directory.join("src/main.rs");
    let lockfile_path = directory.join("Cargo.lock");
    let marker_path = directory.join("cache.json");
    validate_canonical_file(&manifest_path, &expectations.manifest, "generated manifest")?;
    validate_canonical_file(&source_path, &expectations.source, "generated source")?;
    let expected = &expectations.record;
    let lock_hash = hash_file(&lockfile_path, "generated lockfile")?;
    ensure!(
        lock_hash == expected.generated_lock_hash,
        "generated lockfile changed"
    );
    let actual: CacheRecord =
        serde_json::from_slice(&read_regular_file(&marker_path, "generated cache record")?)
            .with_context(|| format!("failed to decode {}", marker_path.display()))?;
    ensure!(actual == *expected, "generated cache record changed");
    Ok(())
}

fn hash_file(path: &Path, description: &str) -> Result<String> {
    Ok(blake3::hash(&read_regular_file(path, description)?).to_string())
}

fn read_verified_source_lock(path: &Path, expected_digest: &[u8; 32]) -> Result<Vec<u8>> {
    let bytes = read_regular_file(path, "source workspace lockfile")?;
    (blake3::hash(&bytes).as_bytes() == expected_digest)
        .then_some(bytes)
        .context("source lock digest changed")
}

fn read_regular_file(path: &Path, description: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    let real_file = metadata.file_type().is_file() && !metadata_is_reparse_point(&metadata);
    ensure!(real_file, "{description} is not a real regular file");
    fs::read(path).with_context(|| format!("failed to read {description} {}", path.display()))
}

fn require_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    let real_directory = metadata.file_type().is_dir() && !metadata_is_reparse_point(&metadata);
    ensure!(real_directory, "{description} is not a real directory");
    Ok(())
}

fn read_target_marker(path: &Path) -> Result<TargetMarker> {
    let bytes = read_regular_file(path, "generated target marker")?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to decode {}", path.display()))
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
    let actual = read_regular_file(path, description)?;
    ensure!(
        actual == expected,
        "{description} differs from its canonical input"
    );
    Ok(())
}

fn create_staging_directory(parent: &Path, prefix: &str) -> Result<tempfile::TempDir> {
    let staging = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .with_context(|| format!("failed to prepare staging in {}", parent.display()))?;
    validate_canonical_containment(parent, staging.path(), "generated staging directory")?;
    Ok(staging)
}

fn prepare_managed_directories(anchor: &Path, components: &[&str]) -> Result<PathBuf> {
    require_real_directory(anchor, "target anchor")?;
    let mut current = anchor.to_path_buf();
    for component in components {
        let child = current.join(component);
        match fs::create_dir(&child) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("failed to prepare managed directory"),
        }
        require_real_directory(&child, "managed path component")?;
        let canonical = validate_canonical_containment(anchor, &child, "managed path component")?;
        ensure!(
            canonical.parent() == Some(current.as_path()),
            "managed path escaped parent"
        );
        current = canonical;
    }
    Ok(current)
}

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
pub(crate) fn resolve_generated_workspace(
    target_directory: &Path,
    request: &GeneratedWorkspaceRequest<'_>,
    reconcile: impl FnOnce(&Path) -> Result<()>,
    validate_graph: impl Fn(&Path) -> Result<()>,
) -> Result<GeneratedWorkspace> {
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

    match fs::symlink_metadata(&final_directory) {
        Ok(_) => {
            let expectations = WorkspaceExpectations::new(
                request,
                hash_file(&final_directory.join("Cargo.lock"), "generated lockfile")?,
            );
            let workspace = GeneratedWorkspace {
                directory: final_directory,
                target_anchor,
                expectations,
            };
            workspace.validate_published(&validate_graph)?;
            return Ok(workspace);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("failed to inspect generated workspace"),
    }

    let staging = create_staging_directory(&parent, ".staging-")?;
    let directory = staging.path();
    let source_directory = directory.join("src");
    fs::create_dir(&source_directory).context("failed to prepare generated source directory")?;
    fs::write(directory.join("Cargo.toml"), request.manifest)
        .context("failed to write generated Cargo.toml")?;
    fs::write(source_directory.join("main.rs"), request.source)
        .context("failed to write generated src/main.rs")?;
    fs::write(directory.join("Cargo.lock"), source_lock)
        .context("failed to seed generated Cargo.lock")?;
    reconcile(directory)?;
    validate_graph(directory)?;
    let expectations = WorkspaceExpectations::new(
        request,
        hash_file(&directory.join("Cargo.lock"), "generated lockfile")?,
    );
    let cache_record = serde_json::to_vec(&expectations.record)
        .context("failed to encode generated workspace cache record")?;
    fs::write(directory.join("cache.json"), cache_record)
        .context("failed to write generated cache.json")?;
    validate_workspace_contents(directory, &expectations)?;
    let workspace = GeneratedWorkspace {
        directory: final_directory.clone(),
        target_anchor,
        expectations,
    };

    match rename_noreplace(directory, &final_directory) {
        Ok(()) => Ok(workspace),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            workspace.validate_published(&validate_graph)?;
            Ok(workspace)
        }
        Err(error) => Err(error).context("failed to publish generated workspace"),
    }
}

/// Selects the Cargo executable while leaving invocation details to the caller.
pub(crate) fn generated_cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, sync::Barrier};

    use anyhow::Result;

    use super::*;

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
            let mut identity = RequestIdentityBuilder::new(GeneratedRole::Descriptor);
            identity.field("manifest", Some(MANIFEST));
            identity.field("source", Some(SOURCE));
            identity.field("source-lock-digest", Some(&self.source_lock_digest));
            GeneratedWorkspaceRequest {
                role: GeneratedRole::Descriptor,
                identity: identity.finish(),
                manifest: MANIFEST,
                source: SOURCE,
                source_lockfile: self.source_lockfile.path(),
                source_lock_digest: &self.source_lock_digest,
            }
        }

        fn resolve(&self) -> Result<GeneratedWorkspace> {
            self.resolve_with_reconcile(|_| Ok(()))
        }

        fn resolve_with_reconcile(
            &self,
            reconcile: impl FnOnce(&Path) -> Result<()>,
        ) -> Result<GeneratedWorkspace> {
            resolve_generated_workspace(self.target.path(), &self.request(), reconcile, |_| Ok(()))
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
    fn changed_cached_source_fails_closed() {
        let (fixture, workspace) = CacheFixture::published();
        fs::write(workspace.directory().join("src/main.rs"), b"changed").unwrap();
        let error = expect_error(fixture.resolve(), "generated source");
        assert!(error.contains("canonical input"), "{error}");
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
        let closure_ran = std::cell::Cell::new(false);
        let error = expect_error(
            workspace.with_locked_target(|_| {
                closure_ran.set(true);
                Ok(())
            }),
            "generated source",
        );
        assert!(error.contains("canonical input"), "{error}");
        assert!(!closure_ran.get());
    }

    #[test]
    fn failed_reconciliation_never_publishes_request_directory() {
        let fixture = CacheFixture::new();
        expect_error(
            fixture.resolve_with_reconcile(|_| anyhow::bail!("intentional reconciliation failure")),
            "intentional reconciliation failure",
        );
        let published = fixture.target.path().join(format!(
            "boomerang/generated/v1/descriptor/{}",
            fixture.request().identity.lowercase_hex()
        ));
        assert!(!published.exists());
    }

    #[test]
    fn unexpected_workspace_entry_fails_closed() {
        let (fixture, workspace) = CacheFixture::published();
        fs::write(workspace.directory().join("unexpected"), b"x").unwrap();
        expect_error(fixture.resolve(), "unexpected generated workspace entry");
    }

    #[test]
    fn request_identity_changes_with_canonical_field_bytes() {
        let mut first = RequestIdentityBuilder::new(GeneratedRole::Descriptor);
        first.field("cargo-config", Some(b"first"));
        let mut second = RequestIdentityBuilder::new(GeneratedRole::Descriptor);
        second.field("cargo-config", Some(b"second"));
        assert_ne!(first.finish(), second.finish());
    }

    #[cfg(unix)]
    #[test]
    fn source_lock_snapshot_survives_atomic_path_replacement() {
        let fixture = CacheFixture::new();
        let path = fixture.source_lockfile.path();
        let snapshot = read_verified_source_lock(path, &fixture.source_lock_digest).unwrap();
        let parent = path.parent().unwrap();
        let replacement = tempfile::NamedTempFile::new_in(parent).unwrap();
        fs::write(replacement.path(), b"replacement lock").unwrap();
        fs::rename(replacement.path(), path).unwrap();
        assert_eq!(snapshot, LOCK);
        assert_eq!(fs::read(path).unwrap(), b"replacement lock");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_managed_parent_is_rejected() {
        let fixture = CacheFixture::new();
        let link = fixture.target.path().join("boomerang");
        std::os::unix::fs::symlink(fixture.outside.path(), link).unwrap();
        expect_error(fixture.resolve(), "is not a real directory");
    }

    #[cfg(windows)]
    #[test]
    fn linked_managed_parent_is_rejected() {
        let fixture = CacheFixture::new();
        let generated = fixture.target.path().join("boomerang");
        if let Err(error) = std::os::windows::fs::symlink_dir(fixture.outside.path(), generated) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("failed to create directory link: {error}");
        }
        expect_error(fixture.resolve(), "is not a real directory");
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_attribute_is_detected() {
        assert!(windows_file_attributes_are_reparse(0x400));
        assert!(!windows_file_attributes_are_reparse(0));
    }

    #[test]
    fn short_target_collision_fails_closed() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        let marker_path = workspace.target_directory().join(".boomerang-request");
        let mut marker = read_target_marker(&marker_path).unwrap();
        marker.request = "f".repeat(64);
        fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        expect_error(
            workspace.with_locked_target(|_| Ok(())),
            "target locator collision",
        );
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

    #[test]
    fn removed_short_target_is_recreated_with_its_full_marker() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        fs::remove_dir_all(workspace.target_directory()).unwrap();
        workspace
            .with_locked_target(|target| {
                assert!(target.join(".boomerang-request").is_file());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn crash_staging_residue_is_ignored() {
        let fixture = CacheFixture::new();
        fs::create_dir_all(fixture.target.path().join("boomerang/.staging-crash")).unwrap();
        assert!(fixture.resolve().is_ok());
    }
}
