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
    // Reuse each fixture group's existing target cache.
    let target = if fixture.starts_with("cargo-native-") {
        "../target/cargo-native-fixtures"
    } else {
        "../target/facet-fixtures"
    };
    let fixture = root.join("tests/fixtures").join(fixture);
    let mut cargo = Command::new(env!("CARGO"));
    cargo
        .arg(command)
        .args(["--quiet", "--offline", "--manifest-path"])
        .arg(fixture.join("Cargo.toml"))
        .args(args)
        .env("CARGO_TARGET_DIR", root.join(target))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTDOCFLAGS")
        .env_remove("CARGO_ENCODED_RUSTDOCFLAGS");
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

#[test]
fn direct_hosted_reactors_preserve_metadata_free_state_and_feature_free_consumers() {
    success(cargo("metadata-free", "test", &[]));
    success(cargo("feature-free", "check", &[]));
    success(cargo("descriptor-duplicate-reaction", "check", &[]));
    success(cargo(
        "descriptor-duplicate-reaction",
        "check",
        &["--features", "duplicate-mode"],
    ));
}

#[test]
fn native_payload_preserves_raw_names_reference_shapes_and_generic_initializers() {
    for features in ["metadata", "payload"] {
        success(cargo(
            "descriptor-pass",
            "test",
            &["--no-default-features", "--features", features],
        ));
    }
}

#[test]
fn native_components_keep_helpers_in_their_owning_modules() {
    success(cargo("named-components", "test", &[]));
    success(cargo(
        "named-components",
        "test",
        &["--features", "metadata,sentinel"],
    ));
    success(cargo(
        "named-components",
        "test",
        &["--no-default-features", "--features", "payload"],
    ));
    for features in ["sentinel", "payload,sentinel"] {
        let error = failure("named-components", &["--features", features]);
        assert!(error.contains("helper sentinel reached"), "{error}");
        assert!(error.contains("reaction sentinel reached"), "{error}");
    }
}

fn failure(fixture: &str, args: &[&str]) -> String {
    let output = cargo(fixture, "check", args);
    assert!(!output.status.success(), "{fixture} unexpectedly succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn component_diagnostics_preserve_distinct_syntax_and_initialization_errors() {
    for (fixture, features, expected) in [
        (
            "descriptor-rejects-body",
            "metadata",
            "deployment descriptor requires reaction! syntax",
        ),
        (
            "descriptor-overflow",
            "metadata",
            "contract_version must fit in u64",
        ),
        (
            "descriptor-invalid-contract",
            "metadata",
            "contract must be non-empty, contain no control characters",
        ),
        (
            "descriptor-duplicate-reaction",
            "metadata",
            "duplicate reaction name",
        ),
        (
            "descriptor-duplicate-reaction",
            "payload,duplicate-mode",
            "duplicate mode name",
        ),
        (
            "descriptor-pass",
            "payload,missing-state-init",
            "payload mode requires `state_init = path` with `state = T`",
        ),
        (
            "descriptor-pass",
            "payload,payload-lexical-relation",
            "payload mode supports only own ports, modes, and lifecycle relations",
        ),
        (
            "state-init-without-state",
            "payload",
            "`state_init` requires `state = T`",
        ),
        (
            "descriptor-pass",
            "invalid-action-attribute",
            "min_delay = <duration>",
        ),
        (
            "descriptor-pass",
            "invalid-action-duration",
            "invalid action minimum delay",
        ),
        (
            "descriptor-pass",
            "action-delay-overflow",
            "nanosecond range",
        ),
        (
            "descriptor-pass",
            "action-duration-unit-overflow",
            "unit conversion",
        ),
        ("named-components", "duplicate", "defined multiple times"),
        ("named-components", "invalid", "exactly one root #[reactor]"),
        (
            "named-components",
            "multiple",
            "exactly one root #[reactor]",
        ),
        (
            "named-components",
            "cfg-root",
            "unsupported component root attribute",
        ),
        (
            "named-components",
            "cfg-attr-root",
            "unsupported component root attribute",
        ),
        (
            "named-components",
            "deprecated-root",
            "unsupported component root attribute",
        ),
    ] {
        let error = failure(fixture, &["--features", features]);
        assert!(error.contains(expected), "{fixture} ({features}): {error}");
    }
}
