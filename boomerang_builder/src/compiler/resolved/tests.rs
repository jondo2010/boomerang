use crate::compiler::{
    ApplicationTopology, ApplicationTopologyBuilder, BoundaryBinding, BoundaryId, BoundaryPolicies,
    CodecCapabilityId, ComponentInstance, ComponentInstanceId, ConnectionSemantics, ContractId,
    CoordinationBackend, CoordinationSelection, FederateConfig, FederateId, FlowId,
    ImplementationBinding, ImplementationId, PhysicalBoundaryMetadata, PlacementAssignment,
    PlacementGroupId, PortDirection, PortId, Reactor, ReactorId, ResolveError, ResolvedDeployment,
    RuntimeBackendId, StableEnclaveId, TargetTriple, TransportCapabilityId,
};
use crate::descriptor::{ComponentDescriptor, DescriptorBounds, COMPONENT_DESCRIPTOR_MACRO_ABI};
use crate::runtime::image::{
    BoundaryFailurePolicy, CodecPolicy, RecoveryPolicy, SecurityPolicy, TimingPolicy,
    TransportPolicy,
};

fn descriptor_at_version(contract: &str, contract_version: u64) -> ComponentDescriptor {
    ComponentDescriptor::try_new(
        ContractId::new(contract).unwrap(),
        contract_version,
        COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        DescriptorBounds::default(),
    )
    .unwrap()
}

fn descriptor(contract: &str) -> ComponentDescriptor {
    descriptor_at_version(contract, 1)
}

fn topology() -> ApplicationTopology {
    topology_with_layout(true, true, false)
}

fn topology_with_layout(
    controller_is_placed: bool,
    sensor_is_placed: bool,
    shared_enclave: bool,
) -> ApplicationTopology {
    let mut topology = ApplicationTopologyBuilder::new("vehicle").unwrap();
    let controller_component = ComponentInstanceId::new("vehicle/controller").unwrap();
    let sensor_component = ComponentInstanceId::new("vehicle/sensor").unwrap();
    let controller_reactor = ReactorId::new("vehicle/controller").unwrap();
    let sensor_reactor = ReactorId::new("vehicle/sensor").unwrap();
    let controller_enclave = StableEnclaveId::new("vehicle/controller").unwrap();
    let sensor_enclave = StableEnclaveId::new("vehicle/sensor").unwrap();
    let controller_placement_group = PlacementGroupId::new("placement/controller").unwrap();
    let sensor_placement_group = PlacementGroupId::new("placement/sensor").unwrap();
    topology
        .add_component(ComponentInstance::new("vehicle/controller", "controller.v1", 1).unwrap())
        .unwrap();
    topology
        .add_component(ComponentInstance::new("vehicle/sensor", "sensor.v1", 1).unwrap())
        .unwrap();
    topology
        .add_placement_group(controller_placement_group.clone(), None)
        .unwrap();
    topology
        .add_placement_group(sensor_placement_group.clone(), None)
        .unwrap();
    topology
        .add_reactor(Reactor::new(
            controller_reactor.clone(),
            controller_component,
            None,
            None,
            controller_enclave.clone(),
            controller_is_placed.then_some(controller_placement_group),
            None,
        ))
        .unwrap();
    topology
        .add_reactor(Reactor::new(
            sensor_reactor.clone(),
            sensor_component,
            None,
            None,
            if shared_enclave {
                controller_enclave.clone()
            } else {
                sensor_enclave.clone()
            },
            sensor_is_placed.then_some(sensor_placement_group),
            None,
        ))
        .unwrap();
    topology
        .add_enclave(controller_enclave, controller_reactor.clone())
        .unwrap();
    if !shared_enclave {
        topology
            .add_enclave(sensor_enclave, sensor_reactor.clone())
            .unwrap();
    }
    let controller_port = PortId::new("vehicle/controller/output").unwrap();
    let sensor_port = PortId::new("vehicle/sensor/input").unwrap();
    topology
        .add_port(
            controller_port.clone(),
            controller_reactor,
            PortDirection::Output,
            None,
            0,
            None,
        )
        .unwrap();
    topology
        .add_port(
            sensor_port.clone(),
            sensor_reactor,
            PortDirection::Input,
            None,
            0,
            None,
        )
        .unwrap();
    topology
        .add_connection(
            BoundaryId::new("controller-to-sensor").unwrap(),
            controller_port,
            sensor_port,
            ConnectionSemantics::Logical { after: None },
        )
        .unwrap();
    topology.finish().unwrap()
}

