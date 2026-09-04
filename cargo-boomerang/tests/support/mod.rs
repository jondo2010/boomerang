use std::{
    ffi::OsString,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub fn fixture_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
}

#[allow(dead_code)]
pub fn shared_target(lane: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cargo-boomerang-fixtures");
    std::fs::create_dir_all(&root).unwrap();
    root.join(lane)
}

fn target_environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TargetDirectoryGuard {
    previous: Option<OsString>,
}

impl TargetDirectoryGuard {
    fn set(target: &Path) -> Self {
        let previous = std::env::var_os("CARGO_TARGET_DIR");
        unsafe { std::env::set_var("CARGO_TARGET_DIR", target) };
        Self { previous }
    }
}

impl Drop for TargetDirectoryGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => unsafe { std::env::set_var("CARGO_TARGET_DIR", previous) },
            None => unsafe { std::env::remove_var("CARGO_TARGET_DIR") },
        }
    }
}

pub fn with_target_directory<T>(target: &Path, operation: impl FnOnce() -> T) -> T {
    let _lock = target_environment_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let guard = TargetDirectoryGuard::set(target);
    let result = catch_unwind(AssertUnwindSafe(operation));
    drop(guard);
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}
