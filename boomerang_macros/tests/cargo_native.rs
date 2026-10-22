use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::Mutex,
};
static FIXTURES: Mutex<()> = Mutex::new(());
fn cargo(fixture: &str, command: &str, args: &[&str]) -> Output {
    let _lock = FIXTURES.lock().unwrap_or_else(|error| error.into_inner());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = root.join("tests/fixtures").join(fixture);
    let mut cargo = Command::new(env!("CARGO"));
    cargo
        .arg(command)
        .args(["--quiet", "--offline", "--manifest-path"])
        .arg(fixture.join("Cargo.toml"))
        .args(args)
        .env(
            "CARGO_TARGET_DIR",
            root.join("../target/cargo-native-fixtures"),
        )
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTDOCFLAGS")
        .env_remove("CARGO_ENCODED_RUSTDOCFLAGS");
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .starts_with("BOOMERANG_PAYLOAD_INPUT_")
        {
            cargo.env_remove(key);
        }
    }
    let output = cargo.output().expect("Cargo starts");
    let _ = fs::remove_file(fixture.join("Cargo.lock"));
    output
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn explicit_host_metadata_and_target_payload_share_a_declaration_without_cli_inputs() {
    success(cargo(
        "cargo-native-explicit",
        "test",
        &["--features", "metadata,payload"],
    ));
}

#[test]
fn component_feature_matrix_is_additive_and_preserves_hosted() {
    success(cargo("cargo-native-component", "test", &[]));
    for features in ["metadata", "payload", "metadata,payload"] {
        success(cargo(
            "cargo-native-component",
            "test",
            &["--no-default-features", "--features", features],
        ));
    }
    success(cargo("cargo-native-unified", "test", &[]));
}
#[test]
fn public_features_reject_legacy_facet_mixing() {
    let _lock = FIXTURES.lock().unwrap_or_else(|error| error.into_inner());
    // A command-local flag keeps the Cargo feature and legacy selection collision explicit.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--manifest-path"])
        .arg(root.join("tests/fixtures/cargo-native-component/Cargo.toml"))
        .args(["--no-default-features", "--features", "metadata"])
        .env(
            "CARGO_TARGET_DIR",
            root.join("../target/cargo-native-conflict"),
        )
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            "--cfg\x1fboomerang_facet=\"descriptor\"",
        )
        .env_remove("RUSTFLAGS")
        .output()
        .unwrap();
    let _ = fs::remove_file(root.join("tests/fixtures/cargo-native-component/Cargo.lock"));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("cannot combine boomerang_facet with component metadata/payload features"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn fingerprint_and_macro_abi_mismatches_are_const_errors() {
    for (feature, expected) in [
        ("fingerprint-mismatch", "descriptor fingerprint mismatch"),
        ("abi-mismatch", "payload macro ABI mismatch"),
    ] {
        let output = cargo("cargo-native-explicit", "check", &["--features", feature]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn target_graph_excludes_host_builder_and_metadata_excludes_payload_helper() {
    let output = cargo(
        "cargo-native-component",
        "tree",
        &[
            "--no-default-features",
            "--features",
            "payload",
            "--edges",
            "normal,no-proc-macro",
        ],
    );
    assert!(output.status.success());
    let graph = String::from_utf8_lossy(&output.stdout);
    assert!(graph.contains("boomerang_runtime"), "{graph}");
    assert!(!graph.contains("boomerang_builder"), "{graph}");
    assert!(!graph.contains("cargo-boomerang"), "{graph}");
    let output = cargo(
        "cargo-native-component",
        "tree",
        &[
            "--no-default-features",
            "--features",
            "metadata",
            "--edges",
            "normal,no-proc-macro",
        ],
    );
    assert!(output.status.success());
    let graph = String::from_utf8_lossy(&output.stdout);
    assert!(!graph.contains("cargo-native-helper"), "{graph}");
}

#[test]
fn canonical_encoding_matches_the_independent_hand_checked_oracle() {
    for features in ["metadata", "payload", "metadata,payload"] {
        success(cargo(
            "cargo-native-compatibility",
            "test",
            &["--features", features],
        ));
    }
}

#[test]
fn metadata_does_not_activate_optional_target_implementation_dependencies() {
    success(cargo(
        "cargo-native-component",
        "check",
        &[
            "--no-default-features",
            "--features",
            "metadata,helper-sentinel",
        ],
    ));
    let output = cargo(
        "cargo-native-component",
        "check",
        &[
            "--no-default-features",
            "--features",
            "payload,helper-sentinel",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("target implementation helper sentinel reached"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = cargo("cargo-native-explicit", "tree", &["--edges", "build"]);
    assert!(output.status.success());
    let graph = String::from_utf8_lossy(&output.stdout);
    assert!(!graph.contains("cargo-native-helper"), "{graph}");
}
#[test]
fn cross_target_payload_keeps_builder_on_host_and_implementation_on_target() {
    // Cross-check when installed; ordinary test users need no additional Rust target.
    let Ok(targets) = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    else {
        eprintln!("rustup unavailable; cross-target sentinel check skipped");
        return;
    };
    if !String::from_utf8_lossy(&targets.stdout)
        .lines()
        .any(|target| target == "wasm32-unknown-unknown")
    {
        eprintln!("wasm32-unknown-unknown not installed; cross-target sentinel check skipped");
        return;
    }
    success(cargo(
        "cargo-native-explicit",
        "check",
        &[
            "--features",
            "payload,wasm-helper",
            "--target",
            "wasm32-unknown-unknown",
        ],
    ));
    let output = cargo(
        "cargo-native-explicit",
        "tree",
        &[
            "--features",
            "payload,wasm-helper",
            "--target",
            "wasm32-unknown-unknown",
            "--edges",
            "normal,no-proc-macro",
        ],
    );
    assert!(output.status.success());
    let graph = String::from_utf8_lossy(&output.stdout);
    assert!(graph.contains("cargo-native-wasm-helper"), "{graph}");
    assert!(!graph.contains("boomerang_builder"), "{graph}");
    assert!(!graph.contains("cargo-boomerang"), "{graph}");
}

#[test]
fn normal_consumer_rejects_an_incorrectly_exported_macro_abi() {
    // This keeps the normal consumer expectation, changing only the supplied actual ABI header.
    let output = cargo(
        "cargo-native-explicit",
        "check",
        &["--features", "incorrect-export-abi"],
    );
    assert!(
        !output.status.success(),
        "normal consumer accepted an incorrectly exported ABI 4"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("payload macro ABI mismatch"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
