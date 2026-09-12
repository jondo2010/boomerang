use super::CompileError;
use crate::{
    compiler::{
        compiled::FederateSliceError, ActionId, ActionKind, ApplicationTopology,
        ApplicationTopologyBuilder, BankMember, BoundaryBinding, BoundaryId, BoundaryPolicies,
        CodecCapabilityId, ComponentInstance, ComponentInstanceId, ConnectionSemantics,
        CoordinationBackend, CoordinationSelection, FederateConfig, FederateId, FlowId,
        ImplementationBinding, ImplementationId, ModeId, ModeTransition, ModeTransitionKind,
        OwnedCompiledDeployment, PhysicalBoundaryId, PhysicalBoundaryMetadata, PlacementAssignment,
        PlacementGroupId, PortDirection, PortId, ReactionId, ReactionOptions, ReactionRelation,
        ReactionRelationFlags, ReactionRelationTarget, Reactor, ReactorId, RequiredBinding,
        ResolvedDeployment, RuntimeBackendId, StableEnclaveId, TargetTriple, TransportCapabilityId,
    },
    descriptor::{
        ActionSlot, ActionSlotId, ComponentDescriptor, DescriptorBound, DescriptorBounds, PortSlot,
        PortSlotId, ReactionSlot, ReactionSlotId, ReactorSlot, ReactorSlotId,
        COMPONENT_DESCRIPTOR_MACRO_ABI,
    },
    runtime::image::{
        ActionIndex, ActionTiming, BindingKind, BoundaryFailurePolicy, CodecPolicy,
        CoordinationProjection, FederateIndex, ModeIndex, ReactionIndex, ReactorIndex,
        RecoveryPolicy, RouteDirection, RouteIndex, RtiImage, RtiRouteIndex, ScopeIndex,
        SecurityPolicy, TimingDomain, TimingPolicy, TransportPolicy,
    },
};

fn federate_enclaves(
    deployment: &OwnedCompiledDeployment,
    federate: FederateIndex,
) -> &[crate::compiler::OwnedEnclaveImage] {
    deployment
        .enclaves()
        .get_span(deployment.federates()[federate].enclaves())
        .expect("lowered Federate span belongs to the deployment Enclave table")
}

fn descriptor(contract: &str, bounds: DescriptorBounds) -> ComponentDescriptor {
    let root = match contract {
        "controller.v1" => "Controller",
        "sensor.v1" => "Sensor",
        _ => panic!("unexpected test descriptor contract {contract}"),
    };
    descriptor_with_root(contract, bounds, root)
}