fn binding(component: &str, implementation: &str, contract: &str) -> ImplementationBinding {
    ImplementationBinding::new(
        ComponentInstanceId::new(component).unwrap(),
        ImplementationId::new(implementation).unwrap(),
        descriptor(contract),
    )
}

fn placement(group: &str, federate: &str) -> PlacementAssignment {
    PlacementAssignment::new(
        PlacementGroupId::new(group).unwrap(),
        FederateId::new(federate).unwrap(),
    )
}

fn federate(id: &str, target: &str, runtime: &str) -> FederateConfig {
    FederateConfig::new(
        FederateId::new(id).unwrap(),
        TargetTriple::new(target).unwrap(),
        RuntimeBackendId::new(runtime).unwrap(),
        RecoveryPolicy::FailStop,
    )
}

fn boundary_binding(boundary: &str, codec: &str, transport: &str) -> BoundaryBinding {
    BoundaryBinding::new(
        BoundaryId::new(boundary).unwrap(),
        FlowId::new("sensor-control").unwrap(),
        PhysicalBoundaryMetadata::new(None, None),
        CodecCapabilityId::new(codec).unwrap(),
        TransportCapabilityId::new(transport).unwrap(),
        BoundaryPolicies::new(
            BoundaryFailurePolicy::PropagateStop,
            TransportPolicy::ReliableOrderedFramed,
            CodecPolicy::CanonicalBounded,
            TimingPolicy::BestEffort,
            SecurityPolicy::None,
        ),
    )
}

fn distributed_coordination() -> CoordinationSelection {
    CoordinationSelection::Distributed {
        backend: CoordinationBackend::CentralRti,
    }
}

fn standard_federates() -> [FederateConfig; 2] {
    [
        federate("host", "x86_64-unknown-linux-gnu", "native"),
        federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
    ]
}

fn controller_to_sensor_binding() -> BoundaryBinding {
    boundary_binding("controller-to-sensor", "serde-json", "quic")
}

fn resolve(reverse: bool) -> ResolvedDeployment {
    let mut bindings = vec![
        binding("vehicle/controller", "controller-host", "controller.v1"),
        binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
    ];
    let mut placements = vec![
        placement("placement/controller", "host"),
        placement("placement/sensor", "edge"),
    ];
    let mut federates = standard_federates().into_iter().collect::<Vec<_>>();
    let mut boundary_bindings = vec![controller_to_sensor_binding()];
    if reverse {
        bindings.reverse();
        placements.reverse();
        federates.reverse();
        boundary_bindings.reverse();
    }
    ResolvedDeployment::new(
        topology(),
        bindings,
        placements,
        federates,
        distributed_coordination(),
        boundary_bindings,
    )
    .unwrap()
}

fn resolution_error(
    bindings: impl IntoIterator<Item = ImplementationBinding>,
    placements: impl IntoIterator<Item = PlacementAssignment>,
    federates: impl IntoIterator<Item = FederateConfig>,
    coordination: CoordinationSelection,
    boundary_bindings: impl IntoIterator<Item = BoundaryBinding>,
) -> ResolveError {
    ResolvedDeployment::new(
        topology(),
        bindings,
        placements,
        federates,
        coordination,
        boundary_bindings,
    )
    .unwrap_err()
}

