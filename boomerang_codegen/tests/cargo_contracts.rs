//! Cargo feature resolution and target compilation contracts.

use std::{
    path::PathBuf,
    process::{Command, Output},
};
/// Keep nested coverage objects beneath the coverage target for explicit collection.
fn target_directory(lane: &str) -> PathBuf {
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
fn clean_environment(command: &mut Command) {
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

fn cargo(command: &str, features: &[&str]) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut command_line = Command::new(env!("CARGO"));
    command_line
        .current_dir(root.join("tests/fixtures/native"))
        .args([command, "--locked"])
        .env(
            "CARGO_TARGET_DIR",
            target_directory("codegen-native-consumer"),
        );
    clean_environment(&mut command_line);
    if command == "metadata" {
        command_line.args(["--format-version", "1"]);
    }
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
fn cargo_resolves_host_and_target_features_and_rejects_invalid_exports() {
    for features in [&[][..], &["source-input"][..]] {
        let metadata = cargo("metadata", features);
        assert!(metadata.status.success(), "{}", text(&metadata));
        let graph: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
        let runtime = graph["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == "boomerang_runtime")
            .unwrap();
        let node = graph["resolve"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == runtime["id"])
            .unwrap();
        let hosted = node["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .filter(|feature| feature.starts_with("hosted-") || *feature == "external-clock")
            .collect::<Vec<_>>();
        let expected = if features.is_empty() {
            vec![]
        } else {
            vec!["hosted-source-input"]
        };
        assert_eq!(hosted, expected, "{node}");
        let output = cargo("check", features);
        assert!(output.status.success(), "{}", text(&output));
    }
    for (feature, expected) in [
        ("missing-export", "could not find `absent`"),
        ("fingerprint-mismatch", "descriptor fingerprint mismatch"),
        ("abi-mismatch", "macro ABI mismatch"),
    ] {
        let output = cargo("check", &[feature]);
        assert!(!output.status.success(), "{feature} unexpectedly built");
        assert!(
            text(&output).contains(expected),
            "{feature}: {}",
            text(&output)
        );
    }
}