fn descriptor_with_root(
    contract: &str,
    bounds: DescriptorBounds,
    root: &str,
) -> ComponentDescriptor {
    let reactions = match contract {
        "controller.v1" => [
            "emit",
            "reset_active",
            "shutdown",
            "start",
            "timer_fired",
            "#g0",
            "#g1",
            "#g2",
            "#g3",
            "#g4",
            "#g5",
            "#g6",
            "#g7",
            "#g8",
            "#g9",
            "#g10",
            "%23generated%5B0%5D",
        ]
        .as_slice(),
        "sensor.v1" => ["receive"].as_slice(),
        _ => panic!("unexpected test descriptor contract {contract}"),
    };
    let reactor = ReactorSlotId::new(root).unwrap();
    let port = |name| PortSlot {
        id: PortSlotId::new(format!("{root}/{name}")).unwrap(),
        reactor: reactor.clone(),
        direction: if name == "output" {
            PortDirection::Output
        } else {
            PortDirection::Input
        },
    };
    let action = |name| ActionSlot {
        id: ActionSlotId::new(format!("{root}/{name}")).unwrap(),
        reactor: reactor.clone(),
    };
    let (ports, actions) = match contract {
        "controller.v1" => (
            vec![port("output"), port("array_in"), port("bank_in")],
            vec![action("pulse")],
        ),
        "sensor.v1" => (vec![port("input")], vec![action("ack")]),
        _ => unreachable!(),
    };
    ComponentDescriptor::try_new(
        contract.parse().unwrap(),
        1,
        COMPONENT_DESCRIPTOR_MACRO_ABI,
        vec![ReactorSlot {
            id: reactor.clone(),
            parent: None,
        }],
        ports,
        actions,
        reactions
            .iter()
            .map(|reaction| ReactionSlot {
                id: ReactionSlotId::new(format!("{root}/{reaction}")).unwrap(),
                reactor: reactor.clone(),
            })
            .collect(),
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        bounds,
    )
    .unwrap()
}
fn ordered<T>(mut values: Vec<T>, reverse: bool) -> Vec<T> {
    if reverse {
        values.reverse();
    }
    values
}
#[derive(Clone, Copy)]
enum DependencyCase {
    None,
    ReactionCycle,
    PortSelfCycle,
    MutuallyExclusiveModes,
    EncodedOrdering,
    ModeTransition,
}
fn topology(
    reverse: bool,
    shared_enclave: bool,
    semantics: ConnectionSemantics,
    dependency_case: DependencyCase,
    parallel: bool,
) -> ApplicationTopology {
    let mut topology = ApplicationTopologyBuilder::new("vehicle").unwrap();
    let controller = ComponentInstanceId::new("vehicle/controller").unwrap();
    let sensor = ComponentInstanceId::new("vehicle/sensor").unwrap();
    let controller_reactor = ReactorId::new("vehicle/controller").unwrap();
    let sensor_reactor = ReactorId::new("vehicle/sensor").unwrap();
    let controller_enclave = StableEnclaveId::new("vehicle/controller").unwrap();
    let sensor_enclave = StableEnclaveId::new("vehicle/sensor").unwrap();
    let controller_group = PlacementGroupId::new("placement/controller").unwrap();
    let sensor_group = PlacementGroupId::new("placement/sensor").unwrap();
    for component in ordered(
        vec![
            ComponentInstance::new("vehicle/controller", "controller.v1", 1).unwrap(),
            ComponentInstance::new("vehicle/sensor", "sensor.v1", 1).unwrap(),
        ],
        reverse,
    ) {
        topology.add_component(component).unwrap();
    }
    for group in ordered(
        vec![controller_group.clone(), sensor_group.clone()],
        reverse,
    ) {
        topology.add_placement_group(group, None).unwrap();
    }
    for reactor in ordered(
        vec![
            Reactor::new(
                controller_reactor.clone(),
                controller.clone(),
                None,
                None,
                controller_enclave.clone(),
                Some(controller_group.clone()),
                None,
            ),
            Reactor::new(
                sensor_reactor.clone(),
                sensor.clone(),
                None,
                None,
                if shared_enclave {
                    controller_enclave.clone()
                } else {
                    sensor_enclave.clone()
                },
                Some(sensor_group.clone()),
                None,
            ),
        ],
        reverse,
    ) {
        topology.add_reactor(reactor).unwrap();
    }
    let mut enclaves = vec![(controller_enclave, controller_reactor.clone())];
    if !shared_enclave {
        enclaves.push((sensor_enclave, sensor_reactor.clone()));
    }
    for (id, root) in ordered(enclaves, reverse) {
        topology.add_enclave(id, root).unwrap();
    }
    let active = ModeId::new("vehicle/controller/active").unwrap();
    let idle = ModeId::new("vehicle/controller/idle").unwrap();
    for (id, initial) in ordered(vec![(active.clone(), false), (idle.clone(), true)], reverse) {
        topology
            .add_mode(id, controller_reactor.clone(), None, initial)
            .unwrap();
    }
    let pulse = ActionId::new("vehicle/controller/pulse").unwrap();
    let shutdown = ActionId::new("vehicle/controller/shutdown").unwrap();
    let startup = ActionId::new("vehicle/controller/startup").unwrap();
    let timer = ActionId::new("vehicle/controller/timer").unwrap();
    let ack = ActionId::new("vehicle/sensor/ack").unwrap();
    let ns = crate::runtime::Duration::nanoseconds;
    for (id, reactor, kind, position, mode) in ordered(
        vec![
            (
                startup.clone(),
                controller_reactor.clone(),
                ActionKind::Startup,
                0,
                None,
            ),
            (
                shutdown.clone(),
                controller_reactor.clone(),
                ActionKind::Shutdown,
                1,
                None,
            ),
            (
                pulse.clone(),
                controller_reactor.clone(),
                ActionKind::Logical {
                    minimum_delay: Some(ns(3)),
                },
                2,
                None,
            ),
            (
                timer.clone(),
                controller_reactor.clone(),
                ActionKind::Timer {
                    offset: Some(ns(5)),
                    period: Some(ns(7)),
                },
                3,
                Some(idle.clone()),
            ),
            (
                ack.clone(),
                sensor_reactor.clone(),
                ActionKind::Physical {
                    minimum_delay: Some(ns(11)),
                },
                0,
                None,
            ),
        ],
        reverse,
    ) {
        topology
            .add_action(id, reactor, kind, position, mode)
            .unwrap();
    }
    let output = PortId::new("vehicle/controller/output").unwrap();
    let input = PortId::new("vehicle/sensor/input").unwrap();
    for (id, reactor, direction) in ordered(
        vec![
            (
                output.clone(),
                controller_reactor.clone(),
                PortDirection::Output,
            ),
            (input.clone(), sensor_reactor.clone(), PortDirection::Input),
        ],
        reverse,
    ) {
        topology
            .add_port(id, reactor, direction, None, 0, None)
            .unwrap();
    }
    for (position, name) in ["array_in", "array_in", "bank_in", "bank_in"]
        .into_iter()
        .enumerate()
    {
        let index = position as u32 % 2;
        topology
            .add_port(
                PortId::new(format!("vehicle/controller/{name}/#b{index}")).unwrap(),
                controller_reactor.clone(),
                PortDirection::Input,
                Some(BankMember::new(index, 2).unwrap()),
                position as u32 + 1,
                None,
            )
            .unwrap();
    }
    topology
        .add_connection(
            BoundaryId::new(if parallel {
                "route/-"
            } else {
                "controller-to-sensor"
            })
            .unwrap(),
            output.clone(),
            input.clone(),
            semantics,
        )
        .unwrap();
    if parallel {
        topology
            .add_connection(
                BoundaryId::new("route/%2F").unwrap(),
                output,
                input,
                semantics,
            )
            .unwrap();
    }
    let mut emit_relations = vec![
        ReactionRelation::new(
            ReactionRelationTarget::Action(pulse.clone()),
            ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
            0,
        ),
        ReactionRelation::new(
            ReactionRelationTarget::Port(PortId::new("vehicle/controller/output").unwrap()),
            if matches!(dependency_case, DependencyCase::PortSelfCycle) {
                ReactionRelationFlags::TRIGGER | ReactionRelationFlags::EFFECT
            } else {
                ReactionRelationFlags::EFFECT
            },
            0,
        ),
    ];
    if matches!(dependency_case, DependencyCase::ReactionCycle) {
        emit_relations.insert(
            1,
            ReactionRelation::new(
                ReactionRelationTarget::Action(startup.clone()),
                ReactionRelationFlags::EFFECT,
                1,
            ),
        );
    }
    let reset_active_relations = matches!(dependency_case, DependencyCase::MutuallyExclusiveModes)
        .then(|| {
            vec![ReactionRelation::new(
                ReactionRelationTarget::Action(timer.clone()),
                ReactionRelationFlags::EFFECT,
                0,
            )]
        })
        .unwrap_or_default();
    let timer_relations = vec![ReactionRelation::new(
        ReactionRelationTarget::Action(timer),
        ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
        0,
    )];
    let mut reactions = vec![
        (
            ReactionId::new("vehicle/controller/emit").unwrap(),
            controller_reactor.clone(),
            emit_relations,
            ReactionOptions::default(),
        ),
        (
            ReactionId::new("vehicle/controller/reset_active").unwrap(),
            controller_reactor.clone(),
            reset_active_relations,
            ReactionOptions {
                mode: Some(active.clone()),
                enabled_modes: vec![active.clone()],
                reset_modes: vec![active.clone()],
                transition: matches!(dependency_case, DependencyCase::ModeTransition)
                    .then(|| ModeTransition::new(idle.clone(), ModeTransitionKind::Reset)),
            },
        ),
        (
            ReactionId::new("vehicle/controller/shutdown").unwrap(),
            controller_reactor.clone(),
            vec![ReactionRelation::new(
                ReactionRelationTarget::Action(shutdown),
                ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                0,
            )],
            ReactionOptions::default(),
        ),
        (
            ReactionId::new("vehicle/controller/start").unwrap(),
            controller_reactor.clone(),
            vec![
                ReactionRelation::new(
                    ReactionRelationTarget::Action(startup),
                    ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                    0,
                ),
                ReactionRelation::new(
                    ReactionRelationTarget::Action(pulse),
                    ReactionRelationFlags::EFFECT,
                    1,
                ),
            ],
            ReactionOptions::default(),
        ),
        (
            ReactionId::new("vehicle/controller/timer_fired").unwrap(),
            controller_reactor,
            timer_relations,
            ReactionOptions {
                mode: Some(idle.clone()),
                enabled_modes: vec![idle],
                reset_modes: vec![],
                transition: None,
            },
        ),
        (
            ReactionId::new("vehicle/sensor/receive").unwrap(),
            sensor_reactor,
            vec![
                ReactionRelation::new(
                    ReactionRelationTarget::Action(ack),
                    ReactionRelationFlags::EFFECT,
                    0,
                ),
                ReactionRelation::new(
                    ReactionRelationTarget::Port(PortId::new("vehicle/sensor/input").unwrap()),
                    ReactionRelationFlags::TRIGGER | ReactionRelationFlags::USE,
                    0,
                ),
            ],
            ReactionOptions::default(),
        ),
    ];
    if matches!(dependency_case, DependencyCase::EncodedOrdering) {
        reactions.extend((0..=10).map(|ordinal| {
            (
                ReactionId::new(format!("vehicle/controller/#g{ordinal}")).unwrap(),
                ReactorId::new("vehicle/controller").unwrap(),
                vec![],
                ReactionOptions::default(),
            )
        }));
        reactions.push((
            ReactionId::new("vehicle/controller/%23generated%5B0%5D").unwrap(),
            ReactorId::new("vehicle/controller").unwrap(),
            vec![],
            ReactionOptions::default(),
        ));
    }
    for (id, reactor, relations, options) in ordered(reactions, reverse) {
        topology
            .add_reaction(id, reactor, relations, options)
            .unwrap();
    }
    topology.finish().unwrap()
}
fn known_bounds() -> DescriptorBounds {
    DescriptorBounds {
        queue_capacity: DescriptorBound::Known(8),
        payload_bytes: DescriptorBound::Known(16),
        state_bytes: DescriptorBound::Known(32),
        scratch_bytes: DescriptorBound::Known(64),
    }
}
#[allow(clippy::too_many_arguments, reason = "shared lowering test fixture")]
fn deployment_with_bounds(
    reverse: bool,
    distributed: bool,
    shared_enclave: bool,
    semantics: ConnectionSemantics,
    bounds: [DescriptorBounds; 2],
    dependency_case: DependencyCase,
    recovery: RecoveryPolicy,
    parallel: bool,
) -> ResolvedDeployment {
    let mut bindings = vec![
        ImplementationBinding::new(
            ComponentInstanceId::new("vehicle/controller").unwrap(),
            ImplementationId::new("controller-host").unwrap(),
            descriptor("controller.v1", bounds[0]),
        ),
        ImplementationBinding::new(
            ComponentInstanceId::new("vehicle/sensor").unwrap(),
            ImplementationId::new("sensor-host").unwrap(),
            descriptor("sensor.v1", bounds[1]),
        ),
    ];
    let mut placements = vec![
        PlacementAssignment::new(
            PlacementGroupId::new("placement/controller").unwrap(),
            FederateId::new("host").unwrap(),
        ),
        PlacementAssignment::new(
            PlacementGroupId::new("placement/sensor").unwrap(),
            FederateId::new(if distributed { "edge" } else { "host" }).unwrap(),
        ),
    ];
    let mut federates = vec![FederateConfig::new(
        FederateId::new("host").unwrap(),
        TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
        RuntimeBackendId::new("native").unwrap(),
        recovery,
    )];
    let mut boundary_bindings = vec![];
    if distributed {
        federates.push(FederateConfig::new(
            FederateId::new("edge").unwrap(),
            TargetTriple::new("aarch64-unknown-none").unwrap(),
            RuntimeBackendId::new("rtic").unwrap(),
            RecoveryPolicy::FailStop,
        ));
        let boundary_binding = |boundary| {
            BoundaryBinding::new(
                BoundaryId::new(boundary).unwrap(),
                FlowId::new("sensor-control").unwrap(),
                PhysicalBoundaryMetadata::new(
                    Some(PhysicalBoundaryId::new("plant/z").unwrap()),
                    Some(PhysicalBoundaryId::new("plant/#g1").unwrap()),
                ),
                CodecCapabilityId::new("postcard").unwrap(),
                TransportCapabilityId::new("udp").unwrap(),
                BoundaryPolicies::new(
                    BoundaryFailurePolicy::PropagateStop,
                    TransportPolicy::ReliableOrderedFramed,
                    CodecPolicy::CanonicalBounded,
                    TimingPolicy::BestEffort,
                    SecurityPolicy::None,
                ),
            )
        };
        boundary_bindings.push(boundary_binding(if parallel {
            "route/-"
        } else {
            "controller-to-sensor"
        }));
        if parallel {
            boundary_bindings.push(boundary_binding("route/%2F"));
        }
    }
    if reverse {
        bindings.reverse();
        placements.reverse();
        federates.reverse();
        boundary_bindings.reverse();
    }
    ResolvedDeployment::new(
        topology(
            reverse,
            shared_enclave,
            semantics,
            dependency_case,
            parallel,
        ),
        bindings,
        placements,
        federates,
        if distributed {
            CoordinationSelection::Distributed {
                backend: CoordinationBackend::CentralRti,
            }
        } else {
            CoordinationSelection::Local
        },
        boundary_bindings,
    )
    .unwrap()
}
fn deployment(reverse: bool, distributed: bool) -> ResolvedDeployment {
    deployment_with_bounds(
        reverse,
        distributed,
        false,
        ConnectionSemantics::Logical { after: None },
        [known_bounds(); 2],
        DependencyCase::None,
        RecoveryPolicy::FailStop,
        false,
    )
}
fn local_deployment(
    shared_enclave: bool,
    semantics: ConnectionSemantics,
    dependency_case: DependencyCase,
) -> ResolvedDeployment {
    deployment_with_bounds(
        false,
        false,
        shared_enclave,
        semantics,
        [known_bounds(); 2],
        dependency_case,
        RecoveryPolicy::FailStop,
        false,
    )
}
fn distributed_with(
    semantics: ConnectionSemantics,
    recovery: RecoveryPolicy,
    parallel: bool,
) -> ResolvedDeployment {
    deployment_with_bounds(
        false,
        true,
        false,
        semantics,
        [known_bounds(); 2],
        DependencyCase::None,
        recovery,
        parallel,
    )
}
fn with_central_rti<T>(compiled: &OwnedCompiledDeployment, f: impl FnOnce(RtiImage<'_>) -> T) -> T {
    compiled.with_coordination(|coordination| {
        let CoordinationProjection::CentralRti(rti) = coordination else {
            panic!("distributed deployment must select the central RTI projection");
        };
        f(rti)
    })
}

fn shared_implementation_deployment(reverse: bool) -> ResolvedDeployment {
    let mut bindings = vec![
        ImplementationBinding::new(
            ComponentInstanceId::new("vehicle/controller").unwrap(),
            ImplementationId::new("shared-host").unwrap(),
            descriptor_with_root("controller.v1", known_bounds(), "Shared"),
        ),
        ImplementationBinding::new(
            ComponentInstanceId::new("vehicle/sensor").unwrap(),
            ImplementationId::new("shared-host").unwrap(),
            descriptor_with_root("sensor.v1", known_bounds(), "Shared"),
        ),
    ];
    let mut placements = vec![
        PlacementAssignment::new(
            PlacementGroupId::new("placement/controller").unwrap(),
            FederateId::new("host").unwrap(),
        ),
        PlacementAssignment::new(
            PlacementGroupId::new("placement/sensor").unwrap(),
            FederateId::new("host").unwrap(),
        ),
    ];
    if reverse {
        bindings.reverse();
        placements.reverse();
    }
    ResolvedDeployment::new(
        topology(
            reverse,
            true,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::None,
            false,
        ),
        bindings,
        placements,
        [FederateConfig::new(
            FederateId::new("host").unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("native").unwrap(),
            RecoveryPolicy::FailStop,
        )],
        CoordinationSelection::Local,
        [],
    )
    .unwrap()
}

fn deployment_with_controller_descriptor(
    controller_descriptor: ComponentDescriptor,
) -> ResolvedDeployment {
    ResolvedDeployment::new(
        topology(
            false,
            false,
            ConnectionSemantics::Logical { after: None },
            DependencyCase::None,
            false,
        ),
        [
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/controller").unwrap(),
                ImplementationId::new("controller-host").unwrap(),
                controller_descriptor,
            ),
            ImplementationBinding::new(
                ComponentInstanceId::new("vehicle/sensor").unwrap(),
                ImplementationId::new("sensor-host").unwrap(),
                descriptor("sensor.v1", known_bounds()),
            ),
        ],
        [
            PlacementAssignment::new(
                PlacementGroupId::new("placement/controller").unwrap(),
                FederateId::new("host").unwrap(),
            ),
            PlacementAssignment::new(
                PlacementGroupId::new("placement/sensor").unwrap(),
                FederateId::new("host").unwrap(),
            ),
        ],
        [FederateConfig::new(
            FederateId::new("host").unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("native").unwrap(),
            RecoveryPolicy::FailStop,
        )],
        CoordinationSelection::Local,
        [],
    )
    .unwrap()
}

