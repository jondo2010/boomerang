use boomerang_builder::{
    compiler::*, ComponentDescriptor, DescriptorBound, DescriptorBounds, PortSlot, PortSlotId,
    ReactorSlot, ReactorSlotId, COMPONENT_DESCRIPTOR_MACRO_ABI,
};
use boomerang_codegen::*;
use boomerang_runtime::image::{
    BoundaryFailurePolicy, CodecPolicy, RecoveryPolicy, SecurityPolicy, TimingPolicy,
    TransportPolicy,
};

/// Supplies explicit shared metadata without CLI discovery or topology serialization.
pub fn inputs(version: u64, delay: u64, codec: &str, reverse: bool) -> DeploymentInputs {
    let state_bytes = 64;
    let queue_capacity = 8;
    let implementation = "impl-a";
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
                capabilities: None,
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
        bindings.push(ImplementationBinding::new(
            ComponentInstanceId::new(name).unwrap(),
            if name == "a" {
                ImplementationId::new(implementation).unwrap()
            } else {
                ImplementationId::new("impl-b").unwrap()
            },
            descriptor,
        ));
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
    DeploymentInputs {
        topology,
        configuration: DeploymentConfiguration {
            name: "shared".into(),
            placements,
            federates,
            coordination: CoordinationSelection::Distributed {
                backend: CoordinationBackend::CentralRti,
            },
            boundaries: vec![BoundaryBinding::new(
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
        },
        bindings,
    }
}
