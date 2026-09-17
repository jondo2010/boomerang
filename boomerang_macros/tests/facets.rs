use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::{Mutex, MutexGuard},
};

const MACRO_ABI_INPUT: &str = "BOOMERANG_PAYLOAD_INPUT_V1_MACRO_ABI";
const SENSOR_FINGERPRINT: &str = "adf86bcf69509f81e115866c31e02ab770c32b966644a3bff0328485d53b88f1";
const EMPTY_FINGERPRINT: &str = "0000000000000000000000000000000000000000000000000000000000000000";

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

struct FixtureLock {
    _guard: MutexGuard<'static, ()>,
    lockfile: PathBuf,
}

impl FixtureLock {
    fn acquire(fixture: &str) -> Self {
        Self {
            _guard: FIXTURE_LOCK
                .lock()
                .expect("fixture Cargo.lock synchronization must not be poisoned"),
            lockfile: fixture_path(fixture).join("Cargo.lock"),
        }
    }

    fn output(&self, mut command: Command) -> Output {
        command.output().expect("cargo command should start")
    }

    fn cleanup(&self) {
        match fs::remove_file(&self.lockfile) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!(
                "failed to remove fixture lockfile {}: {error}",
                self.lockfile.display()
            ),
        }
    }
}

impl Drop for FixtureLock {
    fn drop(&mut self) {
        self.cleanup();
    }
}

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

fn facet_flags(command: &mut Command, facets: &[&str]) {
    for (encoded, plain) in [
        ("CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS"),
        ("CARGO_ENCODED_RUSTDOCFLAGS", "RUSTDOCFLAGS"),
    ] {
        let mut flags: Vec<String> = match std::env::var(encoded) {
            Ok(flags) => flags
                .split('\x1f')
                .filter(|flag| !flag.is_empty())
                .map(str::to_owned)
                .collect(),
            Err(_) => std::env::var(plain)
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        };
        flags.extend(["-D".to_owned(), "warnings".to_owned()]);
        for facet in facets {
            flags.extend(["--cfg".to_owned(), format!("boomerang_facet=\"{facet}\"")]);
        }
        command.env(encoded, flags.join("\x1f")).env_remove(plain);
    }
}

