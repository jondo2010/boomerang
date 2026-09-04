//! Persistent, content-addressed workspaces for generated Cargo crates.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
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
#[derive(Debug, serde::Deserialize, serde::Serialize)]
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

/// Invocation identity used to prove ownership before staging cleanup.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StagingOwnerRecord {
    /// Cache protocol version used to interpret the ownership record.
    schema: u32,
    /// Unique reserved staging-directory name created by this invocation.
    owner: String,
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
    /// Role whose generated workspace and short target are represented.
    role: GeneratedRole,
    /// Full request digest expected in cache and short-target records.
    request: String,
}

impl GeneratedWorkspace {
    /// Derives all stable workspace paths from one cache directory and target directory.
    fn for_directory(
        directory: PathBuf,
        target_directory: PathBuf,
        target_anchor: PathBuf,
        role: GeneratedRole,
        request: String,
    ) -> Self {
        Self {
            manifest_path: directory.join("Cargo.toml"),
            source_path: directory.join("src/main.rs"),
            lockfile_path: directory.join("Cargo.lock"),
            marker_path: directory.join("cache.json"),
            directory,
            target_directory,
            target_anchor,
            role,
            request,
        }
    }

    /// Validates the exact immutable tree and all content-derived record fields.
    fn read_validated_record(&self) -> Result<CacheRecord> {
        validate_workspace_tree(self)?;
        let record = fs::read(&self.marker_path)
            .with_context(|| format!("failed to read {}", self.marker_path.display()))?;
        let record: CacheRecord = serde_json::from_slice(&record)
            .with_context(|| format!("failed to decode {}", self.marker_path.display()))?;
        let generated_lock_hash = hash_file(&self.lockfile_path, "generated lockfile")?;
        let target_locator = self
            .target_directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("generated workspace has no target locator"))?;
        if record.schema != GENERATED_CACHE_SCHEMA
            || !is_full_digest(&record.request)
            || !is_full_digest(&record.manifest_hash)
            || !is_full_digest(&record.source_hash)
            || !is_full_digest(&record.generated_lock_hash)
            || !is_full_digest(&record.source_lock_hash)
            || record.manifest_hash != hash_file(&self.manifest_path, "generated manifest")?
            || record.source_hash != hash_file(&self.source_path, "generated source")?
            || record.generated_lock_hash != generated_lock_hash
            || record.target_locator != target_locator
        {
            bail!(
                "generated workspace cache record is invalid: {}",
                self.marker_path.display()
            );
        }
        Ok(record)
    }

    /// Validates that a content-checked record also belongs at its published cache path.
    fn validate_published_record(&self) -> Result<CacheRecord> {
        let record = self.read_validated_record()?;
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
        if record.role.directory_name() != role || record.request != identity {
            bail!(
                "generated workspace cache record is invalid: {}",
                self.marker_path.display()
            );
        }
        Ok(record)
    }

    /// Validates a published workspace against the current canonical request inputs.
    fn validate_for_request(&self, request: &GeneratedWorkspaceRequest<'_>) -> Result<()> {
        validate_canonical_file(&self.manifest_path, request.manifest, "generated manifest")?;
        validate_canonical_file(&self.source_path, request.source, "generated source")?;
        let record = self.read_validated_record()?;
        let expected_request = request.identity.lowercase_hex();
        let expected_source_lock = digest_hex(request.source_lock_digest);
        let expected_target = request.identity.short_target_name(request.role);
        if record.role != request.role
            || record.request != expected_request
            || record.manifest_hash != hash_bytes(request.manifest)
            || record.source_hash != hash_bytes(request.source)
            || record.source_lock_hash != expected_source_lock
            || record.target_locator != expected_target
        {
            bail!(
                "generated workspace cache record does not match canonical request: {}",
                self.marker_path.display()
            );
        }
        Ok(())
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
    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Returns the reconciled generated Cargo lockfile path.
    pub(crate) fn lockfile_path(&self) -> &Path {
        &self.lockfile_path
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
        self.validate_published_record()?;
        self.prepare_short_target()?;
        let lock_path = self.target_directory().join(".boomerang-request");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {}", lock_path.display()))?;
        let mut lock = TargetLock::acquire(lock_file, &lock_path)?;
        self.validate_target_marker()?;
        self.validate_published_record()?;
        let result = operation(self.target_directory());
        lock.unlock()?;
        result
    }

    /// Creates or validates the short Cargo target using failure-atomic publication.
    fn prepare_short_target(&self) -> Result<()> {
        let target_parent = prepare_managed_directories(&self.target_anchor, &["b"])?;
        if target_parent.join(
            self.target_directory
                .file_name()
                .expect("short target always has a locator"),
        ) != self.target_directory
        {
            bail!(
                "generated target locator escaped its managed parent: {}",
                self.target_directory.display()
            );
        }
        match fs::symlink_metadata(&self.target_directory) {
            Ok(_) => {
                require_real_directory(&self.target_directory, "generated target directory")?;
                validate_canonical_containment(
                    &self.target_anchor,
                    &self.target_directory,
                    "generated target directory",
                )?;
                return self.validate_target_marker();
            }
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
        if let Err(error) = fs::write(&marker_path, &marker_bytes)
            .with_context(|| format!("failed to write {}", marker_path.display()))
        {
            return staging.finish_error(error);
        }
        if let Err(error) = staging.prepare_for_publication() {
            return staging.finish_error(error);
        }
        if let Err(error) = validate_target_directory(staging.path(), &marker) {
            return staging.finish_error(error);
        }
        match rename_noreplace(staging.path(), &self.target_directory) {
            Ok(()) => {
                staging.disarm();
                self.validate_target_marker()
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let winner = self.validate_target_marker();
                let result = winner.and_then(|()| {
                    let candidate = fs::read(&marker_path)
                        .with_context(|| format!("failed to read {}", marker_path.display()))?;
                    let published = fs::read(self.target_directory.join(".boomerang-request"))
                        .with_context(|| {
                            format!(
                                "failed to read {}",
                                self.target_directory.join(".boomerang-request").display()
                            )
                        })?;
                    if candidate != published {
                        bail!(
                            "concurrent generated target winner differs from candidate {}",
                            self.target_directory.display()
                        );
                    }
                    Ok(())
                });
                staging.finish_result(result)
            }
            Err(error) => {
                let error = anyhow!(error).context(format!(
                    "failed to publish generated target directory {}",
                    self.target_directory.display()
                ));
                staging.finish_error(error)
            }
        }
    }

    /// Builds the exact marker expected in this workspace's short target.
    fn expected_target_marker(&self) -> TargetMarker {
        TargetMarker {
            schema: GENERATED_CACHE_SCHEMA,
            role: self.role,
            request: self.request.clone(),
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
        require_regular_file(&marker_path, "generated target marker")?;
        let bytes = fs::read(&marker_path)
            .with_context(|| format!("failed to read {}", marker_path.display()))?;
        let marker: TargetMarker = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", marker_path.display()))?;
        if marker.request != self.request {
            bail!(
                "target locator collision at {}: expected request {}, found {}",
                self.target_directory.display(),
                self.request,
                marker.request
            );
        }
        if marker.schema != GENERATED_CACHE_SCHEMA
            || marker.role != self.role
            || !is_full_digest(&marker.request)
        {
            bail!(
                "generated target marker is invalid: {}",
                marker_path.display()
            );
        }
        Ok(())
    }
}

/// Returns whether text is exactly one lowercase 256-bit hexadecimal digest.
fn is_full_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    require_regular_file(path, description)?;
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read {description} {}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

/// Requires a path to name a real regular file rather than a redirected entry.
fn require_regular_file(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
        bail!(
            "{description} {} is not a real regular file",
            path.display()
        );
    }
    Ok(())
}

/// Requires a path to name a real directory rather than a redirected entry.
fn require_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {description} {}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
        bail!("{description} {} is not a real directory", path.display());
    }
    Ok(())
}