#[test]
fn resolution_reports_duplicate_selection_errors_in_canonical_identity_order() {
    let duplicate_component_bindings = |reverse: bool| {
        let mut bindings = vec![
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-sim", "sensor.v1"),
            binding("vehicle/controller", "controller-sim", "controller.v1"),
        ];
        if reverse {
            bindings.reverse();
        }
        resolution_error(
            bindings,
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
    };
    let forward = duplicate_component_bindings(false);
    let reverse = duplicate_component_bindings(true);
    assert_eq!(forward, reverse);
    assert_eq!(
        forward,
        ResolveError::DuplicateComponentBinding {
            component: ComponentInstanceId::new("vehicle/controller").unwrap(),
        }
    );

    let duplicate_placements = |reverse: bool| {
        let mut placements = vec![
            placement("placement/sensor", "edge"),
            placement("placement/controller", "host"),
            placement("placement/sensor", "host"),
            placement("placement/controller", "edge"),
        ];
        if reverse {
            placements.reverse();
        }
        resolution_error(
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            placements,
            standard_federates(),
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
    };
    let forward = duplicate_placements(false);
    let reverse = duplicate_placements(true);
    assert_eq!(forward, reverse);
    assert_eq!(
        forward,
        ResolveError::DuplicatePlacementAssignment {
            placement_group: PlacementGroupId::new("placement/controller").unwrap(),
        }
    );

    let duplicate_federates = |reverse: bool| {
        let mut federates = vec![
            federate("host", "x86_64-unknown-linux-gnu", "native"),
            federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
            federate("host", "x86_64-unknown-linux-musl", "native"),
            federate("edge", "aarch64-unknown-linux-musl", "rtic"),
        ];
        if reverse {
            federates.reverse();
        }
        resolution_error(
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            federates,
            distributed_coordination(),
            [controller_to_sensor_binding()],
        )
    };
    let forward = duplicate_federates(false);
    let reverse = duplicate_federates(true);
    assert_eq!(forward, reverse);
    assert_eq!(
        forward,
        ResolveError::DuplicateFederateConfig {
            federate: FederateId::new("edge").unwrap(),
        }
    );

    let duplicate_boundary_bindings = |reverse: bool| {
        let mut boundary_bindings = vec![
            boundary_binding("unknown", "serde-json", "quic"),
            controller_to_sensor_binding(),
            boundary_binding("unknown", "postcard", "tcp"),
            boundary_binding("controller-to-sensor", "postcard", "tcp"),
        ];
        if reverse {
            boundary_bindings.reverse();
        }
        resolution_error(
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            [
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates(),
            distributed_coordination(),
            boundary_bindings,
        )
    };
    let forward = duplicate_boundary_bindings(false);
    let reverse = duplicate_boundary_bindings(true);
    assert_eq!(forward, reverse);
    assert_eq!(
        forward,
        ResolveError::DuplicateBoundaryBinding {
            boundary: BoundaryId::new("controller-to-sensor").unwrap(),
        }
    );
}

#[test]
fn resolution_requires_complete_reactor_placement_and_enclave_ownership() {
    let unplaced_reactor = ResolvedDeployment::new(
        topology_with_layout(true, false, false),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        unplaced_reactor,
        ResolveError::UnplacedReactor { reactor }
            if reactor.to_string() == "vehicle/sensor"
    ));

    let split_enclave = ResolvedDeployment::new(
        topology_with_layout(true, true, true),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        split_enclave,
        ResolveError::SplitEnclave {
            enclave,
            first,
            second,
        } if enclave.to_string() == "vehicle/controller"
            && first.as_str() == "host"
            && second.as_str() == "edge"
    ));

    let no_federates = ResolvedDeployment::new(
        ApplicationTopologyBuilder::new("empty")
            .unwrap()
            .finish()
            .unwrap(),
        [],
        [],
        [],
        CoordinationSelection::Local,
        [],
    )
    .unwrap_err();
    assert!(matches!(no_federates, ResolveError::NoFederates));
}

#[test]
fn resolution_requires_coordination_matching_federate_cardinality() {
    let cases = [
        (
            "one Federate with local coordination",
            vec![
                placement("placement/controller", "host"),
                placement("placement/sensor", "host"),
            ],
            vec![federate("host", "x86_64-unknown-linux-gnu", "native")],
            CoordinationSelection::Local,
            vec![],
            1,
            true,
        ),
        (
            "one Federate with distributed coordination",
            vec![
                placement("placement/controller", "host"),
                placement("placement/sensor", "host"),
            ],
            vec![federate("host", "x86_64-unknown-linux-gnu", "native")],
            distributed_coordination(),
            vec![],
            1,
            false,
        ),
        (
            "two Federates with local coordination",
            vec![
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates().into(),
            CoordinationSelection::Local,
            vec![controller_to_sensor_binding()],
            2,
            false,
        ),
        (
            "two Federates with distributed coordination",
            vec![
                placement("placement/controller", "host"),
                placement("placement/sensor", "edge"),
            ],
            standard_federates().into(),
            distributed_coordination(),
            vec![controller_to_sensor_binding()],
            2,
            true,
        ),
    ];

    for (
        name,
        placements,
        federates,
        coordination,
        boundary_bindings,
        expected_federate_count,
        accepted,
    ) in cases
    {
        let result = ResolvedDeployment::new(
            topology(),
            [
                binding("vehicle/controller", "controller-host", "controller.v1"),
                binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
            ],
            placements,
            federates,
            coordination.clone(),
            boundary_bindings,
        );

        match (accepted, result) {
            (true, Ok(_)) => {}
            (
                false,
                Err(ResolveError::InvalidCoordination {
                    federate_count,
                    coordination: selected,
                }),
            ) => {
                assert_eq!(federate_count, expected_federate_count, "{name}");
                assert_eq!(selected, coordination, "{name}");
            }
            (_, result) => panic!("unexpected resolution result for {name}: {result:?}"),
        }
    }
}

#[test]
fn resolution_requires_exact_cross_federate_boundary_bindings() {
    let missing_binding = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [],
    )
    .unwrap_err();
    assert!(matches!(
        missing_binding,
        ResolveError::MissingBoundaryBinding { boundary }
            if boundary.to_string() == "controller-to-sensor"
    ));

    let resolved = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap();
    let selected_binding = resolved
        .boundary_binding(&BoundaryId::new("controller-to-sensor").unwrap())
        .expect("cross-Federate connection must retain its binding");
    assert_eq!(selected_binding.codec().to_string(), "serde-json");
    assert_eq!(selected_binding.transport().to_string(), "quic");

    let unexpected_binding = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "host"),
        ],
        [federate("host", "x86_64-unknown-linux-gnu", "native")],
        CoordinationSelection::Local,
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        unexpected_binding,
        ResolveError::UnexpectedBoundaryBinding { boundary }
            if boundary.to_string() == "controller-to-sensor"
    ));

    let unknown_binding = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [boundary_binding("unknown", "serde-json", "quic")],
    )
    .unwrap_err();
    assert!(matches!(
        unknown_binding,
        ResolveError::UnknownBoundaryBinding { boundary }
            if boundary.to_string() == "unknown"
    ));

    let duplicate_binding = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [
            controller_to_sensor_binding(),
            boundary_binding("controller-to-sensor", "postcard", "tcp"),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_binding,
        ResolveError::DuplicateBoundaryBinding { boundary }
            if boundary.to_string() == "controller-to-sensor"
    ));
}

