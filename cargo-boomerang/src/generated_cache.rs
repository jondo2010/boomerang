//! Persistent, content-addressed workspaces for generated Cargo crates.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, ensure, Context, Result};
use fs2::FileExt;

use crate::bundle::rename_noreplace;

/// Schema version encoded into every generated-workspace identity and marker.
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
    /// Returns the stable cache-directory component assigned to this role.
    fn directory_name(self) -> &'static str {
        match self {
            Self::Descriptor => "descriptor",
            Self::Launcher => "launcher",
        }
    }

    /// Returns the stable one-character target-directory prefix assigned to this role.
    fn target_prefix(self) -> char {
        match self {
            Self::Descriptor => 'd',
            Self::Launcher => 'l',
        }
    }
}

/// Content identity of one generated-workspace request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestIdentity(
    /// BLAKE3 digest of the canonically encoded request fields.
    blake3::Hash,
);

impl RequestIdentity {
    /// Returns the complete lowercase hexadecimal representation of this request digest.
    fn lowercase_hex(self) -> String {
        self.0.to_hex().to_string()
    }

    /// Returns this role's short, collision-resistant Cargo target-directory component.
    fn short_target_name(self, role: GeneratedRole) -> String {
        format!("{}{}", role.target_prefix(), &self.lowercase_hex()[..32])
    }
}

/// Canonically encodes the inputs to a generated-workspace request.
pub(crate) struct RequestIdentityBuilder {
    /// Incremental BLAKE3 state over the schema, role, and caller-provided fields.
    hasher: blake3::Hasher,
}

impl RequestIdentityBuilder {
    /// Starts an identity with the cache schema and generated-workspace role.
    pub(crate) fn new(role: GeneratedRole) -> Self {
        let mut builder = Self {
            hasher: blake3::Hasher::new(),
        };
        builder.field("schema", Some(&GENERATED_CACHE_SCHEMA.to_be_bytes()));
        builder.field("role", Some(role.directory_name().as_bytes()));
        builder
    }

    /// Appends one length-delimited optional request field using the canonical cache encoding.
    pub(crate) fn field(&mut self, label: &str, value: Option<&[u8]>) {
        let label_length = u64::try_from(label.len())
            .expect("request identity labels must fit the canonical u64 representation");
        self.hasher.update(&label_length.to_be_bytes());
        self.hasher.update(label.as_bytes());
        match value {
            Some(value) => {
                self.hasher.update(&[1]);
                let value_length = u64::try_from(value.len())
                    .expect("request identity values must fit the canonical u64 representation");
                self.hasher.update(&value_length.to_be_bytes());
                self.hasher.update(value);
            }
            None => {
                self.hasher.update(&[0]);
            }
        }
    }

    /// Finishes canonical encoding and returns the resulting content identity.
    pub(crate) fn finish(self) -> RequestIdentity {
        RequestIdentity(self.hasher.finalize())
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

/// Serialized integrity record proving that a generated workspace was fully published.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct CacheRecord {
    /// Cache protocol version used to encode the record and request identity.
    schema: u32,
    /// Role namespace that owns the immutable workspace.
    role: GeneratedRole,
    /// Full lowercase request digest used as the workspace directory name.
    request: String,
    /// BLAKE3 digest of the exact generated manifest bytes.
    manifest_hash: String,
    /// BLAKE3 digest of the exact generated Rust source bytes.
    source_hash: String,
    /// BLAKE3 digest of the reconciled generated lockfile.
    generated_lock_hash: String,
    /// BLAKE3 digest of the source workspace lockfile.
    source_lock_hash: String,
    /// Short role-prefixed Cargo target directory name.
    target_locator: String,
}

/// Independently retained canonical bytes and record used for every later cache validation.
#[derive(Debug)]
struct WorkspaceExpectations {
    /// Original generated manifest bytes.
    manifest: Box<[u8]>,
    /// Original generated source bytes.
    source: Box<[u8]>,
    /// Exact immutable cache record established at first validation.
    record: CacheRecord,
}

impl WorkspaceExpectations {
    /// Captures canonical request bytes and all record fields around one generated-lock hash.
    fn new(request: &GeneratedWorkspaceRequest<'_>, generated_lock_hash: String) -> Self {
        Self {
            manifest: request.manifest.into(),
            source: request.source.into(),
            record: CacheRecord {
                schema: GENERATED_CACHE_SCHEMA,
                role: request.role,
                request: request.identity.lowercase_hex(),
                manifest_hash: hash_bytes(request.manifest),
                source_hash: hash_bytes(request.source),
                generated_lock_hash,
                source_lock_hash: digest_hex(request.source_lock_digest),
                target_locator: request.identity.short_target_name(request.role),
            },
        }
    }
}

/// Serialized ownership record stored in every short Cargo target directory.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct TargetMarker {
    /// Cache protocol version used to interpret the marker.
    schema: u32,
    /// Generated-workspace role permitted to use the target directory.
    role: GeneratedRole,
    /// Full lowercase request digest owning the shortened target locator.
    request: String,
}

