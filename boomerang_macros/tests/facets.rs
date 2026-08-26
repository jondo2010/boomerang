use std::{fs, path::PathBuf, process::Command};

fn cargo_check(fixture: &str, args: &[&str]) -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture)
        .join("Cargo.toml");
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("macros crate should be in the workspace")
        .join("target/facet-fixtures");
    let output = Command::new(env!("CARGO"))
        .arg("check")
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
