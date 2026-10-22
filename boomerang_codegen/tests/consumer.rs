//! Runs ordinary Cargo against a checked-in application with a checked-in main.
use std::{
    path::PathBuf,
    process::{Command, Output},
};
fn cargo(command: &str, features: &[&str]) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut command_line = Command::new(env!("CARGO"));
    command_line
        .current_dir(root.join("tests/fixtures/native"))
        .args([command, "--locked", "--offline"])
        .env(
            "CARGO_TARGET_DIR",
            root.join("../target/codegen-native-consumer"),
        )
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("BOOMERANG_WORKSPACE");
    if !features.is_empty() {
        command_line.arg("--features").arg(features.join(","));
    }
    command_line.output().unwrap()
}
fn text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
#[test]
fn normal_cargo_executes_native_exports_and_rejects_target_contract_failures() {
    let output = cargo("run", &[]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("two native instances executed"));
    for (feature, expected) in [
        ("missing-export", "could not find `absent`"),
        ("fingerprint-mismatch", "descriptor fingerprint mismatch"),
        ("abi-mismatch", "macro ABI mismatch"),
    ] {
        let output = cargo("build", &[feature]);
        assert!(!output.status.success(), "{feature} unexpectedly built");
        assert!(
            text(&output).contains(expected),
            "{feature}: {}",
            text(&output)
        );
    }
}