fn controller_descriptor_with(
    reactor_slots: Vec<ReactorSlot>,
    reaction_slots: Vec<ReactionSlot>,
) -> ComponentDescriptor {
    ComponentDescriptor::try_new(
        "controller.v1".parse().unwrap(),
        1,
        COMPONENT_DESCRIPTOR_MACRO_ABI,
        reactor_slots,
        vec![],
        vec![],
        reaction_slots,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        known_bounds(),
    )
    .unwrap()
}

#[test]
fn lowering_keeps_reused_symbols_and_runtime_state_slots_distinct() {
    let forward = shared_implementation_deployment(false).lower().unwrap();
    let reverse = shared_implementation_deployment(true).lower().unwrap();
    assert_eq!(forward, reverse);

    let enclave = &federate_enclaves(&forward, FederateIndex::new(0))[0];
    assert_eq!(
        enclave
            .required_bindings()
            .iter()
            .map(RequiredBinding::symbol)
            .collect::<Vec<_>>(),
        [
            "action_Shared_2fpulse",
            "action_Shared_2fack",
            "port_Shared_2farray_5fin",
            "port_Shared_2farray_5fin",
            "port_Shared_2fbank_5fin",
            "port_Shared_2fbank_5fin",
            "port_Shared_2foutput",
            "reaction_Shared_2femit",
            "reaction_Shared_2freset_5factive",
            "reaction_Shared_2fshutdown",
            "reaction_Shared_2fstart",
            "reaction_Shared_2ftimer_5ffired",
            "reaction_Shared_2freceive",
            "state_Shared",
            "state_Shared",
        ]
    );
    let states = enclave
        .required_bindings()
        .iter()
        .filter_map(|binding| match binding {
            RequiredBinding::State {
                component,
                implementation,
                ..
            } => Some((
                component.to_string(),
                implementation.to_string(),
                binding.symbol(),
            )),
            RequiredBinding::Reaction { .. }
            | RequiredBinding::Port { .. }
            | RequiredBinding::Action { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            (
                "vehicle/controller".to_owned(),
                "shared-host".to_owned(),
                "state_Shared".to_owned(),
            ),
            (
                "vehicle/sensor".to_owned(),
                "shared-host".to_owned(),
                "state_Shared".to_owned(),
            ),
        ]
    );

    enclave
        .with_view(|image| {
            assert_eq!(
                image
                    .ports()
                    .values()
                    .map(|port| port.binding())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                5
            );
            assert_ne!(
                image.reactors()[ReactorIndex::new(0)].state_binding(),
                image.reactors()[ReactorIndex::new(1)].state_binding()
            );
            assert_eq!(image.storage_bounds().state_slots(), 2);
        })
        .unwrap();
}

#[test]
fn lowering_reports_invalid_descriptor_slot_mappings() {
    let rootless = controller_descriptor_with(vec![], vec![]);
    assert!(matches!(
        deployment_with_controller_descriptor(rootless).lower(),
        Err(CompileError::DescriptorRoot { roots: 0, .. })
    ));

    let root = ReactorSlotId::new("Controller").unwrap();
    let missing_reaction = controller_descriptor_with(
        vec![ReactorSlot {
            id: root,
            parent: None,
        }],
        vec![],
    );
    assert!(matches!(
        deployment_with_controller_descriptor(missing_reaction).lower(),
        Err(CompileError::MissingDescriptorSlot {
            kind: "reaction",
            logical,
            ..
        }) if logical == "vehicle/controller/emit"
    ));
}

#[test]
fn lowering_is_canonical_under_selection_reordering() {
    let forward = deployment(false, false).lower().unwrap();
    let reverse = deployment(true, false).lower().unwrap();
    assert_eq!(forward, reverse);
    assert_eq!(
        forward.federates()[FederateIndex::new(0)].id().as_str(),
        "host"
    );
    let enclaves = federate_enclaves(&forward, FederateIndex::new(0));
    assert_eq!(enclaves.len(), 2);
    assert_eq!(enclaves[0].id().to_string(), "vehicle/controller");
    assert_eq!(enclaves[1].id().to_string(), "vehicle/sensor");
    for enclave in enclaves {
        enclave.with_view(|_| ()).unwrap();
    }
}
#[test]
fn distributed_lowering_preserves_canonical_member_identities() {
    let forward = deployment(false, true).lower().unwrap();
    let reverse = deployment(true, true).lower().unwrap();
    assert_eq!(forward, reverse);
    assert_eq!(
        forward
            .federation()
            .members()
            .iter()
            .map(FederateId::as_str)
            .collect::<Vec<_>>(),
        ["edge", "host"]
    );
}

/// Projects a real lowered Federate without retaining sibling Enclaves or bindings.
#[test]
fn federate_slice_from_lowered_deployment_preserves_selected_root_rows() {
    let compiled = deployment(false, true).lower().unwrap();
    let federate = FederateIndex::new(1);
    let selected = &compiled.federates()[federate];
    let expected_range = selected.enclaves();
    let expected_enclaves = compiled.enclaves().get_span(expected_range).unwrap();

    let slice = compiled.federate_slice(federate).unwrap();
    assert_eq!(slice.federate(), federate);
    assert_eq!(slice.enclave_range(), expected_range);
    assert!(std::ptr::eq(slice.enclaves(), expected_enclaves));
    assert_eq!(slice.enclaves()[0].id().to_string(), "vehicle/controller");
    assert_eq!(
        slice
            .enclaves()
            .iter()
            .flat_map(|enclave| enclave.required_bindings().iter())
            .map(|binding| match binding {
                RequiredBinding::State {
                    component,
                    implementation,
                    ..
                }
                | RequiredBinding::Reaction {
                    component,
                    implementation,
                    ..
                }
                | RequiredBinding::Port {
                    component,
                    implementation,
                    ..
                }
                | RequiredBinding::Action {
                    component,
                    implementation,
                    ..
                } => (
                    component.to_string(),
                    implementation.to_string(),
                    binding.symbol(),
                ),
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "action_Controller_2fpulse".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "port_Controller_2farray_5fin".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "port_Controller_2farray_5fin".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "port_Controller_2fbank_5fin".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "port_Controller_2fbank_5fin".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "port_Controller_2foutput".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "reaction_Controller_2femit".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "reaction_Controller_2freset_5factive".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "reaction_Controller_2fshutdown".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "reaction_Controller_2fstart".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "reaction_Controller_2ftimer_5ffired".to_owned(),
            ),
            (
                "vehicle/controller".to_owned(),
                "controller-host".to_owned(),
                "state_Controller".to_owned(),
            ),
        ]
    );
    assert!(matches!(
        compiled.federate_slice(FederateIndex::new(2)),
        Err(FederateSliceError::FederateNotFound {
            federate: missing,
        }) if missing == FederateIndex::new(2)
    ));
}

#[test]
fn central_rti_projection_uses_precomputed_dense_dependencies() {
    let compiled = deployment(false, true).lower().unwrap();
    with_central_rti(&compiled, |rti| {
        let edge = FederateIndex::new(0);
        let host = FederateIndex::new(1);
        assert_eq!(
            rti.direct_incoming(edge)
                .iter()
                .map(|dependency| (dependency.source(), dependency.delay_nanos()))
                .collect::<Vec<_>>(),
            [(host, 0)]
        );
        assert_eq!(
            rti.transitive_incoming(edge)
                .iter()
                .map(|dependency| (dependency.source(), dependency.delay_nanos()))
                .collect::<Vec<_>>(),
            [(host, 0)]
        );
        assert_eq!(rti.affected_downstream(host), [edge]);
    });
}

#[test]
fn central_rti_projection_preserves_route_identity_and_delay() {
    let compiled = distributed_with(
        ConnectionSemantics::Logical {
            after: Some(crate::runtime::Duration::milliseconds(5)),
        },
        RecoveryPolicy::FailStop,
        false,
    )
    .lower()
    .unwrap();
    with_central_rti(&compiled, |rti| {
        let route_index = RtiRouteIndex::new(0);
        let route = rti.routes()[route_index];
        assert_eq!(
            rti.route_boundary(route_index).as_str(),
            "controller-to-sensor"
        );
        assert_eq!(route.source(), FederateIndex::new(1));
        assert_eq!(route.target(), FederateIndex::new(0));
        assert_eq!(route.delay_nanos(), 5_000_000);
    });
}

#[test]
fn backend_neutral_federation_remains_authoritative_after_projection() {
    let compiled = deployment(false, true).lower().unwrap();
    let edge = &compiled.federation().edges()[0];
    assert_eq!(edge.id().to_string(), "controller-to-sensor");
    assert_eq!(edge.source().as_str(), "host");
    assert_eq!(edge.target().as_str(), "edge");
    assert_eq!(edge.delay().as_nanos(), 0);
}

#[test]
fn central_rti_projection_preserves_flow_and_physical_boundary_identities() {
    let compiled = distributed_with(
        ConnectionSemantics::Logical { after: None },
        RecoveryPolicy::FailStop,
        false,
    )
    .lower()
    .unwrap();
    with_central_rti(&compiled, |rti| {
        let route = RtiRouteIndex::new(0);
        assert_eq!(rti.route_flow(route), "sensor-control");
        assert_eq!(rti.route_physical_input(route), Some("plant/z"));
        assert_eq!(rti.route_physical_output(route), Some("plant/#g1"));
    });
}

#[test]
fn cross_federate_physical_connection_is_reserved_for_later_slices() {
    let error = distributed_with(
        ConnectionSemantics::Physical { after: None },
        RecoveryPolicy::FailStop,
        false,
    )
    .lower()
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "cross-Federate physical connection 'controller-to-sensor' is unsupported"
    );
}

