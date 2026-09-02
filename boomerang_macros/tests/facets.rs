use std::{fs, path::PathBuf, process::Command};

const MACRO_ABI_INPUT: &str = "BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI";
const SENSOR_FINGERPRINT: &str = "adf86bcf69509f81e115866c31e02ab770c32b966644a3bff0328485d53b88f1";
const EMPTY_FINGERPRINT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn fixture_path(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture)
}

fn fingerprint_input(contract: &str, reactor_root: &str) -> String {
    let manifest_dir =
        fs::canonicalize(fixture_path("descriptor-pass")).expect("fixture path should resolve");
    boomerang_runtime::binding::payload_fingerprint_compile_input_key(
        manifest_dir.to_str().expect("fixture path should be UTF-8"),
        contract,
        1,
        reactor_root,
    )
}

fn command(fixture: &str, subcommand: &str, args: &[&str]) -> Command {
    let manifest = fixture_path(fixture).join("Cargo.toml");
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("macros crate should be in the workspace")
        .join("target/facet-fixtures");
    let mut command = Command::new(env!("CARGO"));
    command
        .arg(subcommand)
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(manifest)
        .args(args)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("RUSTFLAGS", "-D warnings")
        .env(MACRO_ABI_INPUT, "3");
    for (contract, reactor_root, fingerprint) in [
        ("example.sensor", "Match", SENSOR_FINGERPRINT),
        ("example.custom", "Custom", EMPTY_FINGERPRINT),
        ("example.shaped", "Shaped", EMPTY_FINGERPRINT),
        ("example.lifetime", "Lifetime", EMPTY_FINGERPRINT),
        ("example.private-empty", "Empty", EMPTY_FINGERPRINT),
        ("example.actions", "Actions", EMPTY_FINGERPRINT),
    ] {
        command.env(fingerprint_input(contract, reactor_root), fingerprint);
    }
    command
}

fn run(mut command: Command, fixture: &str) -> std::process::Output {
    let output = command.output().expect("cargo command should start");
    let _ = fs::remove_file(fixture_path(fixture).join("Cargo.lock"));
    output
}

fn failure(command: Command, fixture: &str) -> String {
    let output = run(command, fixture);
    assert!(!output.status.success(), "fixture unexpectedly succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn input_failure(key: &str, value: Option<&str>) -> String {
    let mut cargo = command("payload-launcher", "check", &[]);
    match value {
        Some(value) => cargo.env(key, value),
        None => cargo.env_remove(key),
    };
    failure(cargo, "payload-launcher")
}

fn action_failure(feature: &str) -> String {
    failure(
        command("descriptor-pass", "check", &["--features", feature]),
        "descriptor-pass",
    )
}

fn cargo(fixture: &str, subcommand: &str, args: &[&str]) -> Result<(), String> {
    let output = run(command(fixture, subcommand, args), fixture);

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
    let fixture = "payload-launcher";
    let output = command(fixture, "metadata", &["--format-version", "1"])
        .output()
        .expect("cargo metadata should start");
    assert!(output.status.success(), "{output:?}");
    cargo_check("payload-launcher", &["--locked"]).unwrap();
}

#[test]
fn payload_compile_inputs_report_invalid_values() {
    let fingerprint = fingerprint_input("example.sensor", "Match");
    assert!(input_failure(&fingerprint, None).contains("missing payload descriptor fingerprint"));
    assert!(input_failure(
        &fingerprint,
        Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    )
    .contains("exactly 64 lowercase hex digits"));
    assert!(input_failure(MACRO_ABI_INPUT, Some("two")).contains("decimal u32"));
    assert!(input_failure(MACRO_ABI_INPUT, Some("1")).contains("expected 3, received 1"));
}

#[test]
fn action_declarations_reject_malformed_attributes_and_unrepresentable_delays() {
    assert!(action_failure("invalid-action-attribute").contains("min_delay = <duration>"));
    assert!(action_failure("invalid-action-duration").contains("invalid action minimum delay"));
    assert!(action_failure("action-delay-overflow").contains("nanosecond range"));
    assert!(action_failure("action-duration-unit-overflow").contains("unit conversion"));
}

#[test]
fn payload_only_dependency_graph_excludes_builder() {
    let output = run(
        command(
            "payload-launcher",
            "tree",
            &["--no-default-features", "--features", "__boomerang_payload"],
        ),
        "payload-launcher",
    );
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("boomerang_builder"), "{stdout}");
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
fn deployment_facets_reject_duplicate_mode_names() {
    for facet in ["__boomerang_descriptor", "__boomerang_payload"] {
        let features = format!("{facet} duplicate-mode");
        let stderr = cargo_check("descriptor-duplicate-reaction", &["--features", &features])
            .expect_err("duplicate mode names should fail in deployment facets");
        let normalized_stderr = stderr.replace('\\', "/");
        assert!(
            normalized_stderr.contains("duplicate mode name")
                && normalized_stderr.contains("src/lib.rs:22:13"),
            "unexpected {facet} diagnostic:\n{stderr}"
        );
    }
}

#[test]
fn hosted_mode_accepts_duplicate_named_reactions() {
    cargo_check("descriptor-duplicate-reaction", &[]).unwrap();
}