/// Filesystem locations that form one validated generated Cargo workspace.
#[derive(Clone, Debug)]
pub(crate) struct GeneratedWorkspace {
    /// Published cache directory containing manifest, source, lockfile, and completion marker.
    directory: PathBuf,
    /// Generated Cargo manifest within the published cache directory.
    manifest_path: PathBuf,
    /// Generated Rust entry point within the published cache directory.
    source_path: PathBuf,
    /// Reconciled generated lockfile within the published cache directory.
    lockfile_path: PathBuf,
    /// Shared, short Cargo target directory for this role and request identity.
    target_directory: PathBuf,
    /// Integrity record validated before cache reuse and target-directory use.
    marker_path: PathBuf,
    /// Canonical caller-selected target anchor containing every managed path.
    target_anchor: PathBuf,
    /// Canonical expectations shared by staged and published views.
    expectations: Arc<WorkspaceExpectations>,
}

impl GeneratedWorkspace {
    /// Derives all stable workspace paths from one cache directory and target directory.
    fn for_directory(
        directory: PathBuf,
        target_directory: PathBuf,
        target_anchor: PathBuf,
        expectations: Arc<WorkspaceExpectations>,
    ) -> Self {
        Self {
            manifest_path: directory.join("Cargo.toml"),
            source_path: directory.join("src/main.rs"),
            lockfile_path: directory.join("Cargo.lock"),
            marker_path: directory.join("cache.json"),
            directory,
            target_directory,
            target_anchor,
            expectations,
        }
    }

