#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
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
