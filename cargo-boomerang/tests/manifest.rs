use std::path::PathBuf;

use cargo_boomerang::{
    load_manifest, parse_manifest, CoordinationBackend, ExecutionPolicy, RecoveryPolicy,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifest")
        .join(name)
        .join("Boomerang.toml")
}

fn one_federate_with_coordination() -> &'static str {
    r#"
schema = 1

[topology]
package = "vehicle-topology"
entry = "vehicle::topology"

[deployments.production.federates.host]
groups = ["vehicle"]
runtime = "std"
recovery = "fail-stop"

[deployments.production.coordination]
backend = "central-rti"
"#
}

fn one_federate_without_coordination() -> &'static str {
    r#"
schema = 1

[topology]
package = "vehicle-topology"
entry = "vehicle::topology"

[deployments.production.federates.host]
groups = ["vehicle"]
runtime = "std"
recovery = "fail-stop"
"#
}

#[test]
fn tracing_backend_is_a_validated_build_choice() {
    use cargo_boomerang::TracingBackend;
    let source = one_federate_without_coordination();
    assert_eq!(
        parse_manifest(source)
            .unwrap()
            .deployment("production")
            .unwrap()
            .tracing,
        TracingBackend::Hosted,
    );
    for (name, expected) in [
        ("off", TracingBackend::Off),
        ("bounded", TracingBackend::Bounded),
        ("hosted", TracingBackend::Hosted),
    ] {
        let configured = format!("{source}\n[deployments.production]\ntracing = \"{name}\"\n");
        assert_eq!(
            parse_manifest(&configured)
                .unwrap()
                .deployment("production")
                .unwrap()
                .tracing,
            expected
        );
    }
    let invalid = format!("{source}\n[deployments.production]\ntracing = \"automatic\"\n");
    let error = parse_manifest(&invalid).unwrap_err().to_string();
    assert!(error.contains("deployments.production.tracing"), "{error}");
}

#[test]
fn hosted_telemetry_is_an_explicit_build_choice() {
    let source = format!(
        "{}\n[deployments.production]\ntelemetry = \"hosted\"\n",
        one_federate_without_coordination(),
    );
    assert!(parse_manifest(&source).is_ok());

    let invalid = source.replace("hosted", "automatic");
    let error = parse_manifest(&invalid).unwrap_err().to_string();
    assert!(
        error.contains("deployments.production.telemetry"),
        "{error}"
    );
}

#[test]
fn bounded_tracing_tables_validate_before_building() {
    let source = format!(
        "{}\n[deployments.production]\ntracing = \"bounded\"\n\
         [deployments.production.bounded-tracing]\nrecords = 0\nbytes = 2048\n\
         [deployments.production.federates.host.bounded-tracing]\nrecords = 2\n",
        one_federate_without_coordination(),
    );
    assert!(parse_manifest(&source).is_ok());
    for (field, value) in [
        ("fields", "0"),
        ("bytes", "0"),
        ("spans", "0"),
        ("spans", "4294967295"),
        ("span-fields", "0"),
        ("span-bytes", "0"),
        ("depth", "0"),
        ("producers", "0"),
        ("records", "-1"),
        ("records", "4294967296"),
        ("recordz", "1"),
    ] {
        let invalid = if field == "records" {
            source.replace("records = 2", &format!("records = {value}"))
        } else {
            format!("{source}{field} = {value}\n")
        };
        let error = parse_manifest(&invalid).unwrap_err().to_string();
        assert!(error.contains("bounded-tracing"), "{field}: {error}");
    }
    for backend in ["off", "hosted"] {
        let invalid = source.replace("tracing = \"bounded\"", &format!("tracing = {backend:?}"));
        assert!(parse_manifest(&invalid)
            .unwrap_err()
            .to_string()
            .contains("bounded-tracing"));
    }
    let overflow = source.replace("records = 0", "records = 4294967295\nfields = 4294967295");
    assert!(parse_manifest(&overflow).is_err());
}