    /// Validates every managed file and record field against independently retained expectations.
    fn validate_retained_expectations(&self) -> Result<()> {
        let actual = self.validate_retained_contents()?;
        let role = self
            .directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("generated workspace has no role directory"))?;
        let identity = self
            .directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("generated workspace has no identity directory"))?;
        ensure!(
            actual.role.directory_name() == role && actual.request == identity,
            "generated workspace cache record is invalid: {}",
            self.marker_path.display()
        );
        Ok(())
    }

    /// Validates staged or published content against independently retained expectations.
    fn validate_retained_contents(&self) -> Result<CacheRecord> {
        validate_workspace_tree(self)?;
        validate_canonical_file(
            &self.manifest_path,
            &self.expectations.manifest,
            "generated manifest",
        )?;
        validate_canonical_file(
            &self.source_path,
            &self.expectations.source,
            "generated source",
        )?;
        let expected = &self.expectations.record;
        ensure!(
            hash_file(&self.lockfile_path, "generated lockfile")? == expected.generated_lock_hash,
            "generated workspace lockfile differs from retained canonical expectations: {}",
            self.lockfile_path.display()
        );
        let bytes = read_regular_file(&self.marker_path, "generated cache record")?;
        let actual: CacheRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", self.marker_path.display()))?;
        ensure!(
            actual == *expected,
            "generated workspace cache record differs from retained canonical expectations: {}",
            self.marker_path.display()
        );
        Ok(actual)
    }

    /// Returns the published cache directory.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the generated Cargo manifest path.
    pub(crate) fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Returns the generated Rust entry-point path.
    #[cfg(test)]
    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Returns the short Cargo target directory shared by this exact request.
    pub(crate) fn target_directory(&self) -> &Path {
        &self.target_directory
    }

    /// Serializes target-directory use and revalidates the published cache before an operation.
    pub(crate) fn with_locked_target<T>(
        &self,
        operation: impl FnOnce(&Path) -> Result<T>,
    ) -> Result<T> {
        self.validate_retained_expectations()?;
        self.prepare_short_target()?;
        let lock_path = self.target_directory().join(".boomerang-request");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {}", lock_path.display()))?;
        lock_file
            .lock_exclusive()
            .with_context(|| format!("failed to lock {}", lock_path.display()))?;
        self.validate_target_marker()?;
        self.validate_retained_expectations()?;
        let result = operation(self.target_directory());
        FileExt::unlock(&lock_file)
            .with_context(|| format!("failed to unlock {}", lock_path.display()))?;
        result
    }

    /// Creates or validates the short Cargo target using failure-atomic publication.
    fn prepare_short_target(&self) -> Result<()> {
        let target_parent = prepare_managed_directories(&self.target_anchor, &["b"])?;
        ensure!(
            target_parent.join(
                self.target_directory
                    .file_name()
                    .expect("short target always has a locator")
            ) == self.target_directory,
            "generated target locator escaped its managed parent: {}",
            self.target_directory.display()
        );
        match fs::symlink_metadata(&self.target_directory) {
            Ok(_) => return self.validate_target_marker(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect generated target directory {}",
                        self.target_directory.display()
                    )
                });
            }
        }

        let mut staging = OwnedStagingDirectory::create(&target_parent, ".staging-target-")?;
        let marker_path = staging.path().join(".boomerang-request");
        let marker = self.expected_target_marker();
        let marker_bytes =
            serde_json::to_vec(&marker).context("failed to encode generated target marker")?;
        let prepared = fs::write(&marker_path, &marker_bytes)
            .context("failed to write generated target marker")
            .and_then(|()| staging.prepare_for_publication())
            .and_then(|()| validate_target_directory(staging.path(), &marker));
        if let Err(error) = prepared {
            return staging.finish_result(Err(error));
        }
        match rename_noreplace(staging.path(), &self.target_directory) {
            Ok(()) => self.validate_target_marker(),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let winner = self.validate_target_marker();
                let result = winner.and_then(|()| {
                    let published = fs::read(self.target_directory.join(".boomerang-request"))
                        .context("failed to read concurrent target marker")?;
                    ensure!(
                        marker_bytes == published,
                        "concurrent generated target winner differs from candidate {}",
                        self.target_directory.display()
                    );
                    Ok(())
                });
                staging.finish_result(result)
            }
            Err(error) => {
                let error = anyhow!(error).context(format!(
                    "failed to publish generated target directory {}",
                    self.target_directory.display()
                ));
                staging.finish_result(Err(error))
            }
        }
    }

    /// Builds the exact marker expected in this workspace's short target.
    fn expected_target_marker(&self) -> TargetMarker {
        TargetMarker {
            schema: GENERATED_CACHE_SCHEMA,
            role: self.expectations.record.role,
            request: self.expectations.record.request.clone(),
        }
    }

    /// Validates the short target marker and diagnoses full-digest prefix collisions.
    fn validate_target_marker(&self) -> Result<()> {
        require_real_directory(&self.target_directory, "generated target directory")?;
        validate_canonical_containment(
            &self.target_anchor,
            &self.target_directory,
            "generated target directory",
        )?;
        let marker_path = self.target_directory.join(".boomerang-request");
        let bytes = read_regular_file(&marker_path, "generated target marker")?;
        let marker: TargetMarker = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", marker_path.display()))?;
        ensure!(
            marker.request == self.expectations.record.request,
            "target locator collision at {}: expected request {}, found {}",
            self.target_directory.display(),
            self.expectations.record.request,
            marker.request
        );
        ensure!(
            marker.schema == GENERATED_CACHE_SCHEMA && marker.role == self.expectations.record.role,
            "generated target marker is invalid: {}",
            marker_path.display()
        );
        Ok(())
    }
}

/// Encodes a BLAKE3 digest as lowercase hexadecimal text.
fn digest_hex(digest: &[u8; 32]) -> String {
    blake3::Hash::from(*digest).to_hex().to_string()
}

/// Hashes an in-memory canonical cache input as lowercase BLAKE3 text.
fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Reads and hashes one required regular generated-workspace file.
fn hash_file(path: &Path, description: &str) -> Result<String> {
    Ok(hash_bytes(&read_regular_file(path, description)?))
}