/// Requires an unpublished short target to contain only its exact regular marker file.
fn validate_target_directory(directory: &Path, expected: &TargetMarker) -> Result<()> {
    require_real_directory(directory, "generated target directory")?;
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read generated target {}", directory.display()))?;
    let entry = entries
        .next()
        .transpose()
        .with_context(|| format!("failed to read generated target {}", directory.display()))?
        .ok_or_else(|| {
            anyhow!(
                "generated target directory is empty: {}",
                directory.display()
            )
        })?;
    if entry.file_name() != std::ffi::OsStr::new(".boomerang-request") || entries.next().is_some() {
        bail!(
            "generated target staging contains unexpected entries: {}",
            directory.display()
        );
    }
    require_regular_file(&entry.path(), "generated target marker")?;
    let bytes = fs::read(entry.path())
        .with_context(|| format!("failed to read {}", entry.path().display()))?;
    let marker: TargetMarker = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to decode {}", entry.path().display()))?;
    if marker.schema != expected.schema
        || marker.role != expected.role
        || marker.request != expected.request
        || !is_full_digest(&marker.request)
    {
        bail!(
            "generated target marker is invalid: {}",
            entry.path().display()
        );
    }
    Ok(())
}

/// Requires cached bytes to equal the freshly rendered canonical input bytes.
fn validate_canonical_file(path: &Path, expected: &[u8], description: &str) -> Result<()> {
    require_regular_file(path, description)?;
    let actual = fs::read(path)
        .with_context(|| format!("failed to read {description} {}", path.display()))?;
    if actual != expected {
        bail!(
            "{description} {} differs from its canonical input",
            path.display()
        );
    }
    Ok(())
}

