use crate::{
    compiler::{
        ActionId, ActionKind, ApplicationTopology, ApplicationTopologyBuilder, BankMember,
        BoundaryId, ComponentInstance, ComponentInstanceId, ConnectionSemantics, ContractId,
        ModeId, ModeTransition, ModeTransitionKind, PlacementGroupId, PortDirection, PortId,
        ReactionId, ReactionOptions, ReactionRelation, ReactionRelationFlags,
        ReactionRelationTarget, Reactor as TopologyReactor, ReactorId, StableEnclaveId,
    },
    runtime, Assembly, ModeKind, ReactionSlotId, ReactorPlacement, TimerSpec, TriggerMode,
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
fn assembly_projects_exact_modal_structure() {
    fn build(reverse_modes: bool) -> crate::compiler::ApplicationTopology {
        let mut assembly = Assembly::new();

        let (root, idle, active) = {
            let mut root = assembly.add_reactor("root", None, None, (), ReactorPlacement::Local);
            let tick = root.add_logical_action::<()>("tick", None).unwrap();
            let (idle, active) = if reverse_modes {
                let active = root.add_mode("active", ModeKind::Normal).unwrap();
                let idle = root.add_mode("idle", ModeKind::Initial).unwrap();
                (idle, active)
            } else {
                let idle = root.add_mode("idle", ModeKind::Initial).unwrap();
                let active = root.add_mode("active", ModeKind::Normal).unwrap();
                (idle, active)
            };

            let enter_active = root.reset_mode_effect(active).unwrap();
            root.add_reaction(Some("enter-active"))
                .with_trigger(tick)
                .with_effect(enter_active)
                .with_reaction_fn(|_ctx, _state, (_tick, _active)| {})
                .finish()
                .unwrap();

            let return_to_idle = root.history_mode_effect(idle).unwrap();
            root.in_mode(active, |ctx| {
                ctx.add_logical_action::<()>("active-action", None)?;
                ctx.add_reaction(Some("active-reset"))
                    .with_reset_trigger()
                    .with_effect(return_to_idle)
                    .with_reaction_fn(|_ctx, _state, (_idle,)| {})
                    .finish()
            })
            .unwrap();

            (root.finish().unwrap(), idle, active)
        };

        let (child, sleeping, running) = {
            let mut child =
                assembly.add_reactor("child", Some(root), None, (), ReactorPlacement::Local);
            child.set_scope_mode(active).unwrap();
            let tick = child.add_logical_action::<()>("tick", None).unwrap();
            let (sleeping, running) = if reverse_modes {
                let running = child.add_mode("running", ModeKind::Normal).unwrap();
                let sleeping = child.add_mode("sleeping", ModeKind::Initial).unwrap();
                (sleeping, running)
            } else {
                let sleeping = child.add_mode("sleeping", ModeKind::Initial).unwrap();
                let running = child.add_mode("running", ModeKind::Normal).unwrap();
                (sleeping, running)
            };

            child
                .add_reaction(Some("inherited"))
                .with_trigger(tick)
                .with_reaction_fn(|_ctx, _state, (_tick,)| {})
                .finish()
                .unwrap();
            child
                .in_mode(running, |ctx| {
                    ctx.add_reaction(Some("running-reset"))
                        .with_reset_trigger()
                        .with_reaction_fn(|_ctx, _state, ()| {})
                        .finish()
                })
                .unwrap();

            (child.finish().unwrap(), sleeping, running)
        };

        let grandchild = {
            let mut grandchild =
                assembly.add_reactor("grandchild", Some(child), None, (), ReactorPlacement::Local);
            grandchild.set_scope_mode(running).unwrap();
            grandchild
                .add_reaction(Some("doubly-inherited"))
                .with_startup_trigger()
                .with_reaction_fn(|_ctx, _state, (_startup,)| {})
                .finish()
                .unwrap();
            grandchild.finish().unwrap()
        };

        let topology = assembly.application_topology().unwrap();
        let _ = (idle, sleeping, grandchild);
        topology
    }

    let topology = build(false);
    assert_eq!(topology, build(true));

    let root: ReactorId = id("root");
    let child: ReactorId = id("root/child");
    let grandchild: ReactorId = id("root/child/grandchild");
    let active: ModeId = id("root/active");
    let idle: ModeId = id("root/idle");
    let running: ModeId = id("root/child/running");
    let sleeping: ModeId = id("root/child/sleeping");

    assert_eq!(
        topology
            .modes()
            .map(|(id, mode)| (
                id.clone(),
                mode.reactor().clone(),
                mode.parent().cloned(),
                mode.is_initial(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (active.clone(), root.clone(), None, false),
            (running.clone(), child.clone(), None, false),
            (sleeping.clone(), child.clone(), None, true),
            (idle.clone(), root.clone(), None, true),
        ]
    );
    assert_eq!(topology.reactor(&root).unwrap().scope_mode(), None);
    assert_eq!(
        topology.reactor(&child).unwrap().scope_mode(),
        Some(&active)
    );
    assert_eq!(
        topology.reactor(&grandchild).unwrap().scope_mode(),
        Some(&running)
    );
    assert_eq!(
        topology
            .action(&id::<ActionId>("root/active-action"))
            .unwrap()
            .mode(),
        Some(&active)
    );

    let enter_active = topology
        .reaction(&id::<ReactionId>("root/enter-active/#g0"))
        .unwrap();
    assert_eq!(enter_active.options().mode(), None);
    assert!(enter_active.options().enabled_modes().is_empty());
    assert!(enter_active.options().reset_modes().is_empty());
    let transition = enter_active.options().transition().unwrap();
    assert_eq!(transition.target(), &active);
    assert_eq!(transition.kind(), ModeTransitionKind::Reset);

    let active_reset = topology
        .reaction(&id::<ReactionId>("root/active-reset/#g0"))
        .unwrap();
    assert_eq!(active_reset.options().mode(), Some(&active));
    assert_eq!(
        active_reset.options().enabled_modes(),
        std::slice::from_ref(&active)
    );
    assert_eq!(
        active_reset.options().reset_modes(),
        std::slice::from_ref(&active)
    );
    let transition = active_reset.options().transition().unwrap();
    assert_eq!(transition.target(), &idle);
    assert_eq!(transition.kind(), ModeTransitionKind::History);

    let inherited = topology
        .reaction(&id::<ReactionId>("root/child/inherited/#g0"))
        .unwrap();
    assert_eq!(inherited.options().mode(), None);
    assert!(inherited.options().enabled_modes().is_empty());
    assert!(inherited.options().reset_modes().is_empty());
    assert!(inherited.options().transition().is_none());

    let running_reset = topology
        .reaction(&id::<ReactionId>("root/child/running-reset/#g0"))
        .unwrap();
    assert_eq!(running_reset.options().mode(), Some(&running));
    assert_eq!(
        running_reset.options().enabled_modes(),
        std::slice::from_ref(&running)
    );
    assert_eq!(running_reset.options().reset_modes(), &[running]);
    assert!(running_reset.options().transition().is_none());

    let doubly_inherited = topology
        .reaction(&id::<ReactionId>(
            "root/child/grandchild/doubly-inherited/#g0",
        ))
        .unwrap();
    assert_eq!(doubly_inherited.options().mode(), None);
    assert!(doubly_inherited.options().enabled_modes().is_empty());
    assert!(doubly_inherited.options().reset_modes().is_empty());
}

#[test]
fn assembly_modal_projection_rejects_invalid_structure() {
    fn error(assembly: &Assembly) -> String {
        assembly.application_topology().unwrap_err().to_string()
    }

    let mut missing = Assembly::new();
    let (missing_mode, missing_reaction) = {
        let mut root = missing.add_reactor("root", None, None, (), ReactorPlacement::Local);
        let mode = root.add_mode("active", ModeKind::Initial).unwrap();
        let reaction = root
            .in_mode(mode, |ctx| {
                ctx.add_reaction(Some("scoped"))
                    .with_reset_trigger()
                    .with_reaction_fn(|_ctx, _state, ()| {})
                    .finish()
            })
            .unwrap();
        root.finish().unwrap();
        (mode, reaction)
    };
    missing.mode_specs.remove(missing_mode);
    let message = error(&missing);
    assert!(message.contains("root/scoped/#g0"), "{message}");
    assert!(message.contains("missing mode"), "{message}");
    let _ = missing_reaction;

    let mut wrong_owner = Assembly::new();
    let (root_reaction, foreign_mode) = {
        let mut root = wrong_owner.add_reactor("root", None, None, (), ReactorPlacement::Local);
        let tick = root.add_logical_action::<()>("tick", None).unwrap();
        let reaction = root
            .add_reaction(Some("wrong-owner"))
            .with_trigger(tick)
            .with_reaction_fn(|_ctx, _state, (_tick,)| {})
            .finish()
            .unwrap();
        root.finish().unwrap();

        let mut foreign =
            wrong_owner.add_reactor("foreign", None, None, (), ReactorPlacement::Local);
        let mode = foreign.add_mode("active", ModeKind::Initial).unwrap();
        foreign.finish().unwrap();
        (reaction, mode)
    };
    wrong_owner.reaction_specs[root_reaction].enabled_modes = Some(vec![foreign_mode]);
    let message = error(&wrong_owner);
    assert!(message.contains("root/wrong-owner/#g0"), "{message}");
    assert!(
        message.contains("belongs to reactor 'foreign'"),
        "{message}"
    );

    let mut invalid_ancestry = Assembly::new();
    let root = invalid_ancestry
        .add_reactor("root", None, None, (), ReactorPlacement::Local)
        .finish()
        .unwrap();
    let (child, child_mode) = {
        let mut child =
            invalid_ancestry.add_reactor("child", Some(root), None, (), ReactorPlacement::Local);
        let mode = child.add_mode("local", ModeKind::Initial).unwrap();
        (child.finish().unwrap(), mode)
    };
    invalid_ancestry.reactor_specs[child].scope_mode = Some(child_mode);
    let message = error(&invalid_ancestry);
    assert!(message.contains("root/child"), "{message}");
    assert!(message.contains("expected reactor 'root'"), "{message}");

    let mut duplicate = Assembly::new();
    let duplicate_mode = {
        let mut root = duplicate.add_reactor("root", None, None, (), ReactorPlacement::Local);
        root.add_mode("active", ModeKind::Initial).unwrap();
        let duplicate = root.add_mode("idle", ModeKind::Normal).unwrap();
        root.finish().unwrap();
        duplicate
    };
    duplicate.mode_specs[duplicate_mode].name = "active".to_owned();
    let message = error(&duplicate);
    assert!(message.contains("root/active"), "{message}");
    assert!(
        message.contains("duplicate stable mode identity"),
        "{message}"
    );

    let mut inconsistent = Assembly::new();
    {
        let mut root = inconsistent.add_reactor("root", None, None, (), ReactorPlacement::Local);
        let tick = root.add_logical_action::<()>("tick", None).unwrap();
        let idle = root.add_mode("idle", ModeKind::Initial).unwrap();
        let active = root.add_mode("active", ModeKind::Normal).unwrap();
        let enter_active = root.reset_mode_effect(active).unwrap();
        let enter_idle = root.history_mode_effect(idle).unwrap();
        root.add_reaction(Some("ambiguous"))
            .with_trigger(tick)
            .with_effect(enter_active)
            .with_effect(enter_idle)
            .with_reaction_fn(|_ctx, _state, (_tick, _active, _idle)| {})
            .finish()
            .unwrap();
        root.finish().unwrap();
    }
    let message = error(&inconsistent);
    assert!(message.contains("root/ambiguous/#g0"), "{message}");
    assert!(message.contains("inconsistent transition"), "{message}");
}

#[test]
fn application_topology_projection_preserves_component_metadata_without_consuming_assembly() {
    let mut assembly = Assembly::new();
    let mut root = assembly.add_reactor("root", None, None, (), ReactorPlacement::Local);
    root.set_component_contract(ContractId::new("root.contract").unwrap(), 7);
    root.add_reaction(Some("start"))
        .with_descriptor_slot(ReactionSlotId::new("Root/start").unwrap())
        .with_startup_trigger()
        .with_reaction_fn(|_ctx, _state, (_startup,)| {})
        .finish()
        .unwrap();
    root.finish().unwrap();

    let topology = assembly.application_topology().unwrap();
    let (component_id, component) = topology.components().next().unwrap();
    assert_eq!(component_id.to_string(), "root");
    assert_eq!(component.contract().as_str(), "root.contract");
    assert_eq!(component.contract_version(), 7);
    assert_eq!(
        topology.reactions().next().unwrap().0.to_string(),
        "root/start"
    );
    assert_eq!(topology, assembly.application_topology().unwrap());
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

#[test]
fn application_topology_debug_uses_stable_structure() {
    fn build(reverse: bool) -> ApplicationTopology {
        let component: ComponentInstanceId = id("component/root");
        let root: ReactorId = id("root");
        let workers_0: ReactorId = id("root/workers/#b0");
        let workers_1: ReactorId = id("root/workers/#b1");
        let enclave: StableEnclaveId = id("root");
        let placement: PlacementGroupId = id("placement/root");
        let alternate_placement: PlacementGroupId = id("placement/root/alternate");
        let active: ModeId = id("root/active");
        let tick: ActionId = id("root/tick");
        let output: PortId = id("root/out");
        let input: PortId = id("root/in");
        let reaction: ReactionId = id("root/step");

        let mut topology = ApplicationTopologyBuilder::new("inspection").unwrap();
        topology
            .add_component(ComponentInstance::new("component/root", "root.v1", 1).unwrap())
            .unwrap();
        topology
            .add_placement_group(placement.clone(), None)
            .unwrap();
        topology
            .add_placement_group(alternate_placement.clone(), Some(placement.clone()))
            .unwrap();
        topology.add_enclave(enclave.clone(), root.clone()).unwrap();

        let root_reactor = TopologyReactor::new(
            root.clone(),
            component.clone(),
            None,
            None,
            enclave.clone(),
            Some(placement.clone()),
            None,
        );
        let worker = |id, index, placement_group| {
            TopologyReactor::new(
                id,
                component.clone(),
                Some(root.clone()),
                Some(BankMember::new(index, 2).unwrap()),
                enclave.clone(),
                Some(placement_group),
                None,
            )
        };
        let reactors = if reverse {
            vec![
                worker(workers_1.clone(), 1, alternate_placement.clone()),
                worker(workers_0.clone(), 0, placement.clone()),
                root_reactor,
            ]
        } else {
            vec![
                root_reactor,
                worker(workers_0.clone(), 0, placement.clone()),
                worker(workers_1.clone(), 1, alternate_placement.clone()),
            ]
        };
        for reactor in reactors {
            topology.add_reactor(reactor).unwrap();
        }

        topology
            .add_mode(active.clone(), root.clone(), None, true)
            .unwrap();
        topology
            .add_action(
                tick.clone(),
                root.clone(),
                ActionKind::Logical {
                    minimum_delay: None,
                },
                0,
                Some(active.clone()),
            )
            .unwrap();
        topology
            .add_port(
                output.clone(),
                root.clone(),
                PortDirection::Output,
                None,
                0,
                Some(active.clone()),
            )
            .unwrap();
        topology
            .add_port(
                input.clone(),
                root.clone(),
                PortDirection::Input,
                None,
                1,
                Some(active.clone()),
            )
            .unwrap();
        let mut relations = vec![
            ReactionRelation::new(
                ReactionRelationTarget::Action(tick.clone()),
                ReactionRelationFlags::TRIGGER,
                0,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Port(output.clone()),
                ReactionRelationFlags::EFFECT,
                0,
            ),
            ReactionRelation::new(
                ReactionRelationTarget::Port(input.clone()),
                ReactionRelationFlags::USE,
                1,
            ),
        ];
        if reverse {
            relations.reverse();
        }
        topology
            .add_reaction(
                reaction,
                root.clone(),
                relations,
                ReactionOptions {
                    mode: Some(active.clone()),
                    enabled_modes: vec![active.clone()],
                    reset_modes: vec![active.clone()],
                    transition: Some(ModeTransition::new(active, ModeTransitionKind::Reset)),
                },
            )
            .unwrap();
        topology
            .add_connection(
                id::<BoundaryId>("connection/root-loop"),
                output,
                input,
                ConnectionSemantics::Logical { after: None },
            )
            .unwrap();
        topology.finish().unwrap()
    }

    let topology = build(false);
    let reordered = build(true);
    let formatted = format!("{topology:#?}");
    assert_eq!(formatted, format!("{reordered:#?}"));
    for expected in [
        "root/workers/#b0",
        "root/workers/#b1",
        "root/active",
        "root/step",
        "root/tick",
        "root/out",
        "root/in",
        "connection/root-loop",
        "enclaves",
        "placement_groups",
    ] {
        assert!(
            formatted.contains(expected),
            "missing {expected:?} from {formatted}"
        );
    }
    assert!(!formatted.contains("KeyData"));
    assert!(!formatted.contains("AssemblyReactorKey"));
    assert_eq!(formatted.matches("placement/root/alternate").count(), 2);

    let compact = formatted.split_whitespace().collect::<String>();
    for relation in [
        r#"ReactionRelation{target:"action:root/tick",flags:["trigger",],"#,
        r#"ReactionRelation{target:"port:root/out",flags:["effect",],"#,
        r#"ReactionRelation{target:"port:root/in",flags:["use",],"#,
    ] {
        assert!(
            compact.contains(relation),
            "missing relation {relation} from {compact}"
        );
    }
    assert!(compact.contains(r#"Connection{source:"root/out",target:"root/in",semantics:"#));
    assert!(
        compact.contains(r#"modes:{"root/active":Mode{reactor:"root",parent:None,initial:true,"#)
    );
    assert!(compact.contains(r#"transition:Some(("root/active",Reset,),)"#));

    let reactor_groups = topology.reactors_debug_grouped();
    assert_eq!(reactor_groups.len(), 2);
    assert_eq!(reactor_groups[1].0.to_string(), "root/workers/#b0");
    assert_eq!(
        reactor_groups[1].1.map(ToString::to_string),
        Some("root/workers/#b1".into())
    );
    let graph = topology.build_reactor_graph_grouped();
    assert_eq!(graph.node_count(), 2);
    assert!(graph.contains_edge(&id::<ReactorId>("root"), reactor_groups[1].0));

    let mut assembly = Assembly::new();
    assembly
        .add_reactor("root", None, None, (), ReactorPlacement::Local)
        .finish()
        .unwrap();
    let projected = assembly.application_topology().unwrap();
    assert_eq!(format!("{assembly:#?}"), format!("{projected:#?}"));
    assert!(format!("{:?}", Assembly::new()).contains("ApplicationTopologyProjectionError"));
}

fn singleton_bank_topology() -> (ApplicationTopology, ReactorId, PortId) {
    let component: ComponentInstanceId = id("component/root");
    let reactor: ReactorId = id("root/#b0");
    let enclave: StableEnclaveId = id("root/#b0");
    let port: PortId = id("root/#b0/channel/#b0");
    let singleton = BankMember::new(0, 1).unwrap();

    let mut builder = ApplicationTopologyBuilder::new("singleton-bank").unwrap();
    builder
        .add_component(ComponentInstance::new("component/root", "root.v1", 1).unwrap())
        .unwrap();
    builder
        .add_enclave(enclave.clone(), reactor.clone())
        .unwrap();
    builder
        .add_reactor(TopologyReactor::new(
            reactor.clone(),
            component,
            None,
            Some(singleton),
            enclave,
            None,
            None,
        ))
        .unwrap();
    builder
        .add_port(
            port.clone(),
            reactor.clone(),
            PortDirection::Input,
            Some(singleton),
            0,
            None,
        )
        .unwrap();
    (builder.finish().unwrap(), reactor, port)
}

#[test]
fn application_topology_debug_preserves_singleton_reactor_bank() {
    let (topology, reactor, _) = singleton_bank_topology();
    assert_eq!(
        topology.reactors_debug_grouped(),
        vec![(&reactor, Some(&reactor))]
    );
    let compact = format!("{topology:#?}")
        .split_whitespace()
        .collect::<String>();
    assert!(compact.contains(
        r##""root/#b0..root/#b0":[Reactor{component:"component/root",parent:None,bank:Some("#b0of1",)"##
    ));
}

#[test]
fn application_topology_debug_preserves_singleton_port_bank() {
    let (topology, _, port) = singleton_bank_topology();
    assert_eq!(topology.ports_debug_grouped(), vec![(&port, Some(&port))]);
    let compact = format!("{topology:#?}")
        .split_whitespace()
        .collect::<String>();
    assert!(compact.contains(
        r##""root/#b0/channel/#b0..root/#b0/channel/#b0":[Port{reactor:"root/#b0",direction:Input,bank:Some("#b0of1",)"##
    ));
}