/// Reads one source lockfile handle, validates its kind and digest, and returns that exact snapshot.
fn read_verified_source_lock(path: &Path, expected_digest: &[u8; 32]) -> Result<Vec<u8>> {
    use std::io::Read as _;

    let mut file = File::open(path).context("failed to open source workspace lockfile")?;
    let metadata = file
        .metadata()
        .context("failed to inspect source workspace lockfile")?;
    ensure!(
        metadata.file_type().is_file() && !metadata_is_reparse_point(&metadata),
        "source workspace lockfile {} is not a real regular file",
        path.display()
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .context("failed to read source workspace lockfile")?;
    ensure!(
        hash_bytes(&bytes) == digest_hex(expected_digest),
        "source workspace lockfile {} differs from its canonical digest",
        path.display()
    );
    Ok(bytes)
}

/// Reads a real regular non-reparse file without accepting redirected entries.
fn read_regular_file(path: &Path, description: &str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && !metadata_is_reparse_point(&metadata),
        "{description} {} is not a real regular file",
        path.display()
    );
    fs::read(path).with_context(|| format!("failed to read {description} {}", path.display()))
}

/// Requires a path to name a real directory rather than a redirected entry.
fn require_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    ensure!(
        metadata.file_type().is_dir() && !metadata_is_reparse_point(&metadata),
        "{description} {} is not a real directory",
        path.display()
    );
    Ok(())
}

/// Requires an unpublished short target to contain only its exact regular marker file.
fn validate_target_directory(directory: &Path, expected: &TargetMarker) -> Result<()> {
    validate_exact_directory(
        directory,
        &[".boomerang-request"],
        "generated target staging",
    )?;
    let marker_path = directory.join(".boomerang-request");
    let bytes = read_regular_file(&marker_path, "generated target marker")?;
    let marker: TargetMarker = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to decode {}", marker_path.display()))?;
    ensure!(
        marker.schema == expected.schema
            && marker.role == expected.role
            && marker.request == expected.request,
        "generated target marker is invalid: {}",
        marker_path.display()
    );
    Ok(())
}

/// Requires a real directory to contain exactly the named entries.
fn validate_exact_directory(directory: &Path, expected: &[&str], description: &str) -> Result<()> {
    require_real_directory(directory, description)?;
    let mut actual = std::collections::BTreeSet::new();
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read {description} {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", directory.display()))?;
        let name = entry.file_name();
        let name_text = name
            .to_str()
            .ok_or_else(|| anyhow!("{description} entry is not valid UTF-8"))?;
        ensure!(
            expected.contains(&name_text),
            "unexpected {description} entry {}",
            entry.path().display()
        );
        actual.insert(name);
    }
    for name in expected {
        ensure!(
            actual.contains(std::ffi::OsStr::new(name)),
            "missing {description} entry {}",
            directory.join(name).display()
        );
    }
    Ok(())
}

/// Requires cached bytes to equal the freshly rendered canonical input bytes.
fn validate_canonical_file(path: &Path, expected: &[u8], description: &str) -> Result<()> {
    let actual = read_regular_file(path, description)?;
    ensure!(
        actual == expected,
        "{description} {} differs from its canonical input",
        path.display()
    );
    Ok(())
}

/// Requires exactly the fixed generated Cargo workspace entries and entry kinds.
fn validate_workspace_tree(workspace: &GeneratedWorkspace) -> Result<()> {
    validate_exact_directory(
        workspace.directory(),
        &["Cargo.toml", "Cargo.lock", "cache.json", "src"],
        "generated workspace",
    )?;
    let source_directory = workspace
        .source_path
        .parent()
        .expect("generated source path always has a parent");
    validate_exact_directory(source_directory, &["main.rs"], "generated workspace")?;
    Ok(())
}

/// Owns one reserved staging directory until publication or validated cleanup.
struct OwnedStagingDirectory {
    /// Reserved staging path created by this invocation.
    path: PathBuf,
    /// Canonical managed parent beside the final publication path.
    parent: PathBuf,
    /// Unique directory name recorded in the invocation-owned marker.
    owner: String,
}