/// Requires exactly the fixed generated Cargo workspace entries and entry kinds.
fn validate_workspace_tree(workspace: &GeneratedWorkspace) -> Result<()> {
    require_real_directory(workspace.directory(), "generated workspace")?;
    let mut top_level = std::collections::BTreeSet::new();
    for entry in fs::read_dir(workspace.directory()).with_context(|| {
        format!(
            "failed to read generated workspace {}",
            workspace.directory().display()
        )
    })? {
        let entry = entry.with_context(|| {
            format!(
                "failed to read generated workspace entry in {}",
                workspace.directory().display()
            )
        })?;
        let name = entry.file_name();
        if !matches!(
            name.to_str(),
            Some("Cargo.toml" | "Cargo.lock" | "cache.json" | "src")
        ) {
            bail!(
                "unexpected generated workspace entry {}",
                entry.path().display()
            );
        }
        top_level.insert(name);
    }
    for expected in ["Cargo.toml", "Cargo.lock", "cache.json", "src"] {
        if !top_level.contains(std::ffi::OsStr::new(expected)) {
            bail!(
                "missing generated workspace entry {}",
                workspace.directory().join(expected).display()
            );
        }
    }
    require_regular_file(&workspace.manifest_path, "generated manifest")?;
    require_regular_file(&workspace.lockfile_path, "generated lockfile")?;
    require_regular_file(&workspace.marker_path, "generated cache record")?;
    let source_directory = workspace
        .source_path
        .parent()
        .expect("generated source path always has a parent");
    require_real_directory(source_directory, "generated source directory")?;
    let source_entries = fs::read_dir(source_directory).with_context(|| {
        format!(
            "failed to read generated source directory {}",
            source_directory.display()
        )
    })?;
    for entry in source_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read generated source entry in {}",
                source_directory.display()
            )
        })?;
        if entry.file_name() != std::ffi::OsStr::new("main.rs") {
            bail!(
                "unexpected generated workspace entry {}",
                entry.path().display()
            );
        }
    }
    require_regular_file(&workspace.source_path, "generated source")
}

/// Owns the exclusive advisory lock for one short generated Cargo target directory.
struct TargetLock {
    /// Locked request file, absent only after the normal explicit unlock succeeds.
    file: Option<File>,
    /// Lockfile path retained for contextual lock and unlock diagnostics.
    path: PathBuf,
}

impl TargetLock {
    /// Takes an exclusive advisory lock on an already-open request lockfile.
    fn acquire(file: File, path: &Path) -> Result<Self> {
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", path.display()))?;
        Ok(Self {
            file: Some(file),
            path: path.to_path_buf(),
        })
    }

    /// Explicitly releases the exclusive advisory lock on the normal return path.
    fn unlock(&mut self) -> Result<()> {
        if let Some(file) = self.file.take() {
            FileExt::unlock(&file)
                .with_context(|| format!("failed to unlock {}", self.path.display()))?;
        }
        Ok(())
    }
}

impl Drop for TargetLock {
    /// Best-effort releases a lock if an error or panic bypasses the normal unlock path.
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
        }
    }
}

