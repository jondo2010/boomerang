//! Cargo compiler wrapper for facet-specific Rust configuration.

mod utility;

use std::{env, path::PathBuf};

pub(crate) const INVOCATION_ENV: &str = "BOOMERANG_INTERNAL_COMPILER_WRAPPER";
const EXECUTABLE_ENV: &str = "BOOMERANG_COMPILER_WRAPPER";
pub(crate) const SEMANTICS: &[u8] = include_bytes!("utility.rs");

/// Returns whether Cargo invoked this process as its Rust compiler wrapper.
#[doc(hidden)]
pub fn is_invocation() -> bool {
    env::var_os(INVOCATION_ENV).as_deref() == Some("1".as_ref())
}

/// Runs the Rust compiler wrapper and exits with the compiler status.
#[doc(hidden)]
pub fn main() -> ! {
    utility::main()
}

/// Resolves the cargo-boomerang executable used as `RUSTC_WRAPPER`.
pub(crate) fn executable() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(EXECUTABLE_ENV).filter(|path| !path.is_empty()) {
        return require_file(PathBuf::from(path));
    }
    let current = env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    if current.file_stem().and_then(|name| name.to_str()) == Some("cargo-boomerang") {
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
    let resolved = std::fs::canonicalize(&path).map_err(|error| {
        format!(
            "failed to resolve compiler wrapper {}: {error}",
            path.display()
        )
    })?;
    if !resolved.is_file() {
        return Err(format!(
            "compiler wrapper {} is not a regular file",
            resolved.display()
        ));
    }
    Ok(resolved)
}
