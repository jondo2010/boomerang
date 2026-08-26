use crate::{
    compiler::{
        ActionKind, BankMember, ComponentInstanceId, ConnectionSemantics, PlacementGroupId,
        PortDirection, ReactionId, ReactionRelationTarget, ReactorId, StableEnclaveId,
    },
    runtime, Assembly, ReactorPlacement, TimerSpec, TriggerMode,
};

fn id<T: std::str::FromStr>(value: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    value.parse().unwrap()
}

#[test]
fn assembly_projects_exact_non_modal_structure() {
    let mut assembly = Assembly::new();

    let (root, root_input, root_outputs, logical, physical, named_reaction) = {
        let mut root = assembly.add_reactor("root/%#[x]", None, None, (), ReactorPlacement::Local);
        let root_input = root.add_input_port::<u32>("in/[#]").unwrap();
        let root_outputs = root.add_output_bank::<u32>("out/%]", 2).unwrap();
        let logical = root
            .add_logical_action::<u32>("logical/#", Some(runtime::Duration::milliseconds(1)))
            .unwrap();
        let physical = root.add_physical_action::<u32>("physical/%", None).unwrap();
        root.add_timer(
            "timer/[]",
            TimerSpec::default()
                .with_offset(runtime::Duration::milliseconds(2))
                .with_period(runtime::Duration::milliseconds(3)),
        )
        .unwrap();
        root.add_timer("timer-at-start", TimerSpec::default())
            .unwrap();
        let named_reaction = root
            .add_reaction(Some("step/#"))
            .with_trigger(logical)
            .with_use(root_input)
            .with_effect(root_outputs.get(0).unwrap())
            .with_reaction_fn(|_ctx, _state, (_logical, _input, _output)| {})
            .finish()
            .unwrap();
        root.add_reaction(Some("step/#"))
            .with_startup_trigger()
            .with_reaction_fn(|_ctx, _state, (_startup,)| {})
            .finish()
            .unwrap();
        root.add_reaction(None)
            .with_startup_trigger()
            .with_reaction_fn(|_ctx, _state, (_startup,)| {})
            .finish()
            .unwrap();
        root.add_reaction(None)
            .with_startup_trigger()
            .with_reaction_fn(|_ctx, _state, (_startup,)| {})
            .finish()
            .unwrap();
        (
            root.finish().unwrap(),
            root_input,
            root_outputs,
            logical,
            physical,
            named_reaction,
        )
    };

    let (first_enclave, enclave_input) = {
        let mut child =
            assembly.add_reactor("enc/#[]", Some(root), None, (), ReactorPlacement::Enclave);
        let input = child.add_input_port::<u32>("sink/%[]").unwrap();
        (child.finish().unwrap(), input)
    };
    let nested_enclave = assembly
        .add_reactor(
            "nested%/[]",
            Some(first_enclave),
            None,
            (),
            ReactorPlacement::Enclave,
        )
        .finish()
        .unwrap();
    let nested_local = assembly
        .add_reactor(
            "leaf/#%[]",
            Some(nested_enclave),
            None,
            (),
            ReactorPlacement::Local,
        )
        .finish()
        .unwrap();

    let mut banked_reactors = Vec::new();
    let mut banked_ports = Vec::new();
    for index in 0..2 {
        let mut banked = assembly.add_reactor(
            "bank/#[]",
            Some(root),
            Some(runtime::BankInfo {
                idx: index,
                total: 2,
            }),
            (),
            ReactorPlacement::Local,
        );
        banked_ports.extend(
            banked
                .add_input_bank::<u32>("member/%[]", 2)
                .unwrap()
                .iter(),
        );
        banked_reactors.push(banked.finish().unwrap());
    }

    assembly
        .add_port_connection::<u32, _, _>(root_input, enclave_input, None, false)
        .unwrap();
    assembly
        .add_port_connection::<u32, _, _>(
            root_input,
            enclave_input,
            Some(runtime::Duration::milliseconds(4)),
            false,
        )
        .unwrap();
    assembly
        .add_port_connection::<u32, _, _>(root_input, enclave_input, None, true)
        .unwrap();
    assembly
        .add_port_connection::<u32, _, _>(
            root_input,
            enclave_input,
            Some(runtime::Duration::milliseconds(5)),
            true,
        )
        .unwrap();

    assembly.reaction_specs[named_reaction]
        .record_action_relation(physical.into(), TriggerMode::TriggersOnly);
    assembly.reaction_specs[named_reaction].record_port_relation(
        root_outputs.get(1).unwrap().into(),
        TriggerMode::TriggersAndEffects,
    );

    let topology = assembly.application_topology().unwrap();
    assert_eq!(topology, assembly.application_topology().unwrap());

    let root_id: ReactorId = id("root%2F%25%23%5Bx%5D");
    let component_id: ComponentInstanceId = id("component/root%252F%2525%2523%255Bx%255D");
    let root_group: PlacementGroupId = id("placement/root%252F%2525%2523%255Bx%255D");
    assert_eq!(
        topology.application_id().as_str(),
        "application/root%252F%2525%2523%255Bx%255D"
    );
    assert_eq!(
        topology
            .components()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>(),
        vec![component_id.to_string()]
    );

    let reactor_ids = topology
        .reactors()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        reactor_ids,
        vec![
            "root%2F%25%23%5Bx%5D",
            "root%2F%25%23%5Bx%5D/bank%2F%23%5B%5D/#b0",
            "root%2F%25%23%5Bx%5D/bank%2F%23%5B%5D/#b1",
            "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D",
            "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D/nested%25%2F%5B%5D",
            "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D/nested%25%2F%5B%5D/leaf%2F%23%25%5B%5D",
        ]
    );
    for (_, reactor) in topology.reactors() {
        assert_eq!(reactor.component(), &component_id);
        assert!(reactor.scope_mode().is_none());
    }
    let root_reactor = topology.reactor(&root_id).unwrap();
    assert_eq!(root_reactor.enclave().to_string(), root_id.to_string());
    assert_eq!(root_reactor.placement_group(), Some(&root_group));
    let bank0: ReactorId = id("root%2F%25%23%5Bx%5D/bank%2F%23%5B%5D/#b0");
    assert_eq!(
        topology.reactor(&bank0).unwrap().bank(),
        Some(BankMember::new(0, 2).unwrap())
    );
    assert_eq!(
        topology.reactor(&bank0).unwrap().enclave(),
        &id::<StableEnclaveId>("root%2F%25%23%5Bx%5D")
    );
    assert_eq!(topology.reactor(&bank0).unwrap().parent(), Some(&root_id));
    assert_eq!(
        topology.reactor(&bank0).unwrap().placement_group(),
        Some(&root_group)
    );

    let first_enclave_id: ReactorId = id("root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D");
    let nested_id: ReactorId = id("root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D/nested%25%2F%5B%5D");
    let leaf_id: ReactorId =
        id("root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D/nested%25%2F%5B%5D/leaf%2F%23%25%5B%5D");
    let first_group: PlacementGroupId =
        id("placement/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D");
    let nested_group: PlacementGroupId = id(
        "placement/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D%2Fnested%2525%252F%255B%255D",
    );
    assert_eq!(
        topology
            .reactor(&first_enclave_id)
            .unwrap()
            .enclave()
            .to_string(),
        first_enclave_id.to_string()
    );
    assert_eq!(
        topology
            .reactor(&first_enclave_id)
            .unwrap()
            .placement_group(),
        Some(&first_group)
    );
    assert_eq!(
        topology.reactor(&nested_id).unwrap().parent(),
        Some(&first_enclave_id)
    );
    assert_eq!(
        topology.reactor(&leaf_id).unwrap().placement_group(),
        Some(&nested_group)
    );
    assert_eq!(
        topology.reactor(&leaf_id).unwrap().enclave().to_string(),
        nested_id.to_string()
    );

    assert_eq!(
        topology
            .enclaves()
            .map(|(id, enclave)| (id.to_string(), enclave.root().to_string()))
            .collect::<Vec<_>>(),
        vec![
            (root_id.to_string(), root_id.to_string()),
            (
                "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D".into(),
                "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D".into(),
            ),
            (nested_id.to_string(), nested_id.to_string()),
        ]
    );
    assert_eq!(topology.placement_groups().count(), 3);
    assert!(topology
        .placement_groups()
        .all(|(_, group)| group.parent().is_none()));

    let root_actions = topology
        .actions()
        .filter(|(_, action)| action.reactor() == &root_id)
        .map(|(id, action)| (id.to_string(), action.kind(), action.declaration_position()))
        .collect::<Vec<_>>();
    assert_eq!(
        root_actions,
        vec![
            (
                "root%2F%25%23%5Bx%5D/__shutdown".into(),
                ActionKind::Shutdown,
                1
            ),
            (
                "root%2F%25%23%5Bx%5D/__startup".into(),
                ActionKind::Startup,
                0
            ),
            (
                "root%2F%25%23%5Bx%5D/logical%2F%23".into(),
                ActionKind::Logical {
                    minimum_delay: Some(runtime::Duration::milliseconds(1)),
                },
                2,
            ),
            (
                "root%2F%25%23%5Bx%5D/physical%2F%25".into(),
                ActionKind::Physical {
                    minimum_delay: None
                },
                3,
            ),
            (
                "root%2F%25%23%5Bx%5D/timer-at-start".into(),
                ActionKind::Timer {
                    offset: None,
                    period: None,
                },
                5,
            ),
            (
                "root%2F%25%23%5Bx%5D/timer%2F%5B%5D".into(),
                ActionKind::Timer {
                    offset: Some(runtime::Duration::milliseconds(2)),
                    period: Some(runtime::Duration::milliseconds(3)),
                },
                4,
            ),
        ]
    );

    let root_ports = topology
        .ports()
        .filter(|(_, port)| port.reactor() == &root_id)
        .map(|(id, port)| {
            (
                id.to_string(),
                port.direction(),
                port.bank(),
                port.declaration_position(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        root_ports,
        vec![
            (
                "root%2F%25%23%5Bx%5D/in%2F%5B%23%5D".into(),
                PortDirection::Input,
                None,
                0
            ),
            (
                "root%2F%25%23%5Bx%5D/out%2F%25%5D/#b0".into(),
                PortDirection::Output,
                Some(BankMember::new(0, 2).unwrap()),
                1,
            ),
            (
                "root%2F%25%23%5Bx%5D/out%2F%25%5D/#b1".into(),
                PortDirection::Output,
                Some(BankMember::new(1, 2).unwrap()),
                2,
            ),
        ]
    );
    let banked_port = topology
        .ports()
        .find(|(_, port)| port.reactor() == &bank0)
        .unwrap()
        .1;
    assert_eq!(banked_port.bank(), Some(BankMember::new(0, 2).unwrap()));
    assert_eq!(banked_port.declaration_position(), 0);

    let named_id: ReactionId = id("root%2F%25%23%5Bx%5D/step%2F%23/#g0");
    let named = topology.reaction(&named_id).unwrap();
    assert!(named.options().mode.is_none());
    let relations = named
        .relations()
        .iter()
        .map(|relation| {
            let target = match relation.target() {
                ReactionRelationTarget::Action(id) => format!("action:{id}"),
                ReactionRelationTarget::Port(id) => format!("port:{id}"),
            };
            let flags = relation.flags();
            (
                target,
                flags.is_trigger(),
                flags.is_use(),
                flags.is_effect(),
                relation.declaration_position(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        relations,
        vec![
            (
                "action:root%2F%25%23%5Bx%5D/logical%2F%23".into(),
                true,
                true,
                false,
                0
            ),
            (
                "action:root%2F%25%23%5Bx%5D/physical%2F%25".into(),
                true,
                false,
                false,
                1
            ),
            (
                "port:root%2F%25%23%5Bx%5D/in%2F%5B%23%5D".into(),
                false,
                true,
                false,
                0
            ),
            (
                "port:root%2F%25%23%5Bx%5D/out%2F%25%5D/#b0".into(),
                false,
                false,
                true,
                1
            ),
            (
                "port:root%2F%25%23%5Bx%5D/out%2F%25%5D/#b1".into(),
                true,
                false,
                true,
                2
            ),
        ]
    );
    assert!(topology
        .reaction(&id::<ReactionId>("root%2F%25%23%5Bx%5D/#g0"))
        .is_some());
    assert!(topology
        .reaction(&id::<ReactionId>("root%2F%25%23%5Bx%5D/#g1"))
        .is_some());
    assert!(topology
        .reaction(&id::<ReactionId>("root%2F%25%23%5Bx%5D/step%2F%23/#g1"))
        .is_some());

    let source = "root%2F%25%23%5Bx%5D/in%2F%5B%23%5D";
    let target = "root%2F%25%23%5Bx%5D/enc%2F%23%5B%5D/sink%2F%25%5B%5D";
    let connections = topology
        .connections()
        .map(|(id, connection)| {
            (
                id.to_string(),
                connection.source().to_string(),
                connection.target().to_string(),
                connection.semantics(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        connections,
        vec![
            (
                "boundary/root%252F%2525%2523%255Bx%255D%2Fin%252F%255B%2523%255D/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D%2Fsink%252F%2525%255B%255D/c0".into(),
                source.into(), target.into(), ConnectionSemantics::Logical { after: None },
            ),
            (
                "boundary/root%252F%2525%2523%255Bx%255D%2Fin%252F%255B%2523%255D/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D%2Fsink%252F%2525%255B%255D/c1".into(),
                source.into(), target.into(), ConnectionSemantics::Logical { after: Some(runtime::Duration::milliseconds(4)) },
            ),
            (
                "boundary/root%252F%2525%2523%255Bx%255D%2Fin%252F%255B%2523%255D/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D%2Fsink%252F%2525%255B%255D/c2".into(),
                source.into(), target.into(), ConnectionSemantics::Physical { after: None },
            ),
            (
                "boundary/root%252F%2525%2523%255Bx%255D%2Fin%252F%255B%2523%255D/root%252F%2525%2523%255Bx%255D%2Fenc%252F%2523%255B%255D%2Fsink%252F%2525%255B%255D/c3".into(),
                source.into(), target.into(), ConnectionSemantics::Physical { after: Some(runtime::Duration::milliseconds(5)) },
            ),
        ]
    );
    assert_eq!(topology.modes().count(), 0);

    let _ = (
        root_input,
        logical,
        physical,
        nested_local,
        banked_reactors,
        banked_ports,
    );
}

#[test]
fn application_topology_projection_does_not_consume_runtime_assembly() {
    let mut assembly = Assembly::new();
    assembly
        .add_reactor("root", None, None, (), ReactorPlacement::Local)
        .finish()
        .unwrap();

    assert_eq!(
        assembly.application_topology().unwrap(),
        assembly.application_topology().unwrap()
    );
    let lowered = assembly
        .into_runtime_assembly(&runtime::Config::default())
        .unwrap();
    assert_eq!(lowered.enclaves.len(), 1);
}

#[test]
fn local_top_level_roots_share_the_first_implicit_partition() {
    let mut assembly = Assembly::new();
    assembly
        .add_reactor("z-root", None, None, (), ReactorPlacement::Local)
        .finish()
        .unwrap();
    assembly
        .add_reactor("a-root", None, None, (), ReactorPlacement::Local)
        .finish()
        .unwrap();

    let topology = assembly.application_topology().unwrap();
    let a: ReactorId = id("a-root");
    let z: ReactorId = id("z-root");
    let z_enclave: StableEnclaveId = id("z-root");
    let z_group: PlacementGroupId = id("placement/z-root");
    assert_eq!(
        topology.application_id().as_str(),
        "application/a-root/z-root"
    );
    assert_eq!(topology.components().count(), 2);
    assert_ne!(
        topology.reactor(&a).unwrap().component(),
        topology.reactor(&z).unwrap().component()
    );
    for reactor in [topology.reactor(&a).unwrap(), topology.reactor(&z).unwrap()] {
        assert_eq!(reactor.enclave(), &z_enclave);
        assert_eq!(reactor.placement_group(), Some(&z_group));
    }
    assert_eq!(topology.enclaves().count(), 1);
}

#[test]
fn hierarchical_assembly_connections_project_exact_legal_shapes() {
    let mut assembly = Assembly::new();
    let (root, root_input, root_output) = {
        let mut reactor = assembly.add_reactor("root", None, None, (), ReactorPlacement::Local);
        let input = reactor.add_input_port::<u32>("input").unwrap();
        let output = reactor.add_output_port::<u32>("output").unwrap();
        (reactor.finish().unwrap(), input, output)
    };
    let (left_output, left_input) = {
        let mut reactor =
            assembly.add_reactor("left", Some(root), None, (), ReactorPlacement::Local);
        let input = reactor.add_input_port::<u32>("input").unwrap();
        let output = reactor.add_output_port::<u32>("output").unwrap();
        reactor.finish().unwrap();
        (output, input)
    };
    let right_input = {
        let mut reactor =
            assembly.add_reactor("right", Some(root), None, (), ReactorPlacement::Local);
        let input = reactor.add_input_port::<u32>("input").unwrap();
        reactor.finish().unwrap();
        input
    };
    assembly
        .add_port_connection::<u32, _, _>(root_input, left_input, None, false)
        .unwrap();
    assembly
        .add_port_connection::<u32, _, _>(left_output, root_output, None, false)
        .unwrap();
    assembly
        .add_port_connection::<u32, _, _>(left_output, right_input, None, false)
        .unwrap();

    let topology = assembly.application_topology().unwrap();
    let endpoints = topology
        .connections()
        .map(|(_, connection)| {
            (
                connection.source().to_string(),
                connection.target().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        endpoints,
        vec![
            ("root/input".into(), "root/left/input".into()),
            ("root/left/output".into(), "root/output".into()),
            ("root/left/output".into(), "root/right/input".into()),
        ]
    );
}
