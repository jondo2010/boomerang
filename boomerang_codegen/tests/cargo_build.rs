#![cfg(feature = "cargo-build")]
mod common;
use boomerang_codegen::{cargo_build::CargoBuild, *};
use common::*;
use std::{collections::BTreeMap, fs};

#[test]
fn publication_is_atomic_unchanged_and_failure_invalidates_only_owned_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("unrelated"), "keep").unwrap();
    let adapter = CargoBuild::from_context(dir.path(), target()).unwrap();
    let first = ready();
    let path = adapter.compile(&first, &mut Vec::new()).unwrap();
    let old_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_time))
        .unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let source = fs::read(&path).unwrap();
    adapter.compile(&first, &mut Vec::new()).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read(&path).unwrap(), source);
    let mut changed = compilation();
    register(
        &mut changed,
        "changed",
        "crate::changed::__boomerang::payload",
    );
    select(&mut changed, "first", "changed");
    select(&mut changed, "second", "changed");
    adapter.compile(&changed, &mut Vec::new()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        changed.compile().unwrap().source
    );
    assert_ne!(fs::read(&path).unwrap(), source);
    let leftover = dir.path().join(".boomerang-interrupted");
    fs::write(&leftover, "incomplete").unwrap();
    adapter.compile(&first, &mut Vec::new()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
    assert!(adapter.compile(&compilation(), &mut Vec::new()).is_err());
    assert!(
        !path.exists(),
        "failed generation must invalidate stale success"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("unrelated")).unwrap(),
        "keep"
    );
    adapter.compile(&first, &mut Vec::new()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
}
#[test]
fn declared_files_environment_and_target_cfg_emit_valid_directives() {
    let dir = tempfile::tempdir().unwrap();
    let values = BTreeMap::from([
        ("TARGET".into(), "x86_64-unknown-linux-gnu".into()),
        ("OUT_DIR".into(), dir.path().to_str().unwrap().into()),
        ("CARGO_CFG_TARGET_ARCH".into(), "x86_64".into()),
        ("CARGO_CFG_TARGET_FEATURE".into(), "sse,sse2".into()),
    ]);
    let mut adapter = CargoBuild::from_values(&values).unwrap();
    assert_eq!(adapter.target().cfg()["CARGO_CFG_TARGET_ARCH"], "x86_64");
    adapter.track_file("declaration.rs").unwrap();
    adapter.track_env("APP_VARIANT").unwrap();
    let mut c = Compilation::new(
        topology(),
        configuration(),
        boomerang_builder::compiler::FederateId::new("local").unwrap(),
        adapter.target().clone(),
    );
    register(
        &mut c,
        "controller",
        "crate::controller::__boomerang::payload",
    );
    select(&mut c, "first", "controller");
    select(&mut c, "second", "controller");
    let mut directives = Vec::new();
    adapter.compile(&c, &mut directives).unwrap();
    let lines = String::from_utf8(directives).unwrap();
    for expected in [
        "cargo:rerun-if-changed=declaration.rs",
        "cargo:rerun-if-env-changed=APP_VARIANT",
        "cargo:rerun-if-env-changed=TARGET",
        "cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH",
        "cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FEATURE",
    ] {
        assert!(lines.lines().any(|l| l == expected), "{expected}: {lines}");
    }
}
#[test]
fn missing_context_and_directive_injection_are_rejected() {
    assert_eq!(
        CargoBuild::from_values(&BTreeMap::new())
            .unwrap_err()
            .diagnostic
            .code,
        DiagnosticCode::MissingTarget
    );
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = CargoBuild::from_context(dir.path(), target()).unwrap();
    for bad in ["X\ncargo:rustc-link-lib=bad", "X=bad", "", "X\rY"] {
        assert!(adapter.track_env(bad).is_err());
    }
    for bad in ["foo\ncargo:rustc-cfg=bad", "foo\rbar", ""] {
        assert!(adapter.track_file(bad).is_err());
    }
    let wrong = Compilation::new(
        topology(),
        configuration(),
        boomerang_builder::compiler::FederateId::new("local").unwrap(),
        TargetContext::new("aarch64-unknown-linux-gnu").unwrap(),
    );
    let output = adapter.compile(&ready(), &mut Vec::new()).unwrap();
    assert_eq!(
        adapter
            .compile(&wrong, &mut Vec::new())
            .unwrap_err()
            .diagnostic
            .code,
        DiagnosticCode::TargetMismatch
    );
    assert!(!output.exists());
}
#[cfg(unix)]
#[test]
fn owned_output_never_follows_an_escaping_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::NamedTempFile::new().unwrap();
    fs::write(external.path(), "untouched").unwrap();
    std::os::unix::fs::symlink(external.path(), dir.path().join("boomerang.rs")).unwrap();
    let adapter = CargoBuild::from_context(dir.path(), target()).unwrap();
    adapter.compile(&ready(), &mut Vec::new()).unwrap();
    assert_eq!(fs::read_to_string(external.path()).unwrap(), "untouched");
    assert!(!fs::symlink_metadata(dir.path().join("boomerang.rs"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[path = "common/coordination.rs"]
mod coordination_fixture;

#[test]
fn coordination_publication_preserves_bytes_directives_and_invalidates_stale_output() {
    let inputs = || coordination_fixture::inputs(1, 0, "serde-json", false);
    let dir = tempfile::tempdir().unwrap();
    let mut adapter = CargoBuild::from_context(
        dir.path(),
        TargetContext::new("aarch64-apple-darwin").unwrap(),
    )
    .unwrap();
    adapter.track_file("topology.rs").unwrap();
    adapter.track_env("APP_VARIANT").unwrap();
    fs::write(dir.path().join("unrelated"), "keep").unwrap();
    let mut directives = Vec::new();
    let path = adapter
        .compile_coordination(inputs(), &mut directives)
        .unwrap();
    assert_eq!(path, dir.path().join("boomerang.rs"));
    let source = fs::read(&path).unwrap();
    let old_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_time))
        .unwrap();
    adapter
        .compile_coordination(inputs(), &mut Vec::new())
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old_time);
    assert_eq!(fs::read(&path).unwrap(), source);
    let lines = String::from_utf8(directives).unwrap();
    for expected in [
        "cargo:rerun-if-changed=topology.rs",
        "cargo:rerun-if-env-changed=APP_VARIANT",
        "cargo:rerun-if-env-changed=TARGET",
    ] {
        assert!(lines.lines().any(|line| line == expected));
    }
    adapter
        .compile_coordination(
            coordination_fixture::inputs(1, 1, "serde-json", false),
            &mut Vec::new(),
        )
        .unwrap();
    assert_ne!(fs::read(&path).unwrap(), source);
    let mut invalid = inputs();
    invalid.bindings.clear();
    assert!(adapter
        .compile_coordination(invalid, &mut Vec::new())
        .is_err());
    assert!(!path.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("unrelated")).unwrap(),
        "keep"
    );
    adapter
        .compile_coordination(inputs(), &mut Vec::new())
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), source);
    assert!(fs::read_dir(dir.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".boomerang-")));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut invalid = inputs();
    invalid.bindings.clear();
    let error = adapter
        .compile_coordination(invalid, &mut Vec::new())
        .unwrap_err();
    assert_eq!(error.diagnostic.stage, Stage::Publish);
    assert_eq!(error.diagnostic.code, DiagnosticCode::Io);
    assert!(error
        .to_string()
        .contains("stale output could not be removed"));
}