/// Owns one reserved staging directory until publication or validated cleanup.
struct OwnedStagingDirectory {
    /// Reserved staging path created by this invocation.
    path: PathBuf,
    /// Canonical managed parent beside the final publication path.
    parent: PathBuf,
    /// Reserved prefix required on the staging directory name.
    prefix: &'static str,
    /// Unique directory name recorded in the invocation-owned marker.
    owner: String,
    /// Whether failure handling still owns the staging directory.
    armed: bool,
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
            .ok_or_else(|| {
                anyhow!(
                    "unsafe staging cleanup unavailable and residue was retained: name is not valid UTF-8"
                )
            })?
            .to_owned();
        let staging = Self {
            path,
            parent,
            prefix,
            owner,
            armed: true,
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
        fs::remove_file(self.owner_marker_path()).with_context(|| {
            format!(
                "failed to remove staging owner marker {}",
                self.owner_marker_path().display()
            )
        })
    }

    /// Stops cleanup after a successful rename moved the staging directory.
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Cleans a staging directory after preserving a successful or failed operation result.
    fn finish_result<T>(&mut self, result: Result<T>) -> Result<T> {
        if !self.armed {
            return result;
        }
        let cleanup = self.cleanup();
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

    /// Cleans a staging directory while returning the operation error that triggered cleanup.
    fn finish_error<T>(&mut self, error: anyhow::Error) -> Result<T> {
        self.finish_result(Err(error))
    }

    /// Revalidates ownership and removes only this invocation's reserved staging directory.
    fn cleanup(&mut self) -> Result<()> {
        self.ensure_owner_marker()?;
        self.validate_ownership()?;
        fs::remove_dir_all(&self.path).with_context(|| {
            format!(
                "failed to remove validated generated staging directory {}",
                self.path.display()
            )
        })?;
        self.armed = false;
        Ok(())
    }

    /// Returns the reserved invocation-owner marker path.
    fn owner_marker_path(&self) -> PathBuf {
        self.path.join(".boomerang-staging-owner")
    }

    /// Writes the invocation-owner marker without replacing an existing entry.
    fn write_owner_marker(&self) -> Result<()> {
        let marker_path = self.owner_marker_path();
        let record = StagingOwnerRecord {
            schema: GENERATED_CACHE_SCHEMA,
            owner: self.owner.clone(),
        };
        let bytes = serde_json::to_vec(&record)
            .context("failed to encode generated staging owner marker")?;
        let mut marker = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker_path)
            .with_context(|| format!("failed to create {}", marker_path.display()))?;
        use std::io::Write as _;
        marker
            .write_all(&bytes)
            .with_context(|| format!("failed to write {}", marker_path.display()))
    }

    /// Restores a removed owner marker only while the staging path remains safely contained.
    fn ensure_owner_marker(&self) -> Result<()> {
        match fs::symlink_metadata(self.owner_marker_path()) {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.validate_path_and_name()?;
                self.write_owner_marker()
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to inspect staging owner marker {}",
                    self.owner_marker_path().display()
                )
            }),
        }
    }

    /// Validates the canonical parent, reserved prefix, and exact owner-marker contents.
    fn validate_ownership(&self) -> Result<()> {
        self.validate_path_and_name()?;
        let marker_path = self.owner_marker_path();
        require_regular_file(&marker_path, "generated staging owner marker")?;
        let bytes = fs::read(&marker_path)
            .with_context(|| format!("failed to read {}", marker_path.display()))?;
        let record: StagingOwnerRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", marker_path.display()))?;
        if record.schema != GENERATED_CACHE_SCHEMA || record.owner != self.owner {
            bail!(
                "unsafe staging cleanup refused for owner mismatch at {}",
                self.path.display()
            );
        }
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
        if !name.starts_with(self.prefix) || name != self.owner {
            bail!(
                "unsafe staging cleanup refused for unowned path {}",
                self.path.display()
            );
        }
        let canonical = fs::canonicalize(&self.path)
            .with_context(|| format!("failed to canonicalize {}", self.path.display()))?;
        if canonical.parent() != Some(self.parent.as_path()) {
            bail!(
                "unsafe staging cleanup refused for escaped path {}",
                self.path.display()
            );
        }
        Ok(())
    }
}

