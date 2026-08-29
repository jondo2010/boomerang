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
        .env("RUSTFLAGS", "-D warnings")
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
    cargo_test("descriptor-pass", &["--features", "__boomerang_descriptor"]).unwrap();
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
fn hosted_mode_defers_duplicate_mode_validation_to_the_builder() {
    cargo_check(
        "descriptor-duplicate-reaction",
        &["--features", "duplicate-mode"],
    )
    .unwrap();
}

#[test]
fn descriptor_mode_excludes_metadata_free_reactor_payloads() {
    cargo_check("metadata-free", &["--features", "__boomerang_descriptor"]).unwrap();
}

#[test]
fn payload_mode_excludes_metadata_free_hosted_expansion() {
    cargo_check("metadata-free", &["--features", "__boomerang_payload"]).unwrap();
}

#[test]
fn required_bindings_export_typed_payload_symbols() {
    cargo_test("descriptor-pass", &["--features", "__boomerang_payload"]).unwrap();
}

#[test]
fn required_bindings_compile_in_a_separate_launcher() {
    cargo_check("payload-launcher", &[]).unwrap();
}

#[test]
fn required_bindings_reject_custom_state_without_initializer() {
    let stderr = cargo_check(
        "descriptor-pass",
        &["--features", "__boomerang_payload missing-state-init"],
    )
    .expect_err("custom payload state without state_init should fail");
    assert!(
        stderr.contains("payload mode requires `state_init = path` with `state = T`"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn required_bindings_reject_initializer_without_custom_state() {
    for (facet, args) in [
        ("hosted", &[][..]),
        ("descriptor", &["--features", "__boomerang_descriptor"][..]),
        ("payload", &["--features", "__boomerang_payload"][..]),
    ] {
        let stderr = cargo_check("state-init-without-state", args)
            .expect_err("state_init without custom state should fail in every facet");
        assert!(
            stderr.contains("`state_init` requires `state = T`"),
            "unexpected compiler diagnostic for {facet}:\n{stderr}"
        );
    }
}

#[test]
fn required_bindings_reject_lexical_payload_relations() {
    let stderr = cargo_check(
        "descriptor-pass",
        &["--features", "__boomerang_payload payload-lexical-relation"],
    )
    .expect_err("payload lexical relationships should fail");
    assert!(
        stderr.contains("payload mode supports only own ports, modes, and lifecycle relations"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn required_bindings_reject_macro_abi_mismatch_separately() {
    let stderr = cargo_check(
        "payload-launcher",
        &["--features", "binding-macro-abi-mismatch"],
    )
    .expect_err("payload macro ABI mismatch should fail");
    assert!(
        stderr.contains("macro ABI mismatch"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn payload_launcher_rejects_a_descriptor_fingerprint_mismatch() {
    let stderr = cargo_check(
        "payload-launcher",
        &["--features", "binding-fingerprint-mismatch"],
    )
    .expect_err("payload fingerprint mismatch should fail");
    assert!(
        stderr.contains("descriptor fingerprint mismatch"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
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

#[test]
fn reserved_modes_conflict_before_payload_descriptor_validation() {
    let stderr = cargo_check(
        "descriptor-duplicate-reaction",
        &[
            "--features",
            "__boomerang_descriptor __boomerang_payload duplicate-mode",
        ],
    )
    .expect_err("reserved modes should conflict before payload descriptor validation");
    assert!(
        stderr.contains("__boomerang_descriptor and __boomerang_payload cannot both be enabled"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn feature_free_hosted_consumer_has_no_cfg_warnings() {
    cargo_check("feature-free", &[]).unwrap();
}

#[test]
fn descriptor_mode_rejects_contract_version_overflow() {
    let stderr = cargo_check(
        "descriptor-overflow",
        &["--features", "__boomerang_descriptor"],
    )
    .expect_err("overflowing contract version should fail");
    assert!(
        stderr.contains("contract_version must fit in u64"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_invalid_contract_text() {
    let stderr = cargo_check(
        "descriptor-invalid-contract",
        &["--features", "__boomerang_descriptor"],
    )
    .expect_err("invalid contract text should fail");
    assert!(
        stderr.contains("contract must be non-empty, contain no control characters"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_multiple_reactors_per_module() {
    let stderr = cargo_check(
        "descriptor-multiple",
        &["--features", "__boomerang_descriptor"],
    )
    .expect_err("multiple descriptor reactors should fail");
    assert!(
        stderr.contains("ONLY_ONE_DEPLOYMENT_REACTOR_PER_MODULE"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_duplicate_named_reactions() {
    let stderr = cargo_check(
        "descriptor-duplicate-reaction",
        &["--features", "__boomerang_descriptor"],
    )
    .expect_err("duplicate named reactions should fail");
    assert!(
        stderr.contains("duplicate reaction name"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn hosted_mode_accepts_duplicate_named_reactions() {
    cargo_check("descriptor-duplicate-reaction", &[]).unwrap();
}
