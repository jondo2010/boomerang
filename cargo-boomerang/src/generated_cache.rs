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
const GENERATED_CACHE_SCHEMA: u64 = 1;

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

/// Serialized completion marker proving that a cache directory was fully published.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct CacheMarker {
    /// Marker schema version, coupled to the generated-workspace cache schema.
    schema: u64,
    /// Role namespace that owns the completed workspace.
    role: GeneratedRole,
    /// Full hexadecimal identity used as the completed directory name.
    identity: String,
    /// Source lockfile digest from the request that produced this workspace.
    source_lock_digest: String,
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
    /// Completion marker validated before cache reuse and target-directory use.
    marker_path: PathBuf,
}

impl GeneratedWorkspace {
    /// Derives all stable workspace paths from one cache directory and target directory.
    fn for_directory(directory: PathBuf, target_directory: PathBuf) -> Self {
        Self {
            manifest_path: directory.join("Cargo.toml"),
            source_path: directory.join("src/main.rs"),
            lockfile_path: directory.join("Cargo.lock"),
            marker_path: directory.join("cache.json"),
            directory,
            target_directory,
        }
    }

    /// Decodes and checks the completion marker against this workspace's cache path.
    fn validate_marker(&self) -> Result<()> {
        let marker = fs::read(&self.marker_path)
            .with_context(|| format!("failed to read {}", self.marker_path.display()))?;
        let marker: CacheMarker = serde_json::from_slice(&marker)
            .with_context(|| format!("failed to decode {}", self.marker_path.display()))?;
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
        if marker.schema != GENERATED_CACHE_SCHEMA
            || marker.role.directory_name() != role
            || marker.identity != identity
            || marker.identity.len() != 64
            || !marker
                .identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || marker.source_lock_digest.len() != 64
            || !marker
                .source_lock_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!(
                "generated workspace cache marker is invalid: {}",
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
        fs::create_dir_all(self.target_directory())
            .with_context(|| format!("failed to prepare {}", self.target_directory().display()))?;
        let lock_path = self.target_directory().join(".boomerang-request");
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {}", lock_path.display()))?;
        let mut lock = TargetLock::acquire(lock_file, &lock_path)?;
        self.validate_marker()?;
        let result = operation(self.target_directory());
        lock.unlock()?;
        result
    }
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

/// Resolves or atomically publishes a generated workspace for one exact request.
pub(crate) fn resolve_generated_workspace(
    target_directory: &Path,
    request: &GeneratedWorkspaceRequest<'_>,
    reconcile: impl FnOnce(&Path) -> Result<()>,
    validate_graph: impl Fn(&Path) -> Result<()>,
) -> Result<GeneratedWorkspace> {
    let identity = request.identity.lowercase_hex();
    let parent = target_directory
        .join("boomerang/generated/v1")
        .join(request.role.directory_name());
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let final_directory = parent.join(&identity);
    let target = target_directory
        .join("boomerang/generated/v1/b")
        .join(request.identity.short_target_name(request.role));
    let workspace = GeneratedWorkspace::for_directory(final_directory.clone(), target);

    if final_directory.exists() {
        workspace.validate_marker()?;
        validate_graph(workspace.directory())?;
        return Ok(workspace);
    }

    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&parent)
        .with_context(|| format!("failed to prepare {}", parent.display()))?;
    let staged = GeneratedWorkspace::for_directory(
        staging.path().to_path_buf(),
        workspace.target_directory.clone(),
    );
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
    let marker = CacheMarker {
        schema: GENERATED_CACHE_SCHEMA,
        role: request.role,
        identity,
        source_lock_digest: blake3::Hash::from(*request.source_lock_digest)
            .to_hex()
            .to_string(),
    };
    fs::write(
        staged.marker_path.clone(),
        serde_json::to_vec(&marker).context("failed to encode generated workspace cache marker")?,
    )
    .with_context(|| format!("failed to write {}", staged.marker_path.display()))?;

    match rename_noreplace(staged.directory(), &final_directory) {
        Ok(()) => Ok(workspace),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            workspace.validate_marker()?;
            validate_graph(workspace.directory())?;
            Ok(workspace)
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to publish generated workspace {}",
                final_directory.display()
            )
        }),
    }
}

/// Selects the Cargo executable while leaving invocation details to the caller.
pub(crate) fn generated_cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}