#[test]
fn resolution_is_canonical_under_selection_reordering() {
    let forward = resolve(false);
    let reverse = resolve(true);

    assert_eq!(forward, reverse);
    assert_eq!(
        forward
            .bindings()
            .map(|binding| binding.component().to_string())
            .collect::<Vec<_>>(),
        ["vehicle/controller", "vehicle/sensor"]
    );
    assert_eq!(
        forward
            .binding(&ComponentInstanceId::new("vehicle/controller").unwrap())
            .map(|binding| binding.implementation().to_string()),
        Some("controller-host".to_owned())
    );
    assert_eq!(
        forward
            .placements()
            .map(|assignment| (
                assignment.placement_group().to_string(),
                assignment.federate().as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("placement/controller".to_owned(), "host"),
            ("placement/sensor".to_owned(), "edge"),
        ]
    );
    assert_eq!(
        forward
            .federates()
            .map(|federate| federate.id().as_str())
            .collect::<Vec<_>>(),
        ["edge", "host"]
    );
    let edge = forward
        .federate(&FederateId::new("edge").unwrap())
        .expect("edge Federate configuration must be retained");
    assert_eq!(edge.target().to_string(), "aarch64-unknown-linux-gnu");
    assert_eq!(edge.runtime().to_string(), "rtic");
    assert_eq!(
        forward
            .boundary_bindings()
            .map(|binding| binding.boundary().to_string())
            .collect::<Vec<_>>(),
        ["controller-to-sensor"]
    );
    let boundary = forward
        .boundary_binding(&BoundaryId::new("controller-to-sensor").unwrap())
        .expect("cross-Federate boundary binding must be retained");
    assert_eq!(boundary.codec().to_string(), "serde-json");
    assert_eq!(boundary.transport().to_string(), "quic");
    assert_eq!(forward.coordination(), &distributed_coordination());
}

#[test]
fn resolution_rejects_contract_mismatch_at_the_component_boundary() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "wrong.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::ContractMismatch {
            component,
            required,
            required_version,
            provided,
            provided_version,
            implementation,
        }
            if component.to_string() == "vehicle/controller"
                && required.as_str() == "controller.v1"
                && required_version == 1
                && provided.as_str() == "wrong.v1"
                && provided_version == 1
                && implementation.as_str() == "controller-host"
    ));
}