fn command(fixture: &str, subcommand: &str, facets: &[&str], args: &[&str]) -> Command {
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
        .env(MACRO_ABI_INPUT, "3");
    facet_flags(&mut command, facets);
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

fn run(command: Command, fixture: &str) -> Output {
    FixtureLock::acquire(fixture).output(command)
}

fn failure(command: Command, fixture: &str) -> String {
    let output = run(command, fixture);
    assert!(!output.status.success(), "fixture unexpectedly succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn input_failure(key: &str, value: Option<&str>) -> String {
    let mut cargo = command("payload-launcher", "check", &["payload"], &[]);
    match value {
        Some(value) => cargo.env(key, value),
        None => cargo.env_remove(key),
    };
    failure(cargo, "payload-launcher")
}

fn action_failure(feature: &str) -> String {
    failure(
        command("descriptor-pass", "check", &[], &["--features", feature]),
        "descriptor-pass",
    )
}

fn cargo(fixture: &str, subcommand: &str, facets: &[&str], args: &[&str]) -> Result<(), String> {
    let output = run(command(fixture, subcommand, facets, args), fixture);

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn cargo_check(fixture: &str, facets: &[&str], args: &[&str]) -> Result<(), String> {
    cargo(fixture, "check", facets, args)
}

fn cargo_test(fixture: &str, facets: &[&str], args: &[&str]) -> Result<(), String> {
    cargo(fixture, "test", facets, args)
}

#[test]
fn descriptor_mode_excludes_reaction_payloads() {
    cargo_test("descriptor-pass", &["descriptor"], &[]).unwrap();
}

#[test]
fn descriptor_mode_rejects_unrecognized_closure_builder_code() {
    let stderr = cargo_check("descriptor-rejects-body", &["descriptor"], &[])
        .expect_err("descriptor mode should reject arbitrary builder code");
    assert!(
        stderr.contains("deployment descriptor requires reaction! syntax"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn hosted_mode_preserves_metadata_free_reactors() {
    cargo_test("metadata-free", &[], &[]).unwrap();
}

#[test]
fn hosted_mode_defers_duplicate_mode_validation_to_the_builder() {
    cargo_check(
        "descriptor-duplicate-reaction",
        &[],
        &["--features", "duplicate-mode"],
    )
    .unwrap();
}

#[test]
fn descriptor_mode_excludes_metadata_free_reactor_payloads() {
    cargo_check("metadata-free", &["descriptor"], &[]).unwrap();
}

#[test]
fn payload_mode_excludes_metadata_free_hosted_expansion() {
    cargo_check("metadata-free", &["payload"], &[]).unwrap();
}

#[test]
fn required_bindings_export_typed_payload_symbols() {
    cargo_test("descriptor-pass", &["payload"], &[]).unwrap();
}

#[test]
fn required_bindings_compile_in_a_separate_launcher() {
    let fixture = "payload-launcher";
    let fixture_lock = FixtureLock::acquire(fixture);
    let output = fixture_lock.output(command(
        fixture,
        "metadata",
        &["payload"],
        &["--format-version", "1"],
    ));
    assert!(output.status.success(), "{output:?}");
    let output = fixture_lock.output(command(fixture, "check", &["payload"], &["--locked"]));
    assert!(output.status.success(), "{output:?}");
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
            &["payload"],
            &["--no-default-features"],
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
        &["payload"],
        &["--features", "missing-state-init"],
    )
    .expect_err("custom payload state without state_init should fail");
    assert!(
        stderr.contains("payload mode requires `state_init = path` with `state = T`"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn required_bindings_reject_initializer_without_custom_state() {
    for (facet, facets) in [
        ("hosted", &[][..]),
        ("descriptor", &["descriptor"][..]),
        ("payload", &["payload"][..]),
    ] {
        let stderr = cargo_check("state-init-without-state", facets, &[])
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
        &["payload"],
        &["--features", "payload-lexical-relation"],
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
        &["payload"],
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
        &["payload"],
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
    let stderr = cargo_check("descriptor-pass", &["descriptor", "payload"], &[])
        .expect_err("reserved modes should conflict");
    assert!(
        stderr.contains("invalid boomerang_facet"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn reserved_modes_conflict_for_metadata_free_reactors() {
    let stderr = cargo_check("metadata-free", &["descriptor", "payload"], &[])
        .expect_err("reserved modes should conflict");
    assert!(
        stderr.contains("invalid boomerang_facet"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn reserved_modes_conflict_before_payload_descriptor_validation() {
    let stderr = cargo_check(
        "descriptor-duplicate-reaction",
        &["descriptor", "payload"],
        &["--features", "duplicate-mode"],
    )
    .expect_err("reserved modes should conflict before payload descriptor validation");
    assert!(
        stderr.contains("invalid boomerang_facet"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn feature_free_hosted_consumer_has_no_cfg_warnings() {
    cargo_check("feature-free", &[], &[]).unwrap();
}

#[test]
fn descriptor_mode_rejects_contract_version_overflow() {
    let stderr = cargo_check("descriptor-overflow", &["descriptor"], &[])
        .expect_err("overflowing contract version should fail");
    assert!(
        stderr.contains("contract_version must fit in u64"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_invalid_contract_text() {
    let stderr = cargo_check("descriptor-invalid-contract", &["descriptor"], &[])
        .expect_err("invalid contract text should fail");
    assert!(
        stderr.contains("contract must be non-empty, contain no control characters"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_multiple_reactors_per_module() {
    let stderr = cargo_check("descriptor-multiple", &["descriptor"], &[])
        .expect_err("multiple descriptor reactors should fail");
    assert!(
        stderr.contains("ONLY_ONE_DEPLOYMENT_REACTOR_PER_MODULE"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn descriptor_mode_rejects_duplicate_named_reactions() {
    let stderr = cargo_check("descriptor-duplicate-reaction", &["descriptor"], &[])
        .expect_err("duplicate named reactions should fail");
    assert!(
        stderr.contains("duplicate reaction name"),
        "unexpected compiler diagnostic:\n{stderr}"
    );
}

#[test]
fn deployment_facets_reject_duplicate_mode_names() {
    for facet in ["descriptor", "payload"] {
        let stderr = cargo_check(
            "descriptor-duplicate-reaction",
            &[facet],
            &["--features", "duplicate-mode"],
        )
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
    cargo_check("descriptor-duplicate-reaction", &[], &[]).unwrap();
}

#[test]
fn named_components_preserve_owned_modules_in_hosted_builds() {
    cargo_test("named-components", &[], &[]).unwrap();
}

fn named_input_key() -> String {
    let manifest_dir = fs::canonicalize(fixture_path("named-components")).unwrap();
    boomerang_runtime::binding::component_payload_fingerprint_compile_inputs_key(
        manifest_dir.to_str().unwrap(),
        "named.same",
        1,
        "Root",
    )
}

fn named_command(mode: &str, features: &str) -> Command {
    let mut cargo = command(
        "named-components",
        "test",
        &[mode],
        &["--features", features],
    );
    cargo.env(named_input_key(), concat!(
        "named_components::parent::keyboard=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f\n",
        "named_components::parent::alternate=1af19367a6fe4903a6f7a28a6a630513a160c6d9397e2777d31c78e05f0c12a5",
    ));
    cargo
}

#[test]
fn named_components_descriptor_excludes_all_helpers_and_reactions() {
    let output = run(
        named_command("descriptor", "sentinel,descriptor-assert"),
        "named-components",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_components_payload_uses_full_module_identity() {
    let output = run(
        named_command("payload", "payload-assert"),
        "named-components",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_components_reject_duplicate_and_invalid_modules() {
    let duplicate = failure(
        command(
            "named-components",
            "check",
            &[],
            &["--features", "duplicate"],
        ),
        "named-components",
    );
    assert!(duplicate.contains("defined multiple times"), "{duplicate}");
    let invalid = failure(
        command("named-components", "check", &[], &["--features", "invalid"]),
        "named-components",
    );
    assert!(invalid.contains("exactly one root #[reactor]"), "{invalid}");
}

#[test]
fn named_components_reject_invalid_and_conflicting_facet_selectors() {
    let invalid = failure(named_command("invalid", ""), "named-components");
    assert!(invalid.contains("invalid boomerang_facet"), "{invalid}");
    let mut both = named_command("payload", "");
    facet_flags(&mut both, &["descriptor", "payload"]);
    let conflict = failure(both, "named-components");
    assert!(conflict.contains("invalid boomerang_facet"), "{conflict}");
}

#[test]
fn named_components_reject_missing_or_invalid_payload_inputs() {
    let key = named_input_key();
    let mut missing = named_command("payload", "subset-assert");
    missing.env_remove(&key);
    let diagnostic = failure(missing, "named-components");
    assert!(
        diagnostic.contains("missing payload descriptor fingerprint compile input"),
        "{diagnostic}"
    );
    let mut invalid = named_command("payload", "subset-assert");
    invalid.env(&key, "named_components::parent::keyboard=ABC");
    let diagnostic = failure(invalid, "named-components");
    assert!(
        diagnostic.contains("exactly 64 lowercase hex digits"),
        "{diagnostic}"
    );
    let mut abi = named_command("payload", "subset-assert");
    abi.env(MACRO_ABI_INPUT, "4");
    let diagnostic = failure(abi, "named-components");
    assert!(
        diagnostic.contains("payload macro ABI mismatch"),
        "{diagnostic}"
    );
}

#[test]
fn named_component_helper_and_reaction_sentinels_remain_active_outside_descriptor() {
    let hosted = failure(
        command(
            "named-components",
            "check",
            &[],
            &["--features", "sentinel"],
        ),
        "named-components",
    );
    assert!(hosted.contains("helper sentinel reached"), "{hosted}");
    assert!(hosted.contains("reaction sentinel reached"), "{hosted}");
    let payload = failure(named_command("payload", "sentinel"), "named-components");
    assert!(payload.contains("helper sentinel reached"), "{payload}");
    assert!(payload.contains("reaction sentinel reached"), "{payload}");
}

#[test]
fn named_components_default_features_do_not_pull_builder_into_payload() {
    let cargo = command(
        "named-components",
        "tree",
        &["payload"],
        &["--edges", "normal", "--prefix", "none"],
    );
    let output = run(cargo, "named-components");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = String::from_utf8_lossy(&output.stdout);
    assert!(graph.contains("boomerang_runtime"), "{graph}");
    assert!(!graph.contains("boomerang_builder"), "{graph}");
}

#[test]
fn named_components_reject_root_attributes_instead_of_discarding_them() {
    for feature in ["cfg-root", "cfg-attr-root", "deprecated-root"] {
        let error = failure(
            command("named-components", "check", &[], &["--features", feature]),
            "named-components",
        );
        assert!(
            error.contains("unsupported component root attribute"),
            "{error}"
        );
        assert!(error.contains("component module"), "{error}");
    }
}

#[test]
fn named_components_payload_can_compile_unselected_components() {
    let mut cargo = named_command("payload", "");
    cargo.env_remove(named_input_key());
    cargo.env_remove(MACRO_ABI_INPUT);
    let output = run(cargo, "named-components");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_components_payload_can_select_one_component_from_an_owning_crate() {
    let mut cargo = named_command("payload", "subset-assert");
    cargo.env(named_input_key(), "named_components::parent::keyboard=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f");
    let output = run(cargo, "named-components");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_components_payload_normalizes_raw_module_path_segments() {
    let mut cargo = named_command("payload", "subset-assert");
    cargo.env(named_input_key(), "r#named_components::r#parent::r#keyboard=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f");
    let output = run(cargo, "named-components");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_components_payload_rejects_missing_entries_and_ambiguous_or_malformed_tables() {
    for (input, message) in [
        ("named_components::parent::alternate=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f", "missing payload descriptor fingerprint compile input"),
        ("not a record", "malformed component payload fingerprint input record"),
        (concat!("named_components::parent::keyboard=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f\n",
            "named_components::parent::r#keyboard=e0055db4be88cc8e0abadd0f4314cfa176d1b46c9b65730bef680f2676a08e6f"), "duplicate component payload fingerprint input record"),
    ] {
        let mut cargo = named_command("payload", "subset-assert");
        cargo.env(named_input_key(), input);
        let error = failure(cargo, "named-components");
        assert!(error.contains(message), "{error}");
        assert!(error.contains("E0080"), "selected-component validation must fail at compile time: {error}");
    }
}
