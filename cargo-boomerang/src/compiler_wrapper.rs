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
};

pub(crate) const INVOCATION_ENV: &str = "BOOMERANG_INTERNAL_COMPILER_WRAPPER";
const EXECUTABLE_ENV: &str = "BOOMERANG_COMPILER_WRAPPER";

/// Returns whether the current `cargo-boomerang` process was selected as Cargo's compiler wrapper.
#[doc(hidden)]
pub fn is_invocation() -> bool {
    env::var_os(INVOCATION_ENV).as_deref() == Some("1".as_ref())
}

/// Runs the compiler-wrapper protocol and terminates with the wrapped compiler's status.
#[doc(hidden)]
pub fn main() -> ! {
    utility::main()
}

/// Resolves the host utility used as `RUSTC_WRAPPER`.
pub(crate) fn executable() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(EXECUTABLE_ENV).filter(|path| !path.is_empty()) {
        return require_file(PathBuf::from(path));
    }
    let executable = env::current_exe()
        .map_err(|error| format!("failed to resolve cargo-boomerang executable: {error}"))?;
    let expected = Path::new("cargo-boomerang");
    if executable.file_stem() != expected.file_stem() {
        return Err(format!(
            "library caller must set {EXECUTABLE_ENV} to a cargo-boomerang executable"
        ));
    }
    require_file(executable)
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
    Ok(path)
}