#[test]
fn bounded_tracing_overrides_inherit_each_field() {
    let source = format!(
        "{}\n[deployments.production]\ntracing = \"bounded\"\n\
         [deployments.production.bounded-tracing]\nrecords = 9\nfields = 10\nbytes = 2048\n\
         spans = 11\nspan-fields = 12\nspan-bytes = 512\nproducers = 13\ndepth = 14\n\
         [deployments.production.federates.host.bounded-tracing]\nrecords = 0\nspan-fields = 15\n",
        one_federate_without_coordination(),
    );
    let manifest = parse_manifest(&source).unwrap();
    let deployment = manifest.deployment("production").unwrap();
    let limits = deployment
        .bounded_tracing_limits(deployment.federates["host"].bounded_tracing.as_ref())
        .unwrap();
    assert_eq!(
        serde_json::to_value(limits).unwrap(),
        serde_json::json!({
            "records": 0, "fields": 10, "bytes": 2048, "spans": 11,
            "span_fields": 15, "span_bytes": 512, "producers": 13, "depth": 14,
            "level": "debug", "targets": ["boomerang::coordination"],
        })
    );
}

#[test]
fn bounded_trace_filters_resolve_and_validate_at_the_manifest_boundary() {
    let source = format!(
        "{}\n[deployments.production]\ntracing = \"bounded\"\n\
         [deployments.production.bounded-tracing]\nlevel = \"trace\"\n\
         targets = [\"boomerang::runtime\", \"boomerang::coordination\"]\n\
         [deployments.production.federates.host.bounded-tracing]\nlevel = \"info\"\n",
        one_federate_without_coordination(),
    );
    let resolve = |source: &str| {
        let manifest = parse_manifest(source).unwrap();
        let deployment = manifest.deployment("production").unwrap();
        let limits = deployment
            .bounded_tracing_limits(deployment.federates["host"].bounded_tracing.as_ref())
            .unwrap();
        // Configuration passes directly to the native subscriber without conversion.
        let config = tracing_bounded::Config {
            level: limits.level,
            ..Default::default()
        };
        let encoded = serde_json::to_value(&limits).unwrap();
        let decoded: cargo_boomerang::BoundedTracingLimits =
            serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.level, config.level);
        assert_eq!(decoded, limits);
        encoded
    };
    let resolved = resolve(&source);
    assert_eq!(resolved["level"], "info");
    assert_eq!(
        resolved["targets"],
        serde_json::json!(["boomerang::runtime", "boomerang::coordination"])
    );
    assert_eq!(
        resolve(&format!("{source}targets = []\n"))["targets"],
        serde_json::json!([])
    );
    assert_eq!(
        resolve(&format!("{source}targets = [\"application\"]\n"))["targets"],
        serde_json::json!(["application"])
    );
    for level in ["off", "error", "warn", "info", "debug", "trace"] {
        assert_eq!(
            resolve(&source.replace("level = \"info\"", &format!("level = {level:?}")))["level"],
            level
        );
    }
    for invalid in [
        "", "0", "1", "5", "6", "DEBUG", " debug", "debug ", "verbose",
    ] {
        let invalid_source = source.replace("level = \"info\"", &format!("level = {invalid:?}"));
        assert!(parse_manifest(&invalid_source)
            .unwrap_err()
            .to_string()
            .contains("bounded-tracing"));
        let mut invalid_report = resolved.clone();
        invalid_report["level"] = serde_json::json!(invalid);
        assert!(
            serde_json::from_value::<cargo_boomerang::BoundedTracingLimits>(invalid_report)
                .is_err()
        );
    }
    for invalid in [
        source.replace("level = \"info\"", "level = \"verbose\""),
        format!("{source}targets = [\"\"]\n"),
        format!("{source}targets = [\" boomerang::runtime\"]\n"),
    ] {
        assert!(parse_manifest(&invalid)
            .unwrap_err()
            .to_string()
            .contains("bounded-tracing"));
    }
}