#[test]
fn one_flow_identity_may_span_parallel_boundary_identities() {
    let compiled = distributed_with(
        ConnectionSemantics::Logical { after: None },
        RecoveryPolicy::FailStop,
        true,
    )
    .lower()
    .unwrap();
    with_central_rti(&compiled, |rti| {
        assert_eq!(rti.flow_count(), 1);
        assert_eq!(
            rti.routes()
                .keys()
                .map(|route| (rti.route_boundary(route).as_str(), rti.route_flow(route)))
                .collect::<Vec<_>>(),
            [
                ("route/%2F", "sensor-control"),
                ("route/-", "sensor-control"),
            ]
        );
    });
}

#[test]
fn central_rti_projection_preserves_typed_policies_and_dense_capability_references() {
    let compiled = deployment(false, true).lower().unwrap();
    with_central_rti(&compiled, |rti| {
        let route = RtiRouteIndex::new(0);
        assert_eq!(
            rti.member_recovery_policy(FederateIndex::new(1)),
            RecoveryPolicy::FailStop
        );
        assert_eq!(
            rti.route_failure_policy(route),
            BoundaryFailurePolicy::PropagateStop
        );
        assert_eq!(
            rti.route_transport_policy(route),
            TransportPolicy::ReliableOrderedFramed
        );
        assert_eq!(rti.route_codec_policy(route), CodecPolicy::CanonicalBounded);
        assert_eq!(rti.route_timing_policy(route), TimingPolicy::BestEffort);
        assert_eq!(rti.route_security_policy(route), SecurityPolicy::None);
        assert_eq!(rti.route_transport_capability(route), "udp");
        assert_eq!(rti.route_codec_capability(route), "postcard");
    });
}

