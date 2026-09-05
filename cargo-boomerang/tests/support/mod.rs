#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

pub fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

pub fn copied_fixture_workspace() -> tempfile::TempDir {
    fn copy_tree(source: &Path, destination: &Path) {
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == "target" {
                continue;
            }
            let destination = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir(&destination).unwrap();
                copy_tree(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    let source = fixture_workspace();
    let destination = tempfile::tempdir_in(source.parent().unwrap()).unwrap();
    copy_tree(&source, destination.path());
    destination
}

pub fn shared_target(lane: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cargo-boomerang-fixtures");
    std::fs::create_dir_all(&root).unwrap();
    root.join(lane)
}

pub fn toolchain_target() -> PathBuf {
    shared_target("toolchain")
}

pub fn reset_deployment_output(target: &Path, deployment: &str) {
    let output = target.join("boomerang").join(deployment);
    if output.exists() {
        std::fs::remove_dir_all(output).unwrap();
    }
}

pub fn toolchain_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Removes terminal styling before making semantic assertions about CLI output.
pub fn without_ansi(output: &str) -> String {
    anstream::adapter::strip_str(output).to_string()
}

/// Asserts the complete ordered sequence of cargo-boomerang progress labels.
pub fn assert_progress_phases(stderr: &str, expected: &[&str]) {
    const PHASES: [&str; 7] = [
        "Analyzing",
        "Generating",
        "Building",
        "Validating",
        "Bundling",
        "Publishing",
        "Running",
    ];
    let plain_stderr = without_ansi(stderr);
    let actual = plain_stderr
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| PHASES.contains(word))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "unexpected progress sequence:\n{stderr}");
}

struct TargetDirectoryGuard(Option<OsString>);

impl Drop for TargetDirectoryGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(previous) => unsafe { std::env::set_var("CARGO_TARGET_DIR", previous) },
            None => unsafe { std::env::remove_var("CARGO_TARGET_DIR") },
        }
    }
}

pub fn with_target_directory<T>(target: &Path, operation: impl FnOnce() -> T) -> T {
    static LOCK: Mutex<()> = Mutex::new(());
    let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _guard = TargetDirectoryGuard(std::env::var_os("CARGO_TARGET_DIR"));
    unsafe { std::env::set_var("CARGO_TARGET_DIR", target) };
    operation()
}