#[test]
fn bindings_select_named_component_modules_and_reject_expressions() {
    for entry in ["keyboard", "input::keyboard", "input::r#type"] {
        let source = format!(
            "{}\n[deployments.production.bindings.keys]\npackage = \"components\"\ncomponent = {entry:?}\n",
            one_federate_without_coordination(),
        );
        let manifest = cargo_boomerang::parse_manifest(&source).unwrap();
        assert_eq!(
            manifest.deployment("production").unwrap().bindings["keys"]
                .component
                .as_deref(),
            Some(entry)
        );
    }
    for entry in [
        "",
        "::keyboard",
        "crate::keyboard",
        "super::keyboard",
        "self::keyboard",
        "keyboard::<u8>",
        "keyboard()",
        "keyboard; panic!()",
    ] {
        let source = format!(
            "{}\n[deployments.production.bindings.keys]\npackage = \"components\"\ncomponent = {entry:?}\n",
            one_federate_without_coordination(),
        );
        assert!(
            cargo_boomerang::parse_manifest(&source).is_err(),
            "accepted {entry}"
        );
    }
}

#[test]
fn valid_manifest_preserves_the_complete_schema() {
    let manifest = load_manifest(fixture("valid")).unwrap();
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.topology.package, "vehicle-topology");
    assert_eq!(manifest.topology.entry, "vehicle::topology");

    let production = manifest.deployment("production").unwrap();
    let sensor = &production.bindings["vehicle/sensor"];
    assert_eq!(sensor.package, "sensor-stm32");
    assert_eq!(sensor.features, ["board-a"]);

    let edge = &production.federates["sensor-edge"];
    assert_eq!(edge.groups, ["sensor"]);
    assert_eq!(edge.target.as_deref(), Some("thumbv7em-none-eabihf"));
    assert_eq!(edge.toolchain.as_deref(), Some("nightly-2026-08-01"));
    assert_eq!(edge.profile.as_deref(), Some("release"));
    assert_eq!(edge.runtime, "bare-metal");
    assert_eq!(edge.recovery, RecoveryPolicy::FailStop);
    assert_eq!(edge.target_json.as_deref(), Some("targets/sensor.json"));
    assert_eq!(
        edge.cargo_config.as_deref(),
        Some(".cargo/sensor-target.toml")
    );

    assert_eq!(
        production.coordination.as_ref().unwrap().backend,
        CoordinationBackend::CentralRti
    );
    assert_eq!(
        production.rti.as_ref().unwrap().profile.as_deref(),
        Some("release")
    );
    assert_eq!(
        production.rti.as_ref().unwrap().target,
        "aarch64-unknown-linux-gnu"
    );
    let boundary = &production.boundaries["controller-to-sensor"];
    assert_eq!(boundary.flow, "sensor-control");
    assert_eq!(boundary.physical_input.as_deref(), Some("plant/sensor"));
    assert_eq!(boundary.physical_output.as_deref(), Some("plant/actuator"));
    assert_eq!(boundary.security_policy.as_str(), "none");

    let future = manifest.deployment("future-p2p").unwrap();
    assert_eq!(
        future.coordination.as_ref().unwrap().backend,
        CoordinationBackend::PeerToPeer
    );
    assert!(future.rti.is_none());
}

#[test]
fn manifest_policy_vocabulary_rejects_unknown_names_during_parsing() {
    let source = one_federate_without_coordination().replace(
        "recovery = \"fail-stop\"",
        "recovery = \"mystery-recovery\"",
    );
    let error = parse_manifest(&source).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("deployments.production.federates.host.recovery"),
        "{error}"
    );
}

#[test]
fn deployment_execution_policy_is_nameable_from_the_public_crate_root() {
    let policy: ExecutionPolicy = Default::default();
    assert!(!policy.fast_forward);
    assert!(!policy.keep_alive);
    assert_eq!(policy.logical_horizon, None);
}

#[test]
fn central_rti_requires_an_rti_table() {
    let mut source: toml::Value =
        toml::from_str(&std::fs::read_to_string(fixture("valid")).unwrap()).unwrap();
    source["deployments"]["production"]
        .as_table_mut()
        .unwrap()
        .remove("rti");
    let error = parse_manifest(&toml::to_string(&source).unwrap()).unwrap_err();
    assert!(error
        .to_string()
        .contains("central-rti requires deployments.production.rti"));
}