#[test]
fn known_unsupported_policy_selection_fails_closed() {
    let error = distributed_with(
        ConnectionSemantics::Logical { after: None },
        RecoveryPolicy::RestartReset,
        false,
    )
    .lower()
    .unwrap_err();
    assert!(matches!(
        error,
        CompileError::UnsupportedPolicy { category: "recovery", selection }
            if selection == "restart-reset"
    ));
}
#[test]
fn lowering_preserves_canonical_mode_transition_identity() {
    let compiled = local_deployment(
        false,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::ModeTransition,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(1)].mode_effect(),
                Some(crate::runtime::CompiledModeEffectRef {
                    target: ModeIndex::new(1),
                    transition: crate::runtime::TransitionKind::Reset,
                })
            );
        })
        .unwrap();
}
#[test]
fn bounded_lowering_rejects_unbounded_resources() {
    let cases = [
        (
            DescriptorBounds {
                queue_capacity: DescriptorBound::Unknown,
                ..known_bounds()
            },
            "event-queue",
        ),
        (
            DescriptorBounds {
                payload_bytes: DescriptorBound::Unknown,
                ..known_bounds()
            },
            "payload-bytes",
        ),
    ];
    for (bounds, expected) in cases {
        let error = deployment_with_bounds(
            false,
            false,
            false,
            ConnectionSemantics::Logical { after: None },
            [bounds; 2],
            DependencyCase::None,
            RecoveryPolicy::FailStop,
            false,
        )
        .lower()
        .unwrap_err();
        assert!(matches!(
            error,
            CompileError::UnboundedResource { resource, .. } if resource == expected
        ));
    }
}
#[test]
fn bounded_lowering_rejects_queue_and_byte_overflow() {
    let cases = [
        (
            [DescriptorBounds {
                queue_capacity: DescriptorBound::Known(u32::MAX as u64),
                ..known_bounds()
            }; 2],
            "event-queue",
        ),
        (
            [
                DescriptorBounds {
                    payload_bytes: DescriptorBound::Known(u64::MAX),
                    ..known_bounds()
                },
                DescriptorBounds {
                    payload_bytes: DescriptorBound::Known(1),
                    ..known_bounds()
                },
            ],
            "payload-bytes",
        ),
    ];
    for (bounds, expected) in cases {
        let error = deployment_with_bounds(
            false,
            false,
            true,
            ConnectionSemantics::Logical { after: None },
            bounds,
            DependencyCase::None,
            RecoveryPolicy::FailStop,
            false,
        )
        .lower()
        .unwrap_err();
        assert!(matches!(
            error,
            CompileError::ResourceOverflow { resource, .. } if resource == expected
        ));
    }
}