/// Creates or validates the caller-selected target anchor without following its final component.
fn prepare_target_anchor(target: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(target) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(target)
                .with_context(|| format!("failed to prepare target anchor {}", target.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect target anchor {}", target.display()));
        }
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
        match fs::symlink_metadata(&child) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&child) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to prepare managed directory {}", child.display())
                    });
                }
            },
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect managed directory {}", child.display())
                });
            }
        }
        require_real_directory(&child, "managed path component")?;
        let canonical =
            validate_canonical_containment(&canonical_anchor, &child, "managed path component")?;
        if canonical.parent() != Some(current.as_path()) {
            bail!(
                "managed path component escaped its parent: {}",
                child.display()
            );
        }
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
    if !canonical.starts_with(anchor) {
        bail!(
            "{description} {} escapes target anchor {}",
            path.display(),
            anchor.display()
        );
    }
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
    for (candidate_path, winner_path, description) in [
        (
            candidate.manifest_path(),
            winner.manifest_path(),
            "generated manifest",
        ),
        (
            candidate.lockfile_path(),
            winner.lockfile_path(),
            "generated lockfile",
        ),
        (
            candidate.source_path(),
            winner.source_path(),
            "generated source",
        ),
        (
            candidate.marker_path.as_path(),
            winner.marker_path.as_path(),
            "generated cache record",
        ),
    ] {
        let candidate_bytes = fs::read(candidate_path).with_context(|| {
            format!("failed to read {description} {}", candidate_path.display())
        })?;
        let winner_bytes = fs::read(winner_path)
            .with_context(|| format!("failed to read {description} {}", winner_path.display()))?;
        if candidate_bytes != winner_bytes {
            bail!(
                "concurrent generated workspace winner has different {description}: {}",
                winner_path.display()
            );
        }
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
    let source_lock_hash = hash_file(request.source_lockfile, "source workspace lockfile")?;
    if source_lock_hash != digest_hex(request.source_lock_digest) {
        bail!(
            "source workspace lockfile {} differs from its canonical digest",
            request.source_lockfile.display()
        );
    }
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
    let workspace = GeneratedWorkspace::for_directory(
        final_directory.clone(),
        target,
        target_anchor.clone(),
        request.role,
        identity.clone(),
    );

    match fs::symlink_metadata(&final_directory) {
        Ok(_) => {
            require_real_directory(&final_directory, "generated workspace")?;
            validate_canonical_containment(
                &target_anchor,
                &final_directory,
                "generated workspace",
            )?;
            workspace.validate_for_request(request)?;
            workspace.validate_published_record()?;
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
    let staged = GeneratedWorkspace::for_directory(
        staging.path().to_path_buf(),
        workspace.target_directory.clone(),
        target_anchor,
        request.role,
        identity.clone(),
    );
    let prepare_candidate = (|| -> Result<()> {
        let source_directory = staged
            .source_path()
            .parent()
            .expect("generated source path always has a parent");
        fs::create_dir(source_directory)
            .with_context(|| format!("failed to prepare {}", source_directory.display()))?;
        fs::write(staged.manifest_path(), request.manifest)
            .with_context(|| format!("failed to write {}", staged.manifest_path().display()))?;
        fs::write(staged.source_path(), request.source)
            .with_context(|| format!("failed to write {}", staged.source_path().display()))?;
        fs::copy(request.source_lockfile, staged.lockfile_path())
            .with_context(|| format!("failed to seed {}", staged.lockfile_path().display()))?;
        reconcile(staged.directory())?;
        validate_graph(staged.directory())?;
        let marker = CacheRecord {
            schema: GENERATED_CACHE_SCHEMA,
            role: request.role,
            request: identity,
            manifest_hash: hash_bytes(request.manifest),
            source_hash: hash_bytes(request.source),
            generated_lock_hash: hash_file(staged.lockfile_path(), "generated lockfile")?,
            source_lock_hash: digest_hex(request.source_lock_digest),
            target_locator: request.identity.short_target_name(request.role),
        };
        fs::write(
            staged.marker_path.clone(),
            serde_json::to_vec(&marker)
                .context("failed to encode generated workspace cache record")?,
        )
        .with_context(|| format!("failed to write {}", staged.marker_path.display()))?;
        staging.prepare_for_publication()?;
        staged.validate_for_request(request)?;
        Ok(())
    })();
    if let Err(error) = prepare_candidate {
        return staging.finish_error(error);
    }

    match rename_noreplace(staged.directory(), &final_directory) {
        Ok(()) => {
            staging.disarm();
            Ok(workspace)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let winner = (|| -> Result<GeneratedWorkspace> {
                require_real_directory(&final_directory, "generated workspace")?;
                validate_canonical_containment(
                    &workspace.target_anchor,
                    &final_directory,
                    "generated workspace",
                )?;
                workspace.validate_for_request(request)?;
                workspace.validate_published_record()?;
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
            staging.finish_error(error)
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