impl OwnedStagingDirectory {
    /// Creates a reserved staging directory and writes its invocation-owned marker.
    fn create(parent: &Path, prefix: &'static str) -> Result<Self> {
        require_real_directory(parent, "generated staging parent")?;
        let parent = fs::canonicalize(parent)
            .with_context(|| format!("failed to canonicalize {}", parent.display()))?;
        let temporary = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(&parent)
            .with_context(|| format!("failed to prepare staging in {}", parent.display()))?;
        let path = temporary.keep();
        let owner = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("unsafe staging cleanup unavailable: name is not valid UTF-8"))?
            .to_owned();
        let staging = Self {
            path,
            parent,
            owner,
        };
        staging.write_owner_marker().map_err(|error| {
            error.context("unsafe staging cleanup unavailable and residue was retained")
        })?;
        Ok(staging)
    }

    /// Returns the invocation-owned staging path.
    fn path(&self) -> &Path {
        &self.path
    }

    /// Removes the owner marker immediately before exact-tree validation and publication.
    fn prepare_for_publication(&self) -> Result<()> {
        self.validate_ownership()?;
        fs::remove_file(self.owner_marker_path()).context("failed to remove staging owner marker")
    }

    /// Cleans a staging directory after preserving a successful or failed operation result.
    fn finish_result<T>(&mut self, result: Result<T>) -> Result<T> {
        let cleanup = (|| -> Result<()> {
            self.validate_path_and_name()?;
            if !self.owner_marker_path().try_exists()? {
                self.write_owner_marker()?;
            }
            self.validate_ownership()?;
            fs::remove_dir_all(&self.path).context("failed to remove generated staging directory")
        })();
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => {
                Err(cleanup.context("unsafe staging cleanup failed and residue was retained"))
            }
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "unsafe staging cleanup failed and residue was retained: {cleanup:#}"
            ))),
        }
    }

    /// Returns the reserved invocation-owner marker path.
    fn owner_marker_path(&self) -> PathBuf {
        self.path.join(".boomerang-staging-owner")
    }

    /// Writes the invocation-owner marker without replacing an existing entry.
    fn write_owner_marker(&self) -> Result<()> {
        let marker_path = self.owner_marker_path();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker_path)
            .with_context(|| format!("failed to create {}", marker_path.display()))?;
        use std::io::Write as _;
        marker
            .write_all(self.owner.as_bytes())
            .with_context(|| format!("failed to write {}", marker_path.display()))
    }

    /// Validates the canonical parent, reserved prefix, and exact owner-marker contents.
    fn validate_ownership(&self) -> Result<()> {
        self.validate_path_and_name()?;
        let marker_path = self.owner_marker_path();
        let bytes = read_regular_file(&marker_path, "generated staging owner marker")?;
        ensure!(
            bytes == self.owner.as_bytes(),
            "unsafe staging cleanup refused for owner mismatch at {}",
            self.path.display()
        );
        Ok(())
    }

    /// Validates the reserved staging name and its canonical direct parent.
    fn validate_path_and_name(&self) -> Result<()> {
        require_real_directory(&self.path, "generated staging directory")?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("generated staging directory name is not valid UTF-8"))?;
        ensure!(
            name.starts_with(".staging-") && name == self.owner,
            "unsafe staging cleanup refused for unowned path {}",
            self.path.display()
        );
        let canonical = fs::canonicalize(&self.path)
            .with_context(|| format!("failed to canonicalize {}", self.path.display()))?;
        ensure!(
            canonical.parent() == Some(self.parent.as_path()),
            "unsafe staging cleanup refused for escaped path {}",
            self.path.display()
        );
        Ok(())
    }
}

/// Creates or validates the caller-selected target anchor without following its final component.
fn prepare_target_anchor(target: &Path) -> Result<PathBuf> {
    if let Err(error) = fs::symlink_metadata(target) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error).context("failed to inspect target anchor");
        }
        fs::create_dir_all(target).context("failed to prepare target anchor")?;
    }
    require_real_directory(target, "target anchor")?;
    fs::canonicalize(target)
        .with_context(|| format!("failed to canonicalize target anchor {}", target.display()))
}

/// Creates and validates fixed tool-managed directory components one level at a time.
fn prepare_managed_directories(anchor: &Path, components: &[&str]) -> Result<PathBuf> {
    require_real_directory(anchor, "target anchor")?;
    let canonical_anchor = fs::canonicalize(anchor)
        .with_context(|| format!("failed to canonicalize target anchor {}", anchor.display()))?;
    let mut current = canonical_anchor.clone();
    for component in components {
        let child = current.join(component);
        if let Err(error) = fs::symlink_metadata(&child) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error).context("failed to inspect managed directory");
            }
            if let Err(error) = fs::create_dir(&child) {
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error).context("failed to prepare managed directory");
                }
            }
        }
        require_real_directory(&child, "managed path component")?;
        let canonical =
            validate_canonical_containment(&canonical_anchor, &child, "managed path component")?;
        ensure!(
            canonical.parent() == Some(current.as_path()),
            "managed path component escaped its parent: {}",
            child.display()
        );
        current = canonical;
    }
    Ok(current)
}