#[test]
fn cross_enclave_connection_lowers_to_paired_scheduler_routes() {
    let compiled = deployment(false, false).lower().unwrap();
    let enclaves = federate_enclaves(&compiled, FederateIndex::new(0));
    enclaves[0]
        .with_view(|source| {
            assert_eq!(source.routes().len(), 1);
            assert_eq!(
                source.routes()[RouteIndex::new(0)].direction(),
                RouteDirection::Outbound
            );
            assert_eq!(
                source.route_boundary_id(RouteIndex::new(0)).as_str(),
                "controller-to-sensor"
            );
        })
        .unwrap();
    enclaves[1]
        .with_view(|target| {
            assert_eq!(target.routes().len(), 1);
            assert_eq!(
                target.routes()[RouteIndex::new(0)].direction(),
                RouteDirection::Inbound
            );
        })
        .unwrap();
}

#[test]
fn same_enclave_zero_delay_connection_collapses_to_one_port_slot() {
    for after in [None, Some(crate::runtime::Duration::ZERO)] {
        let compiled = local_deployment(
            true,
            ConnectionSemantics::Logical { after },
            DependencyCase::None,
        )
        .lower()
        .unwrap();
        federate_enclaves(&compiled, FederateIndex::new(0))[0]
            .with_view(|enclave| {
                assert_eq!(enclave.ports().len(), 5);
                assert!(enclave.routes().is_empty());
            })
            .unwrap();
    }
}
#[test]
fn placement_only_deployment_with_no_enclaves_lowers_to_an_empty_root_image() {
    let mut topology = ApplicationTopologyBuilder::new("placement-only").unwrap();
    topology
        .add_placement_group(PlacementGroupId::new("placement/host").unwrap(), None)
        .unwrap();
    let deployment = ResolvedDeployment::new(
        topology.finish().unwrap(),
        [],
        [PlacementAssignment::new(
            PlacementGroupId::new("placement/host").unwrap(),
            FederateId::new("host").unwrap(),
        )],
        [FederateConfig::new(
            FederateId::new("host").unwrap(),
            TargetTriple::new("x86_64-unknown-linux-gnu").unwrap(),
            RuntimeBackendId::new("native").unwrap(),
            RecoveryPolicy::FailStop,
        )],
        CoordinationSelection::Local,
        [],
    )
    .unwrap();
    let compiled = deployment.lower().unwrap();
    assert!(compiled.federates()[FederateIndex::new(0)]
        .enclaves()
        .is_empty());
    compiled.validate().unwrap();
}

