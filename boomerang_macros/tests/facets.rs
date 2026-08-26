use std::{fs, path::PathBuf, process::Command};

fn cargo(fixture: &str, subcommand: &str, args: &[&str]) -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture)
        .join("Cargo.toml");
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("macros crate should be in the workspace")
        .join("target/facet-fixtures");
    let output = Command::new(env!("CARGO"))
        .arg(subcommand)
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(manifest)
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .expect("cargo check should start");
    let _ = fs::remove_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture)
            .join("Cargo.lock"),
    );

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn cargo_check(fixture: &str, args: &[&str]) -> Result<(), String> {
    cargo(fixture, "check", args)
}

fn cargo_test(fixture: &str, args: &[&str]) -> Result<(), String> {
    cargo(fixture, "test", args)
}

#[test]
fn descriptor_mode_excludes_reaction_payloads() {
    cargo_check("descriptor-pass", &["--features", "__boomerang_descriptor"]).unwrap();
}

#[test]
fn descriptor_mode_rejects_unrecognized_closure_builder_code() {
    let stderr = cargo_check(
        "descriptor-rejects-body",
        &["--features", "__boomerang_descriptor"],
    )
    .expect_err("descriptor mode should reject arbitrary builder code");
    assert!(
        stderr.contains("deployment descriptor requires reaction! syntax"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn hosted_mode_preserves_metadata_free_reactors() {
    cargo_test("metadata-free", &[]).unwrap();
}

#[test]
fn descriptor_mode_excludes_metadata_free_reactor_payloads() {
    cargo_check("metadata-free", &["--features", "__boomerang_descriptor"]).unwrap();
}

#[test]
fn reserved_modes_conflict_for_complete_metadata() {
    let stderr = cargo_check(
        "descriptor-pass",
        &["--features", "__boomerang_descriptor __boomerang_payload"],
    )
    .expect_err("reserved modes should conflict");
    assert!(
        stderr.contains("__boomerang_descriptor and __boomerang_payload cannot both be enabled"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn reserved_modes_conflict_for_metadata_free_reactors() {
    let stderr = cargo_check(
        "metadata-free",
        &["--features", "__boomerang_descriptor __boomerang_payload"],
    )
    .expect_err("reserved modes should conflict");
    assert!(
        stderr.contains("__boomerang_descriptor and __boomerang_payload cannot both be enabled"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}
