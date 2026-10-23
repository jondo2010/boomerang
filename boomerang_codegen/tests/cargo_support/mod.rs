//! Shared Cargo fixture paths and application environment isolation.

use std::{path::PathBuf, process::Command};

/// Keep nested coverage objects beneath the coverage target for explicit collection.
pub fn target_directory(lane: &str) -> PathBuf {
    if std::env::var_os("CARGO_LLVM_COV").is_some() {
        let target = std::env::var_os("CARGO_LLVM_COV_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                    .parent()
                    .expect("Cargo test temporary directory has a target parent")
                    .to_path_buf()
            });
        target.join("native-fixtures").join(lane)
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target")
            .join(lane)
    }
}

/// Clear application configuration while preserving cargo-llvm-cov instrumentation.
pub fn clean_environment(command: &mut Command) {
    let coverage = std::env::var_os("CARGO_LLVM_COV").is_some();
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("BOOMERANG_")
            || name.starts_with("NATIVE_VEHICLE_")
            || matches!(
                name.as_ref(),
                "RUSTC_WORKSPACE_WRAPPER" | "CARGO_BUILD_TARGET"
            )
            || (!coverage
                && matches!(
                    name.as_ref(),
                    "RUSTC_WRAPPER" | "RUSTFLAGS" | "CARGO_ENCODED_RUSTFLAGS"
                ))
        {
            command.env_remove(key);
        }
    }
}
