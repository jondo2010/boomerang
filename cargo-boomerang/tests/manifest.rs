use std::path::PathBuf;

use cargo_boomerang::{load_manifest, parse_manifest, CoordinationBackend};

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

[deployments.production.coordination]
backend = "central-rti"
"#
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

    let future = manifest.deployment("future-p2p").unwrap();
    assert_eq!(
        future.coordination.as_ref().unwrap().backend,
        CoordinationBackend::PeerToPeer
    );
    assert!(future.rti.is_none());
}

#[test]
fn central_rti_requires_an_rti_table() {
    let source = std::fs::read_to_string(fixture("invalid-rti")).unwrap();
    let error = parse_manifest(&source).unwrap_err();
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

[deployments.production.federates.right]
groups = ["right"]
runtime = "std"
"#;
    let cases = [
        (
            one_federate_with_coordination().replace("schema = 1", "schema = 2"),
            "unsupported Boomerang.toml schema 2; expected 1",
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
            "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n[deployments.production.federates.host]\ngroups = [\"host\"]\nruntime = \"std\"\n[deployments.production.rti]\ntarget = \"host\"".to_owned(),
            "deployments.production.rti is valid only with central-rti",
        ),
        (
            "schema = 1\n[topology]\npackage = \"topology\"\nentry = \"topology\"\n[deployments.production]\nfederates = {}".to_owned(),
            "deployments.production.federates must contain at least one Federate",
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