/// Canonicalizes a managed path and requires it to remain beneath the target anchor.
fn validate_canonical_containment(
    anchor: &Path,
    path: &Path,
    description: &str,
) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize {description} {}", path.display()))?;
    ensure!(
        canonical.starts_with(anchor),
        "{description} {} escapes target anchor {}",
        path.display(),
        anchor.display()
    );
    Ok(canonical)
}

/// Returns whether Windows filesystem attributes identify a reparse point.
#[cfg(windows)]
fn windows_file_attributes_are_reparse(attributes: u32) -> bool {
    attributes & 0x400 != 0
}

/// Returns whether metadata identifies a Windows reparse point.
#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    windows_file_attributes_are_reparse(metadata.file_attributes())
}

/// Reports no Windows reparse attribute on non-Windows hosts.
#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

/// Requires concurrent generated-workspace candidates to contain identical immutable bytes.
fn validate_identical_workspaces(
    candidate: &GeneratedWorkspace,
    winner: &GeneratedWorkspace,
) -> Result<()> {
    for relative in ["Cargo.toml", "Cargo.lock", "src/main.rs", "cache.json"] {
        let candidate_bytes =
            fs::read(candidate.directory().join(relative)).context("failed to read candidate")?;
        let winner_path = winner.directory().join(relative);
        ensure!(
            candidate_bytes == fs::read(&winner_path).context("failed to read winner")?,
            "concurrent generated workspace winner differs at {}",
            winner_path.display()
        );
    }
    Ok(())
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
    let target_anchor = prepare_target_anchor(target_directory)?;
    let parent = prepare_managed_directories(
        &target_anchor,
        &[
            "boomerang",
            "generated",
            "v1",
            request.role.directory_name(),
        ],
    )?;
    let target_parent = prepare_managed_directories(&target_anchor, &["b"])?;
    let final_directory = parent.join(&identity);
    let target = target_parent.join(request.identity.short_target_name(request.role));
    let workspace_at = |directory, expectations| {
        GeneratedWorkspace::for_directory(
            directory,
            target.clone(),
            target_anchor.clone(),
            expectations,
        )
    };

    match fs::symlink_metadata(&final_directory) {
        Ok(_) => {
            require_real_directory(&final_directory, "generated workspace")?;
            validate_canonical_containment(
                &target_anchor,
                &final_directory,
                "generated workspace",
            )?;
            let expectations = Arc::new(WorkspaceExpectations::new(
                request,
                hash_file(&final_directory.join("Cargo.lock"), "generated lockfile")?,
            ));
            let workspace = workspace_at(final_directory, expectations);
            workspace.validate_retained_expectations()?;
            validate_graph(workspace.directory())?;
            return Ok(workspace);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect generated workspace {}",
                    final_directory.display()
                )
            });
        }
    }

    let mut staging = OwnedStagingDirectory::create(&parent, ".staging-")?;
    let candidate = (|| -> Result<(GeneratedWorkspace, GeneratedWorkspace)> {
        let directory = staging.path();
        let source_directory = directory.join("src");
        fs::create_dir(&source_directory)
            .with_context(|| format!("failed to prepare {}", source_directory.display()))?;
        fs::write(directory.join("Cargo.toml"), request.manifest)
            .context("failed to write generated Cargo.toml")?;
        fs::write(source_directory.join("main.rs"), request.source)
            .context("failed to write generated src/main.rs")?;
        fs::write(directory.join("Cargo.lock"), &source_lock)
            .context("failed to seed generated Cargo.lock")?;
        reconcile(directory)?;
        validate_graph(directory)?;
        let expectations = Arc::new(WorkspaceExpectations::new(
            request,
            hash_file(&directory.join("Cargo.lock"), "generated lockfile")?,
        ));
        fs::write(
            directory.join("cache.json"),
            serde_json::to_vec(&expectations.record)
                .context("failed to encode generated workspace cache record")?,
        )
        .context("failed to write generated cache.json")?;
        staging.prepare_for_publication()?;
        let staged = workspace_at(directory.to_path_buf(), expectations.clone());
        let workspace = workspace_at(final_directory.clone(), expectations);
        staged.validate_retained_contents()?;
        Ok((staged, workspace))
    })();
    let (staged, workspace) = match candidate {
        Ok(candidate) => candidate,
        Err(error) => return staging.finish_result(Err(error)),
    };

    match rename_noreplace(staged.directory(), &final_directory) {
        Ok(()) => Ok(workspace),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let winner = (|| -> Result<GeneratedWorkspace> {
                require_real_directory(&final_directory, "generated workspace")?;
                validate_canonical_containment(
                    &workspace.target_anchor,
                    &final_directory,
                    "generated workspace",
                )?;
                workspace.validate_retained_expectations()?;
                validate_graph(workspace.directory())?;
                validate_identical_workspaces(&staged, &workspace)?;
                Ok(workspace)
            })();
            staging.finish_result(winner)
        }
        Err(error) => {
            let error = anyhow!(error).context(format!(
                "failed to publish generated workspace {}",
                final_directory.display()
            ));
            staging.finish_result(Err(error))
        }
    }
}