#[test]
fn one_federate_rejects_distributed_coordination() {
    let error = parse_manifest(one_federate_with_coordination()).unwrap_err();
    assert!(error
        .to_string()
        .contains("coordination is absent for one-Federate deployments"));
}

#[test]
fn unknown_fields_report_their_toml_path() {
    let cases = [
        ("unexpected", "topology.unexpected"),
        ("topology", "topology.topology"),
    ];

    for (field, expected_path) in cases {
        let source = one_federate_with_coordination().replace(
            "entry = \"vehicle::topology\"",
            &format!("entry = \"vehicle::topology\"\n{field} = true"),
        );
        let error = parse_manifest(&source).unwrap_err();
        assert!(
            error.to_string().contains(&format!("at {expected_path}:")),
            "{error}"
        );
    }
}

#[test]
fn remaining_manifest_consistency_rules_share_one_validation_boundary() {
    let two_federates = r#"
[deployments.production.federates.left]
groups = ["left"]
runtime = "std"
recovery = "fail-stop"

[deployments.production.federates.right]
groups = ["right"]
runtime = "std"
recovery = "fail-stop"
"#;
    let cases = [
        (
            one_federate_with_coordination().replace("schema = 1", "schema = 3"),
            "unsupported Boomerang.toml schema 3; expected 1",
        ),
        (
            format!(
                "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n{two_federates}"
            ),
            "deployments.production.coordination is required for multi-Federate deployments",
        ),
        (
            format!(
                "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n{two_federates}\n[deployments.production.coordination]\nbackend = \"peer-to-peer\"\n[deployments.production.rti]\ntarget = \"host\""
            ),
            "deployments.production.rti is not valid with peer-to-peer",
        ),
        (
            "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n[deployments.production.federates.host]\ngroups = [\"host\"]\nruntime = \"std\"\nrecovery = \"fail-stop\"\n[deployments.production.rti]\ntarget = \"host\"".to_owned(),
            "deployments.production.rti is valid only with central-rti",
        ),
        (
            "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n[deployments.production]\nfederates = {}".to_owned(),
            "deployments.production.federates must contain at least one Federate",
        ),
        (
            "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n[deployments.\"../escape\".federates.host]\ngroups = [\"host\"]\nruntime = \"std\"\nrecovery = \"fail-stop\"".to_owned(),
            "invalid deployment ../escape: deployment names must be non-empty and contain only ASCII letters, digits, '-', '_', or '.'; '.' and '..' are reserved",
        ),
    ];

    for (source, expected) in cases {
        let error = parse_manifest(&source).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}

#[test]
fn schema_one_accepts_deployment_execution_policy_and_rejects_schema_two() {
    let schema_one = one_federate_without_coordination().replace(
        "runtime = \"std\"\nrecovery = \"fail-stop\"",
        "runtime = \"std\"\nrecovery = \"fail-stop\"\n\n[deployments.production.execution]\nfast-forward = true",
    );
    parse_manifest(&schema_one).unwrap();

    let schema_two_default =
        one_federate_without_coordination().replace("schema = 1", "schema = 2");
    let error = parse_manifest(&schema_two_default).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported Boomerang.toml schema 2; expected 1"),
        "{error}"
    );
}

#[test]
fn execution_policy_rejects_invalid_logical_horizons_at_the_manifest_boundary() {
    for duration in ["not-a-duration", "-1ns", "0.1ns", "18446744073709551616ns"] {
        let source = one_federate_without_coordination()
            .replace(
                "runtime = \"std\"\nrecovery = \"fail-stop\"",
                &format!("runtime = \"std\"\nrecovery = \"fail-stop\"\n\n[deployments.production.execution]\nlogical-horizon = {duration:?}"),
            );
        let error = parse_manifest(&source).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("deployments.production.execution.logical-horizon"),
            "duration {duration:?} produced {error}"
        );
    }

    let zero = one_federate_without_coordination().replace(
        "runtime = \"std\"\nrecovery = \"fail-stop\"",
        "runtime = \"std\"\nrecovery = \"fail-stop\"\n\n[deployments.production.execution]\nlogical-horizon = \"0ns\"",
    );
    parse_manifest(&zero).unwrap();
}