#[test]
fn delayed_same_enclave_connection_lowers_to_a_valid_route_pair() {
    let compiled = local_deployment(
        true,
        ConnectionSemantics::Logical {
            after: Some(crate::runtime::Duration::nanoseconds(2)),
        },
        DependencyCase::None,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(enclave.routes().len(), 2);
            for route in enclave.routes().values() {
                assert_eq!(route.timing_domain(), TimingDomain::Logical);
                assert_eq!(route.delay_nanos(), 2);
            }
        })
        .unwrap();
}
#[test]
fn physical_connection_is_routed_and_breaks_same_tag_dependency() {
    let compiled = local_deployment(
        true,
        ConnectionSemantics::Physical { after: None },
        DependencyCase::None,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(enclave.routes().len(), 2);
            assert!(enclave
                .routes()
                .values()
                .all(|route| route.timing_domain() == TimingDomain::Physical));
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(5)].dependency_level(),
                0
            );
        })
        .unwrap();
}
#[test]
fn same_tag_reaction_dependencies_are_precomputed_before_slicing() {
    let compiled = local_deployment(
        true,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::None,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(enclave.reactions().len(), 6);
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(0)].dependency_level(),
                1
            );
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(3)].dependency_level(),
                0
            );
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(5)].dependency_level(),
                2
            );
            assert_eq!(enclave.required_bindings().len(), 15);
        })
        .unwrap();
}
#[test]
fn modes_actions_lifecycle_and_scopes_are_fully_lowered() {
    let compiled = deployment(false, false).lower().unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(enclave.actions().len(), 4);
            assert_eq!(enclave.modes().len(), 2);
            assert_eq!(enclave.scopes().len(), 3);
            assert_eq!(
                enclave.reactors()[crate::runtime::image::ReactorIndex::new(0)].initial_mode(),
                Some(ModeIndex::new(1))
            );
            assert_eq!(
                enclave.actions()[ActionIndex::new(0)].timing(),
                ActionTiming::Standard {
                    domain: TimingDomain::Logical,
                    min_delay_nanos: 3,
                }
            );
            let standard_action = enclave.actions()[ActionIndex::new(0)];
            assert_eq!(
                enclave.required_bindings()[standard_action.binding().unwrap()].kind(),
                BindingKind::Action
            );
            assert_eq!(enclave.actions()[ActionIndex::new(3)].binding(), None);
            let port_binding = enclave.ports()[crate::runtime::image::PortIndex::new(0)].binding();
            assert_eq!(
                enclave.required_bindings()[port_binding].kind(),
                BindingKind::Port
            );
            assert_eq!(
                enclave.actions()[ActionIndex::new(3)].timing(),
                ActionTiming::Timer {
                    period_nanos: Some(7)
                }
            );
            assert_eq!(enclave.scope_descendants(ScopeIndex::new(0)).len(), 3);
            assert_eq!(
                enclave.scope_reset_reactions(ScopeIndex::new(1))[0].reaction(),
                ReactionIndex::new(1)
            );
            assert_eq!(enclave.startup_actions()[0].action(), ActionIndex::new(2));
            assert_eq!(
                enclave.timer_startup_actions()[0].action(),
                ActionIndex::new(3)
            );
            assert_eq!(enclave.timer_startup_actions()[0].logical_delay_nanos(), 5);
            assert_eq!(
                enclave.shutdown_reactions()[0].action(),
                ActionIndex::new(1)
            );
            assert_eq!(enclave.shutdown_actions(), &[ActionIndex::new(1)]);
            assert_eq!(enclave.storage_bounds().action_slots(), 4);
        })
        .unwrap();
}
#[test]
fn reaction_cycles_report_stable_reaction_identities() {
    let error = local_deployment(
        true,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::ReactionCycle,
    )
    .lower()
    .unwrap_err();
    assert!(matches!(
        error,
        CompileError::ReactionCycle { reactions, .. }
            if reactions.iter().any(|id| id.to_string() == "vehicle/controller/emit")
                && reactions.iter().any(|id| id.to_string() == "vehicle/controller/start")
    ));
}
#[test]
fn a_reaction_triggered_by_its_own_effect_reports_a_cycle() {
    let error = local_deployment(
        true,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::PortSelfCycle,
    )
    .lower()
    .unwrap_err();
    assert!(matches!(
        error,
        CompileError::ReactionCycle { reactions, .. }
            if reactions.as_ref() == [ReactionId::new("vehicle/controller/emit").unwrap()]
    ));
}
#[test]
fn mutually_exclusive_modal_reactions_do_not_create_dependencies() {
    let compiled = local_deployment(
        true,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::MutuallyExclusiveModes,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert_eq!(
                enclave.reactions()[ReactionIndex::new(4)].dependency_level(),
                0
            );
        })
        .unwrap();
}
#[test]
fn dense_reaction_order_uses_encoded_stable_identity_text() {
    let compiled = local_deployment(
        true,
        ConnectionSemantics::Logical { after: None },
        DependencyCase::EncodedOrdering,
    )
    .lower()
    .unwrap();
    federate_enclaves(&compiled, FederateIndex::new(0))[0]
        .with_view(|enclave| {
            assert!(enclave
                .reactions()
                .values()
                .enumerate()
                .all(|(index, reaction)| {
                    reaction.binding().as_u32() == u32::try_from(index).unwrap() + 7
                }));
        })
        .unwrap();
}
#[test]
fn dense_cardinality_overflow_is_reported_before_conversion() {
    #[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
    struct SmallKey(usize);
    impl From<usize> for SmallKey {
        fn from(value: usize) -> Self {
            Self(value)
        }
    }
    impl tinymap::Key for SmallKey {
        const MAX_LEN: usize = 1;

        fn index(&self) -> usize {
            self.0
        }
    }

    let enclave = StableEnclaveId::new("vehicle/controller").unwrap();
    let result =
        tinymap::TinyMap::<SmallKey, _>::try_from_iter(["first", "second"]).map_err(|_| {
            CompileError::ResourceOverflow {
                enclave: enclave.clone(),
                resource: "reactions",
            }
        });
    assert!(matches!(
        result,
        Err(CompileError::ResourceOverflow {
            resource: "reactions",
            ..
        })
    ));
}
