#[path = "common/coordination.rs"]
mod fixture;
use boomerang_builder::compiler::*;
use boomerang_codegen::{coordination::*, *};

fn inputs() -> DeploymentInputs {
    fixture::inputs(1, 0, "serde-json", false)
}

fn selected(inputs: DeploymentInputs, member: &str) -> Compilation {
    let mut compilation = Compilation::new(
        inputs.topology,
        inputs.configuration,
        FederateId::new(member).unwrap(),
        TargetContext::new("x86_64-unknown-linux-gnu").unwrap(),
    );
    for binding in inputs.bindings {
        compilation
            .register(
                binding.implementation().clone(),
                binding.descriptor().clone(),
                PayloadExports::new(format!(
                    "crate::{}::payload",
                    binding.implementation().as_str().replace('-', "_")
                ))
                .unwrap(),
            )
            .unwrap();
        compilation
            .select(
                binding.component().clone(),
                binding.implementation().clone(),
            )
            .unwrap();
    }
    compilation
}

#[test]
fn independent_members_and_coordinator_share_definitions() {
    let output = inputs().compile_coordination().unwrap();
    let definitions =
        render::format_rust(render_definitions(&output.image, &inputs().topology).unwrap())
            .unwrap();
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
            coordination_fingerprint(&local.image, &inputs().topology).unwrap(),
            coordination_fingerprint(&output.image, &inputs().topology).unwrap()
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
    let base = inputs();
    let image = inputs().lower().unwrap();
    let identity = coordination_fingerprint(&image, &base.topology).unwrap();
    let mapping = mapping_fingerprint(&image).unwrap();
    let reordered = fixture::inputs(1, 0, "serde-json", true)
        .compile_coordination()
        .unwrap();
    assert_eq!(
        identity,
        coordination_fingerprint(&reordered.image, &base.topology).unwrap()
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
        let topology = changed.topology.clone();
        assert_ne!(
            identity,
            coordination_fingerprint(&changed.lower().unwrap(), &topology).unwrap()
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

#[test]
fn coordination_only_source_compiles_without_payload_dependencies() {
    let source = inputs().compile_coordination().unwrap().source;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let dependencies = [
        "boomerang_runtime",
        "boomerang_central_rti",
        "boomerang_federated",
        "boomerang_tinymap",
    ]
    .map(|name| {
        let alias = if name == "boomerang_tinymap" {
            "tinymap"
        } else {
            name
        };
        format!(
            "{alias} = {{ package = {name:?}, path = {:?} }}",
            root.join(name)
        )
    })
    .join("\n");
    std::fs::write(dir.path().join("Cargo.toml"), format!("[package]\nname = \"coordination-consumer\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n[dependencies]\n{dependencies}\n")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), format!("{source}\npub fn validate() {{ let _ = wire_contract(); let _ = COORDINATION_IDENTITY; let _ = wire_handshake(boomerang_runtime::image::FederateIndex::new(0)); }}")).unwrap();
    let output = std::process::Command::new(env!("CARGO"))
        .args(["check", "--offline"])
        .current_dir(dir.path())
        .env(
            "CARGO_TARGET_DIR",
            root.join("target/codegen-coordination-consumer"),
        )
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