#[test]
fn resolution_rejects_contract_version_mismatch_at_the_component_boundary() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/controller").unwrap(),
                ImplementationId::new("controller-host").unwrap(),
                descriptor_at_version("controller.v1", 2),
            ),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::ContractMismatch {
            component,
            required,
            required_version,
            provided,
            provided_version,
            implementation,
        } if component.to_string() == "vehicle/controller"
            && required.as_str() == "controller.v1"
            && required_version == 1
            && provided.as_str() == "controller.v1"
            && provided_version == 2
            && implementation.as_str() == "controller-host"
    ));
}

#[test]
fn resolution_requires_exact_component_and_placement_coverage() {
    let missing_binding = ResolvedDeployment::new(
        topology(),
        [binding(
            "vehicle/controller",
            "controller-host",
            "controller.v1",
        )],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        missing_binding,
        ResolveError::MissingComponentBinding { component }
            if component.to_string() == "vehicle/sensor"
    ));

    let missing_placement = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [placement("placement/controller", "host")],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        missing_placement,
        ResolveError::MissingPlacementAssignment { placement_group }
            if placement_group.to_string() == "placement/sensor"
    ));
}

#[test]
fn resolution_rejects_duplicate_component_and_placement_selections() {
    let duplicate_binding = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/controller", "controller-sim", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_binding,
        ResolveError::DuplicateComponentBinding { component }
            if component.to_string() == "vehicle/controller"
    ));

    let duplicate_placement = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/controller", "edge"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_placement,
        ResolveError::DuplicatePlacementAssignment { placement_group }
            if placement_group.to_string() == "placement/controller"
    ));
}

#[test]
fn resolution_rejects_duplicate_federate_configs() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        [
            federate("host", "x86_64-unknown-linux-gnu", "native"),
            federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
            federate("edge", "aarch64-unknown-linux-musl", "rtic"),
        ],
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::DuplicateFederateConfig { federate } if federate.as_str() == "edge"
    ));
}

#[test]
fn resolution_rejects_duplicate_boundary_bindings() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        standard_federates(),
        distributed_coordination(),
        [
            controller_to_sensor_binding(),
            boundary_binding("controller-to-sensor", "postcard", "tcp"),
        ],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::DuplicateBoundaryBinding { boundary }
            if boundary.to_string() == "controller-to-sensor"
    ));
}

#[test]
fn resolution_requires_a_federate_config_for_every_placement() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        [federate("host", "x86_64-unknown-linux-gnu", "native")],
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::MissingFederateConfig {
            placement_group,
            federate,
        } if placement_group.to_string() == "placement/sensor" && federate.as_str() == "edge"
    ));
}

#[test]
fn resolution_rejects_unused_federate_configs() {
    let error = ResolvedDeployment::new(
        topology(),
        [
            binding("vehicle/controller", "controller-host", "controller.v1"),
            binding("vehicle/sensor", "sensor-mcu", "sensor.v1"),
        ],
        [
            placement("placement/controller", "host"),
            placement("placement/sensor", "edge"),
        ],
        [
            federate("host", "x86_64-unknown-linux-gnu", "native"),
            federate("edge", "aarch64-unknown-linux-gnu", "rtic"),
            federate("spare", "wasm32-wasip1", "sim"),
        ],
        distributed_coordination(),
        [controller_to_sensor_binding()],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ResolveError::UnusedFederateConfig { federate } if federate.as_str() == "spare"
    ));
}
