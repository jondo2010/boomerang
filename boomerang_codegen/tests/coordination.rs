#[path = "common/coordination.rs"]
mod fixture;
use boomerang_builder::compiler::*;
use boomerang_codegen::{coordination::*, *};

fn inputs() -> DeploymentInputs {
    fixture::inputs(1, 0, "serde-json", false)
}

use fixture::selected;

#[test]
fn independent_members_and_coordinator_share_definitions() {
    let output = inputs().compile_coordination().unwrap();
    let definitions = render::format_rust(render_definitions(&output.image).unwrap()).unwrap();
    assert_eq!(output.source, definitions);
    for name in [
        "COORDINATION_IMAGE",
        "COORDINATION_MEMBERS",
        "COORDINATION_IDENTITY",
        "WIRE_COORDINATION_FINGERPRINT",
        "WIRE_MAPPING",
        "WirePayloadCodec",
        "wire_contract",
        "wire_handshake",
    ] {
        assert!(output.source.contains(name), "{name}");
    }
    for absent in [
        "generated_bindings",
        "FEDERATE_VIEW",
        "fn main",
        "execute_owned",
        "crate::impl_",
    ] {
        assert!(!output.source.contains(absent), "{absent}");
    }
    for member in ["a", "b"] {
        let local = selected(inputs(), member).compile().unwrap();
        syn::parse_file(&local.source).unwrap();
        assert!(local.source.contains(&output.source));
        assert_eq!(
            mapping_fingerprint(&local.image).unwrap(),
            mapping_fingerprint(&output.image).unwrap()
        );
        assert_eq!(
            coordination_fingerprint(&local.image).unwrap(),
            coordination_fingerprint(&output.image).unwrap()
        );
        let remote = if member == "a" { "impl_b" } else { "impl_a" };
        assert!(!local.source.contains(&format!("crate::{remote}")));
        assert!(local.source.contains("boomerang_central_rti"));
        assert_eq!(
            local.source,
            selected(fixture::inputs(1, 0, "serde-json", true), member)
                .compile()
                .unwrap()
                .source
        );
    }
}

#[test]
fn coordination_is_canonical_and_tracks_contract_delay_and_codec() {
    let image = inputs().lower().unwrap();
    let identity = coordination_fingerprint(&image).unwrap();
    assert_eq!(
        identity.to_hex().as_str(),
        "a87f8224897d1828de6c0c772c590a23655480c547570c887f9bbb91a72eacef"
    );
    let mapping = mapping_fingerprint(&image).unwrap();
    let reordered = fixture::inputs(1, 0, "serde-json", true)
        .compile_coordination()
        .unwrap();
    assert_eq!(
        identity,
        coordination_fingerprint(&reordered.image).unwrap()
    );
    assert_eq!(mapping, mapping_fingerprint(&reordered.image).unwrap());
    assert_eq!(
        inputs().compile_coordination().unwrap().source,
        reordered.source
    );
    for (version, delay, codec) in [
        (2, 0, "serde-json"),
        (1, 1, "serde-json"),
        (1, 0, "postcard-v1"),
    ] {
        let changed = fixture::inputs(version, delay, codec, false);
        assert_ne!(
            identity,
            coordination_fingerprint(&changed.lower().unwrap()).unwrap()
        );
    }
}

#[test]
fn coordinator_rejects_local_deployment_and_members_retain_target_checks() {
    let mut local = inputs();
    let member = FederateId::new("a").unwrap();
    local.configuration.coordination = CoordinationSelection::Local;
    local.configuration.boundaries.clear();
    local.configuration.federates.retain(|f| f.id() == &member);
    for placement in &mut local.configuration.placements {
        *placement = PlacementAssignment::new(placement.placement_group().clone(), member.clone());
    }
    assert_eq!(
        local.compile_coordination().unwrap_err().diagnostic.stage,
        Stage::Render
    );
    let mut wrong = inputs();
    wrong.configuration.federates[0] = FederateConfig::new(
        member,
        TargetTriple::new("aarch64-unknown-linux-gnu").unwrap(),
        RuntimeBackendId::new("std").unwrap(),
        boomerang_runtime::image::RecoveryPolicy::FailStop,
    );
    assert_eq!(
        selected(wrong, "a").compile().unwrap_err().diagnostic.code,
        DiagnosticCode::TargetMismatch
    );
}

fn assert_unsupported(inputs: DeploymentInputs, coordinator: bool) {
    let error = if coordinator {
        inputs.compile_coordination().unwrap_err()
    } else {
        selected(inputs, "a").compile().unwrap_err()
    };
    assert_eq!(error.diagnostic.stage, Stage::Render);
    assert_eq!(error.diagnostic.code, DiagnosticCode::Render);
    assert_eq!(
        error.diagnostic.message,
        "unsupported generated central-rti boundary configuration for 'a-to-b'"
    );
}

#[test]
fn coordinator_rejects_unsupported_codec() {
    assert_unsupported(fixture::with_capabilities("unsupported-codec", "tcp"), true);
}

#[test]
fn coordinator_rejects_unsupported_transport() {
    assert_unsupported(
        fixture::with_capabilities("serde-json", "unsupported-transport"),
        true,
    );
}

#[test]
fn member_rejects_unsupported_codec() {
    assert_unsupported(
        fixture::with_capabilities("unsupported-codec", "tcp"),
        false,
    );
}

#[test]
fn member_rejects_unsupported_transport() {
    assert_unsupported(
        fixture::with_capabilities("serde-json", "unsupported-transport"),
        false,
    );
}