/// Selects the Cargo executable while leaving invocation details to the caller.
pub(crate) fn generated_cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, path::Path, sync::Arc};

    use anyhow::Result;

    use super::{
        resolve_generated_workspace, GeneratedRole, GeneratedWorkspace, GeneratedWorkspaceRequest,
        RequestIdentityBuilder,
    };

    /// Isolated generated-cache inputs and filesystem roots for unit tests.
    struct CacheFixture {
        /// Target anchor containing all tool-managed cache state.
        target: tempfile::TempDir,
        /// Directory outside the target anchor used by redirection tests.
        outside: tempfile::TempDir,
        /// Source-workspace lockfile copied into cache candidates.
        source_lockfile: tempfile::NamedTempFile,
        /// Digest of the fixed source-workspace lockfile bytes.
        source_lock_digest: [u8; 32],
        /// Canonical generated manifest bytes for this request.
        manifest: Vec<u8>,
        /// Canonical generated Rust source bytes for this request.
        source: Vec<u8>,
    }

    impl CacheFixture {
        /// Creates fixed request inputs and only the caller-selected target anchor.
        fn new() -> Self {
            let target = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let mut source_lockfile = tempfile::NamedTempFile::new().unwrap();
            let source_lock = b"version = 4\n";
            source_lockfile.write_all(source_lock).unwrap();
            source_lockfile.flush().unwrap();
            Self {
                target,
                outside,
                source_lockfile,
                source_lock_digest: *blake3::hash(source_lock).as_bytes(),
                manifest: b"[package]\nname = \"generated-test\"\nversion = \"0.0.0\"\n".to_vec(),
                source: b"fn main() {}\n".to_vec(),
            }
        }

        /// Returns the complete descriptor request derived from the fixture's canonical bytes.
        fn request(&self) -> GeneratedWorkspaceRequest<'_> {
            let mut identity = RequestIdentityBuilder::new(GeneratedRole::Descriptor);
            identity.field("manifest", Some(&self.manifest));
            identity.field("source", Some(&self.source));
            identity.field("source-lock-digest", Some(&self.source_lock_digest));
            GeneratedWorkspaceRequest {
                role: GeneratedRole::Descriptor,
                identity: identity.finish(),
                manifest: &self.manifest,
                source: &self.source,
                source_lockfile: self.source_lockfile.path(),
                source_lock_digest: &self.source_lock_digest,
            }
        }

        /// Resolves the fixture request without changing its seeded lockfile or graph.
        fn resolve(&self) -> Result<GeneratedWorkspace> {
            self.resolve_with_reconcile(|_| Ok(()))
        }

        /// Resolves the fixture request with a caller-selected reconciliation behavior.
        fn resolve_with_reconcile(
            &self,
            reconcile: impl FnOnce(&Path) -> Result<()>,
        ) -> Result<GeneratedWorkspace> {
            resolve_generated_workspace(self.target.path(), &self.request(), reconcile, |_| Ok(()))
        }

        /// Publishes one fixture workspace and returns it with its owning fixture.
        fn published() -> (Self, GeneratedWorkspace) {
            let fixture = Self::new();
            let workspace = fixture.resolve().unwrap();
            (fixture, workspace)
        }

        /// Derives the final full-digest workspace directory for this request.
        fn final_workspace(&self) -> std::path::PathBuf {
            self.target
                .path()
                .join("boomerang/generated/v1/descriptor")
                .join(self.request().identity.lowercase_hex())
        }

        /// Returns the first tool-managed directory beneath the target anchor.
        fn generated_root(&self) -> std::path::PathBuf {
            self.target.path().join("boomerang")
        }

        /// Rewrites a published short target's full request digest for collision injection.
        fn replace_marker_request(&self, workspace: &GeneratedWorkspace, request: &str) {
            let marker_path = workspace.target_directory().join(".boomerang-request");
            let mut marker: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
            marker["request"] = serde_json::Value::String(request.to_owned());
            std::fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        }
    }

    #[test]
    fn changed_cached_source_fails_closed() {
        let (fixture, workspace) = CacheFixture::published();
        std::fs::write(workspace.source_path(), b"changed").unwrap();
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(error.contains("generated source"), "{error}");
        assert!(error.contains("canonical input"), "{error}");
    }

    #[test]
    fn locked_target_rejects_coordinated_source_and_record_mutation() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        let changed = b"fn main() { panic!() }\n";
        std::fs::write(workspace.source_path(), changed).unwrap();
        let mut record: super::CacheRecord =
            serde_json::from_slice(&std::fs::read(&workspace.marker_path).unwrap()).unwrap();
        record.source_hash = super::hash_bytes(changed);
        std::fs::write(&workspace.marker_path, serde_json::to_vec(&record).unwrap()).unwrap();
        let closure_ran = std::cell::Cell::new(false);

        let error = workspace
            .with_locked_target(|_| {
                closure_ran.set(true);
                Ok(())
            })
            .unwrap_err()
            .to_string();

        assert!(error.contains("generated source"), "{error}");
        assert!(error.contains("canonical input"), "{error}");
        assert!(!closure_ran.get());
    }

    #[test]
    fn failed_reconciliation_never_publishes_request_directory() {
        let fixture = CacheFixture::new();
        let error = fixture
            .resolve_with_reconcile(|_| anyhow::bail!("intentional reconciliation failure"))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("intentional reconciliation failure"));
        assert!(!fixture.final_workspace().exists());
    }

    #[test]
    fn unexpected_workspace_entry_fails_closed() {
        let (fixture, workspace) = CacheFixture::published();
        std::fs::write(workspace.directory().join("unexpected"), b"x").unwrap();
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(
            error.contains("unexpected generated workspace entry"),
            "{error}"
        );
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
        let snapshot = super::read_verified_source_lock(
            fixture.source_lockfile.path(),
            &fixture.source_lock_digest,
        )
        .unwrap();
        let parent = fixture.source_lockfile.path().parent().unwrap();
        let mut replacement = tempfile::NamedTempFile::new_in(parent).unwrap();
        replacement.write_all(b"replacement lock").unwrap();
        replacement.flush().unwrap();
        std::fs::rename(replacement.path(), fixture.source_lockfile.path()).unwrap();

        assert_eq!(snapshot, b"version = 4\n");
        assert_eq!(
            std::fs::read(fixture.source_lockfile.path()).unwrap(),
            b"replacement lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_managed_parent_is_rejected() {
        let fixture = CacheFixture::new();
        std::os::unix::fs::symlink(fixture.outside.path(), fixture.generated_root()).unwrap();
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(error.contains("is not a real directory"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn linked_managed_parent_is_rejected() {
        let fixture = CacheFixture::new();
        if let Err(error) =
            std::os::windows::fs::symlink_dir(fixture.outside.path(), fixture.generated_root())
        {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("failed to create directory link: {error}");
        }
        let error = fixture.resolve().unwrap_err().to_string();
        assert!(error.contains("is not a real directory"), "{error}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_attribute_is_detected() {
        assert!(super::windows_file_attributes_are_reparse(0x400));
        assert!(!super::windows_file_attributes_are_reparse(0));
    }

    #[test]
    fn short_target_collision_fails_closed() {
        let (fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        fixture.replace_marker_request(
            &workspace,
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        );
        let error = workspace
            .with_locked_target(|_| Ok(()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("target locator collision"), "{error}");
    }

    #[test]
    fn concurrent_identical_publishers_reuse_one_workspace() {
        let fixture = Arc::new(CacheFixture::new());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let fixture = fixture.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    fixture.resolve().unwrap().directory().to_path_buf()
                })
            })
            .collect::<Vec<_>>();
        let paths = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(paths[0], paths[1]);
    }

    #[test]
    fn removed_short_target_is_recreated_with_its_full_marker() {
        let (_fixture, workspace) = CacheFixture::published();
        workspace.with_locked_target(|_| Ok(())).unwrap();
        std::fs::remove_dir_all(workspace.target_directory()).unwrap();
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
        std::fs::create_dir_all(fixture.generated_root().join(".staging-crash")).unwrap();
        assert!(fixture.resolve().is_ok());
    }
}
