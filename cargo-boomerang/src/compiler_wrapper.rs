//! Host compiler-wrapper utility used by cargo-boomerang's generated Cargo builds.
//!
//! The installed `cargo-boomerang` executable dispatches here when Cargo invokes it through
//! `RUSTC_WRAPPER`; the wrapper is therefore built once with the deployment tool instead of being
//! generated for each application workspace.

#[path = "compiler_wrapper/utility.rs"]
mod utility;

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const INVOCATION_ENV: &str = "BOOMERANG_INTERNAL_COMPILER_WRAPPER";
const EXECUTABLE_ENV: &str = "BOOMERANG_COMPILER_WRAPPER";
const IDENTITY_REQUEST: &str = "identity";
const IDENTITY_PREFIX: &str = "boomerang-compiler-wrapper-v1:";

/// Resolved compiler-wrapper executable and its reported semantic identity.
pub(crate) struct Executable {
    path: PathBuf,
    identity: String,
}

impl Executable {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn identity(&self) -> &[u8] {
        self.identity.as_bytes()
    }

    pub(crate) fn into_path(self) -> PathBuf {
        self.path
    }
}

/// Returns whether the current `cargo-boomerang` process was selected as Cargo's compiler wrapper.
#[doc(hidden)]
pub fn is_invocation() -> bool {
    env::var_os(INVOCATION_ENV).as_deref() == Some("1".as_ref())
}

/// Returns whether another cargo-boomerang process requested this utility's semantic identity.
#[doc(hidden)]
pub fn is_identity_request() -> bool {
    env::var_os(INVOCATION_ENV).as_deref() == Some(IDENTITY_REQUEST.as_ref())
}

/// Prints the compiler-wrapper semantic identity for another cargo-boomerang process.
#[doc(hidden)]
pub fn print_identity() {
    println!("{}", local_identity());
}

/// Runs the compiler-wrapper protocol and terminates with the wrapped compiler's status.
#[doc(hidden)]
pub fn main() -> ! {
    utility::main()
}

/// Resolves and validates the host utility used as `RUSTC_WRAPPER`.
pub(crate) fn executable() -> Result<Executable, String> {
    let path = resolve_path()?;
    let identity = reported_identity(&path)?;
    Ok(Executable { path, identity })
}

fn resolve_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(EXECUTABLE_ENV).filter(|path| !path.is_empty()) {
        return require_file(PathBuf::from(path));
    }
    let current = env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    if current.file_stem() == Some(Path::new("cargo-boomerang").as_os_str()) {
        return require_file(current);
    }
    let filename = format!("cargo-boomerang{}", env::consts::EXE_SUFFIX);
    if let Some(path) = env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(&filename))
            .find(|candidate| candidate.is_file())
    }) {
        return require_file(path);
    }
    Err(format!(
        "library caller could not find cargo-boomerang on PATH; set {EXECUTABLE_ENV} to its executable"
    ))
}

fn require_file(path: PathBuf) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(&path).map_err(|error| {
        format!(
            "failed to inspect compiler wrapper {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "compiler wrapper {} is not a regular file",
            path.display()
        ));
    }
    std::fs::canonicalize(&path).map_err(|error| {
        format!(
            "failed to canonicalize compiler wrapper {}: {error}",
            path.display()
        )
    })
}

fn local_identity() -> String {
    format!(
        "{IDENTITY_PREFIX}{}",
        blake3::hash(include_bytes!("compiler_wrapper/utility.rs")).to_hex()
    )
}

fn reported_identity(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .env(INVOCATION_ENV, IDENTITY_REQUEST)
        .output()
        .map_err(|error| {
            format!(
                "failed to query compiler wrapper {} identity: {error}",
                path.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "compiler wrapper {} rejected the identity query: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let identity = std::str::from_utf8(&output.stdout)
        .map_err(|_| {
            format!(
                "compiler wrapper {} returned a non-UTF-8 identity",
                path.display()
            )
        })?
        .trim();
    let hash = identity.strip_prefix(IDENTITY_PREFIX).ok_or_else(|| {
        format!(
            "compiler wrapper {} returned an incompatible identity",
            path.display()
        )
    })?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "compiler wrapper {} returned a malformed identity",
            path.display()
        ));
    }
    Ok(identity.to_owned())
}
