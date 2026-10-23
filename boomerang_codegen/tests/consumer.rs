//! Runs ordinary Cargo against a checked-in application with a checked-in main.
mod cargo_support;

use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
fn cargo(command: &str, features: &[&str]) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut command_line = Command::new(env!("CARGO"));
    command_line
        .current_dir(root.join("tests/fixtures/native"))
        .args([command, "--locked", "--offline"])
        .env(
            "CARGO_TARGET_DIR",
            cargo_support::target_directory("codegen-native-consumer"),
        );
    cargo_support::clean_environment(&mut command_line);
    if command == "metadata" {
        command_line.args(["--format-version", "1"]);
    }
    if !features.is_empty() {
        command_line.arg("--features").arg(features.join(","));
    }
    command_line.output().unwrap()
}
/// Bound execution even if a regression leaves required input sources idle.
fn run() -> Output {
    let mut command = Command::new(
        cargo_support::target_directory("codegen-native-consumer")
            .join("debug")
            .join(format!(
                "codegen-native-consumer{}",
                std::env::consts::EXE_SUFFIX
            )),
    );
    cargo_support::clean_environment(&mut command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("native consumer exceeded deadline: {}", text(&output));
        }
        std::thread::park_timeout(Duration::from_millis(5));
    }
    child.wait_with_output().unwrap()
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
        let output = cargo("build", features);
        assert!(output.status.success(), "{}", text(&output));
        let output = run();
        assert!(output.status.success(), "{}", text(&output));
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("connected routing and initialized input"));
    }
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
