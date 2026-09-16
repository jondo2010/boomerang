//! Behavioral identity tests using the real resolved-deployment lowering pipeline.

use super::*;
use boomerang_builder::{
    compiler::*, ComponentDescriptor, DescriptorBound, DescriptorBounds, PortSlot, PortSlotId,
    ReactorSlot, ReactorSlotId, COMPONENT_DESCRIPTOR_MACRO_ABI,
};
use boomerang_runtime::image::{
    BoundaryFailurePolicy, CodecPolicy, RecoveryPolicy, SecurityPolicy, TimingPolicy,
    TransportPolicy,
};

/// Constructs two real scheduler images with one shared cross-Federate contract.
fn fixture(
    version: u64,
    state_bytes: u64,
    delay: u64,
    implementation: &str,
) -> (
    ApplicationTopology,
    Vec<DescriptorDriverBinding>,
    OwnedCompiledDeployment,
) {
    fixture_profile(
        version,
        state_bytes,
        delay,
        implementation,
        "serde-json",
        false,
        8,
    )
}

/// Varies transport capability identity and construction order without a second lowering path.
fn fixture_profile(
    version: u64,
    state_bytes: u64,
    delay: u64,
    implementation: &str,
    codec: &str,
    reverse: bool,
    queue_capacity: u64,
) -> (
    ApplicationTopology,
    Vec<DescriptorDriverBinding>,
    OwnedCompiledDeployment,
) {
    let mut topology = ApplicationTopologyBuilder::new("app").unwrap();
    let mut bindings = Vec::new();
    let mut placements = Vec::new();
    let mut federates = Vec::new();
    let mut members = [("a", PortDirection::Output), ("b", PortDirection::Input)];
    if reverse {
        members.reverse();
    }
    for (name, direction) in members {
        let version = if name == "a" { version } else { 1 };
        let id = ComponentInstanceId::new(name).unwrap();
        let root = ReactorId::new(name).unwrap();
        let enclave = StableEnclaveId::new(name).unwrap();
        let group = PlacementGroupId::new(name).unwrap();
        topology
            .add_component(ComponentInstance::new(name, name, version).unwrap())
            .unwrap();
        topology.add_placement_group(group.clone(), None).unwrap();
        topology
            .add_reactor(Reactor::new(
                root.clone(),
                id.clone(),
                None,
                None,
                enclave.clone(),
                Some(group.clone()),
                None,
            ))
            .unwrap();
        topology.add_enclave(enclave, root.clone()).unwrap();
        topology
            .add_port(
                PortId::new(format!("{name}/port")).unwrap(),
                root,
                direction,
                None,
                0,
                None,
            )
            .unwrap();
        let descriptor = ComponentDescriptor::try_new(
            ContractId::new(name).unwrap(),
            version,
            COMPONENT_DESCRIPTOR_MACRO_ABI,
            vec![ReactorSlot {
                id: ReactorSlotId::new("Root").unwrap(),
                parent: None,
            }],
            vec![PortSlot {
                id: PortSlotId::new("Root/port").unwrap(),
                reactor: ReactorSlotId::new("Root").unwrap(),
                direction,
            }],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            DescriptorBounds {
                queue_capacity: DescriptorBound::Known(queue_capacity),
                payload_bytes: DescriptorBound::Known(512),
                state_bytes: DescriptorBound::Known(if name == "a" { state_bytes } else { 64 }),
                scratch_bytes: DescriptorBound::Known(64),
            },
        )
        .unwrap();
        bindings.push(
            DescriptorDriverBinding::new(
                name,
                if name == "a" {
                    implementation
                } else {
                    "impl-b"
                },
                descriptor,
            )
            .unwrap(),
        );
        placements.push(PlacementAssignment::new(
            group,
            FederateId::new(name).unwrap(),
        ));
        federates.push(FederateConfig::new(
            FederateId::new(name).unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("std").unwrap(),
            RecoveryPolicy::FailStop,
        ));
    }
    topology
        .add_connection(
            BoundaryId::new("a-to-b").unwrap(),
            PortId::new("a/port").unwrap(),
            PortId::new("b/port").unwrap(),
            ConnectionSemantics::Logical {
                after: (delay != 0)
                    .then_some(boomerang_runtime::Duration::nanoseconds_i128(delay.into())),
            },
        )
        .unwrap();
    let topology = topology.finish().unwrap();
    let compiled = ResolvedDeployment::new(
        topology.clone(),
        bindings.iter().map(|b| {
            ImplementationBinding::new(
                b.component().clone(),
                b.implementation().clone(),
                b.descriptor().clone(),
            )
        }),
        placements,
        federates,
        CoordinationSelection::Distributed {
            backend: CoordinationBackend::CentralRti,
        },
        [BoundaryBinding::new(
            BoundaryId::new("a-to-b").unwrap(),
            FlowId::new("flow").unwrap(),
            PhysicalBoundaryMetadata::new(None, None),
            CodecCapabilityId::new(codec).unwrap(),
            TransportCapabilityId::new("tcp").unwrap(),
            BoundaryPolicies::new(
                BoundaryFailurePolicy::PropagateStop,
                TransportPolicy::ReliableOrderedFramed,
                CodecPolicy::CanonicalBounded,
                TimingPolicy::BestEffort,
                SecurityPolicy::None,
            ),
        )],
    )
    .unwrap()
    .lower()
    .unwrap();
    (topology, bindings, compiled)
}

/// Local storage layout must not reject an otherwise compatible peer.
#[test]
fn coordination_ignores_private_descriptor_storage() {
    let (topology, _bindings, compiled) = fixture(1, 64, 0, "impl-a");
    let baseline = coordination(&compiled, &topology).unwrap();
    let (topology, _bindings, compiled) = fixture(1, 128, 0, "impl-a");
    assert_eq!(baseline, coordination(&compiled, &topology).unwrap());
}

/// Declared schema versions and boundary delays change coordination, private bindings do not.
#[test]
fn coordination_tracks_shared_semantics() {
    let (topology, _, compiled) = fixture(1, 64, 0, "impl-a");
    let base = coordination(&compiled, &topology).unwrap();
    for (version, delay) in [(2, 0), (1, 1)] {
        let (topology, _, compiled) = fixture(version, 64, delay, "impl-a");
        assert_ne!(base, coordination(&compiled, &topology).unwrap());
    }
    let (topology, _, compiled) = fixture(1, 64, 0, "replacement-a");
    assert_eq!(base, coordination(&compiled, &topology).unwrap());
}

/// A local layout or binding change affects only its owning Federate image.
#[test]
fn federate_images_isolate_local_changes() {
    let (_, bindings, compiled) = fixture(1, 64, 0, "impl-a");
    let images = |compiled: &OwnedCompiledDeployment, bindings: &[DescriptorDriverBinding]| {
        compiled
            .federates()
            .iter()
            .map(|(key, _)| {
                federate_image(&compiled.federate_slice(key).unwrap(), bindings).unwrap()
            })
            .collect::<Vec<_>>()
    };
    let base = images(&compiled, &bindings);
    assert_ne!(base[0], base[1]);
    for (storage, implementation) in [(128, "impl-a"), (64, "replacement-a")] {
        let (_, bindings, compiled) = fixture(1, storage, 0, implementation);
        let changed = images(&compiled, &bindings);
        assert_ne!(base[0], changed[0]);
        assert_eq!(base[1], changed[1]);
    }
}

/// Canonical order is stable while a different codec capability changes the shared mapping.
#[test]
fn mapping_is_canonical_and_tracks_selected_codec() {
    let (topology, bindings, compiled) = fixture(1, 64, 0, "impl-a");
    let base = coordination(&compiled, &topology).unwrap();
    let key = compiled.federates().iter().next().unwrap().0;
    let image = federate_image(&compiled.federate_slice(key).unwrap(), &bindings).unwrap();
    let (topology, bindings, reordered) =
        fixture_profile(1, 64, 0, "impl-a", "serde-json", true, 8);
    assert_eq!(base, coordination(&reordered, &topology).unwrap());
    assert_eq!(
        image,
        federate_image(&reordered.federate_slice(key).unwrap(), &bindings).unwrap()
    );
    let (topology, _, changed) = fixture_profile(1, 64, 0, "impl-a", "postcard-v1", false, 8);
    assert_ne!(base, coordination(&changed, &topology).unwrap());
    assert_ne!(mapping(&compiled).unwrap(), mapping(&changed).unwrap());
}

/// A reused package does not make another component's descriptor part of the local image.
#[test]
fn reused_implementation_descriptors_follow_owning_component() {
    let (_, mut bindings, compiled) = fixture(1, 64, 0, "impl-b");
    let key = compiled.federates().iter().next().unwrap().0;
    let slice = compiled.federate_slice(key).unwrap();
    let baseline = federate_image(&slice, &bindings).unwrap();
    bindings.reverse();
    assert_eq!(baseline, federate_image(&slice, &bindings).unwrap());
}

#[test]
fn coordination_tracks_compiled_in_transit_capacity() {
    let (topology, _, compiled) = fixture(1, 64, 0, "impl-a");
    let base = coordination(&compiled, &topology).unwrap();
    let (topology, _, changed) = fixture_profile(1, 64, 0, "impl-a", "serde-json", false, 9);
    assert_ne!(base, coordination(&changed, &topology).unwrap());
}
