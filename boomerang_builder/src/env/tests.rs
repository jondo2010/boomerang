use runtime::BaseAction;

use crate::{TimerSpec, TriggerMode};

use super::*;

#[test]
fn test_duplicate_ports() {
    let mut env_builder = EnvBuilder::new();
    let reactor_key = env_builder
        .add_reactor("test_reactor", None, None, (), false)
        .finish()
        .unwrap();
    let _ = env_builder
        .add_input_port::<()>("port0", reactor_key)
        .unwrap();

    assert!(matches!(
        env_builder
            .add_output_port::<()>("port0", reactor_key)
            .expect_err("Expected duplicate"),
        BuilderError::DuplicatePortDefinition {
            reactor_name,
            port_name
        } if reactor_name == "test_reactor" && port_name == "port0"
    ));
}

#[test]
fn test_duplicate_actions() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    reactor_builder
        .add_logical_action::<()>("action0", None)
        .unwrap();

    assert!(matches!(
        reactor_builder
            .add_logical_action::<()>("action0", None)
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name== "test_reactor" && action_name == "action0"
    ));

    assert!(matches!(
        reactor_builder
            .add_timer(
                "action0",
                TimerSpec {
                    period: Some(Duration::ZERO),
                    offset: Some(Duration::ZERO),
                }
            )
            .expect_err("Expected duplicate"),
        BuilderError::DuplicateActionDefinition {
            reactor_name,
            action_name,
        } if reactor_name == "test_reactor" && action_name == "action0"
    ));
}

/// Assert that building a reaction without any triggers is an error
#[test]
fn test_reactions_with_trigger() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let res = reactor_builder
        .add_reaction("test", runtime::reaction_closure!())
        .finish();

    assert!(matches!(res, Err(BuilderError::ReactionBuilderError(_))));
}

#[test]
fn test_reactions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let startup = reactor_builder.get_startup_action();

    let r0_key = reactor_builder
        .add_reaction("test", runtime::reaction_closure!())
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", runtime::reaction_closure!())
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();

    assert_eq!(env_builder.reactor_builders.len(), 1);
    assert_eq!(env_builder.reaction_builders.len(), 2);
    assert_eq!(
        env_builder.reaction_builders.keys().collect::<Vec<_>>(),
        vec![r0_key, r1_key]
    );

    let runtime_parts = env_builder.into_runtime_parts().unwrap();
    assert_eq!(runtime_parts[0].env.reactions.len(), 2);
}

#[test]
fn test_actions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let action_a = reactor_builder
        .add_logical_action::<()>("a", Some(Duration::from_secs(1)))
        .unwrap();
    let action_b = reactor_builder.add_logical_action::<()>("b", None).unwrap();

    // Triggered by a+b, schedules b
    let reaction_a = reactor_builder
        .add_reaction("ra", runtime::reaction_closure!())
        .with_action(action_a, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .with_action(action_b, 1, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction("rb", runtime::reaction_closure!())
        .with_action(action_a, 0, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();
    let runtime_parts = env_builder.into_runtime_parts().unwrap();
    let EnclaveParts {
        env,
        graph,
        aliases,
    } = &runtime_parts[0];

    //runtime::check_consistency(&env, &dep_info);

    let reaction_a = aliases.reaction_aliases[reaction_a];
    let reaction_b = aliases.reaction_aliases[reaction_b];
    let action_a = aliases.action_aliases[action_a.into()];
    let action_b = aliases.action_aliases[action_b.into()];

    assert_eq!(
        env.actions[action_a]
            .downcast_ref::<runtime::Action>()
            .expect("Action")
            .name(),
        "a"
    );

    // action_a is TriggersOnly on reaction_a, so should not be in the `reaction_actions`
    itertools::assert_equal(graph.reaction_actions[reaction_a].iter(), [action_b]);

    itertools::assert_equal(
        graph.action_triggers[action_a].iter().map(|&(_, r)| r),
        [reaction_a, reaction_b],
    );

    itertools::assert_equal(graph.reaction_actions[reaction_b].iter(), [action_a]);
}

#[test]
fn test_enclave1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("world", None, None, (), false);

    let hello1 = reactor_builder
        .add_child_with(|builder_reactor_key, builder| {
            let mut reactor =
                builder.add_reactor("hello1", Some(builder_reactor_key), None, (), false);
            let startup = reactor.get_startup_action();
            let _ = reactor
                .add_reaction(
                    "startup",
                    runtime::reaction_closure!(_ctx, _state, _inputs, _outputs, _actions => {
                        println!("Hello, world!");
                    }),
                )
                .with_action(startup, 0, TriggerMode::TriggersOnly)
                .unwrap()
                .finish()
                .unwrap();
            reactor.finish()
        })
        .unwrap();

    let hello2 = reactor_builder
        .add_child_with(|builder_reactor_key, builder| {
            let mut reactor =
                builder.add_reactor("hello2", Some(builder_reactor_key), None, (), true);
            let startup = reactor.get_startup_action();
            let _ = reactor
                .add_reaction(
                    "startup",
                    runtime::reaction_closure!(_ctx, _state, _inputs, _outputs, _actions => {
                        println!("Hello, enclave!");
                    }),
                )
                .with_action(startup, 0, TriggerMode::TriggersOnly)
                .unwrap()
                .finish()
                .unwrap();
            reactor.finish()
        })
        .unwrap();

    let world = reactor_builder.finish().unwrap();

    dbg!(&env_builder);

    let runtime_parts = env_builder.into_runtime_parts().unwrap();
    assert_eq!(runtime_parts.len(), 2, "Expected 2 enclaves");

    dbg!(&runtime_parts);

    // the first enclave should contain the world and hello1 reactors
    let world_key = runtime_parts[0].aliases.reactor_aliases[world];
    let hello1_key = runtime_parts[0].aliases.reactor_aliases[hello1];
    assert_eq!(runtime_parts[0].env.reactors.len(), 2);
    assert_eq!(runtime_parts[0].env.reactors[world_key].name(), "world");
    assert_eq!(runtime_parts[0].env.reactors[hello1_key].name(), "hello1");

    // the second enclave should contain the hello2 reactor
    let hello2_key = runtime_parts[1].aliases.reactor_aliases[hello2];
    assert_eq!(runtime_parts[1].env.reactors.len(), 1);
    assert_eq!(runtime_parts[1].env.reactors[hello2_key].name(), "hello2");
}

/// Create a simple ping-pong system with two child enclaves
fn create_ping_pong() -> EnvBuilder {
    fn ping_pong(
        name: &str,
        is_enclave: bool,
    ) -> impl FnOnce(BuilderReactorKey, &mut EnvBuilder) -> Result<BuilderReactorKey, BuilderError>
           + use<'_> {
        let greeting = format!("{} received", name);
        move |parent, env: &mut EnvBuilder| {
            let mut builder = env.add_reactor(name, Some(parent), None, (), is_enclave);
            let t1 = builder
                .add_timer(
                    "t1",
                    TimerSpec {
                        period: Some(Duration::from_millis(1)),
                        offset: None,
                    },
                )
                .unwrap();
            let i1 = builder.add_input_port::<()>("i1")?;
            let o1 = builder.add_output_port::<()>("o1")?;

            let _ = builder
                .add_reaction(
                    "reaction_t1",
                    runtime::reaction_closure!(_ctx, _reactor, _ref_ports, mut_ports, _actions => {
                        let mut o1: runtime::OutputRef<()> = mut_ports.partition_mut().unwrap();
                        *o1 = Some(());
                    }),
                )
                .with_action(t1, 0, TriggerMode::TriggersOnly)?
                .with_port(o1, 0, TriggerMode::EffectsOnly)?
                .finish()?;
            let _ = builder
                .add_reaction(
                    "reaction_i1",
                    runtime::reaction_closure!(_ctx, _reactor, ref_ports, _mut_ports, _actions => {
                        let _i1: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        println!("{greeting}");
                    }),
                )
                .with_port(i1, 0, TriggerMode::TriggersAndUses)?
                .finish()?;
            builder.finish()
        }
    }

    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);

    // build ping and pong as child enclave reactors of main
    let ping = builder.add_child_with(ping_pong("Ping", true)).unwrap();
    let pong = builder.add_child_with(ping_pong("Pong", true)).unwrap();
    let _main = builder.finish().unwrap();

    let ping_i1 = env_builder.find_port_by_name("i1", ping).unwrap();
    let ping_o1 = env_builder.find_port_by_name("o1", ping).unwrap();
    let pong_i1 = env_builder.find_port_by_name("i1", pong).unwrap();
    let pong_o1 = env_builder.find_port_by_name("o1", pong).unwrap();

    env_builder
        .add_port_connection::<(), _, _>(ping_o1, pong_i1, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(pong_o1, ping_i1, Some(Duration::from_millis(50)), false)
        .unwrap();

    env_builder
}

#[test]
fn test_build_partition_map() {
    let env_builder = create_ping_pong();

    let main = env_builder.find_reactor_by_fqn("main").unwrap();
    let ping = env_builder.find_reactor_by_fqn("main::Ping").unwrap();
    let pong = env_builder.find_reactor_by_fqn("main::Pong").unwrap();

    let partition_map = env_builder.build_partition_map();
    assert_eq!(partition_map.len(), 3);
    // The main partition will contain the main reactor, but also the enclave connection aux reactors
    assert_eq!(partition_map[main].first(), Some(&main));
    assert_eq!(partition_map[ping], vec![ping]);
    assert_eq!(partition_map[pong], vec![pong]);
}

#[cfg(feature = "disable")]
#[test]
fn test_enclave2() {
    let env_builder = create_ping_pong();

    dbg!(&env_builder);

    let gv = env_builder.create_plantuml_graph().unwrap();
    let mut f = std::fs::File::create("test_enclave2.puml").unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    //let reaction_levels = self.build_runtime_level_map()?;

    let mut partitioned_port_keys =
        build::partition_port_builders(&env_builder.port_builders, &partition_map);

    assert_eq!(partitioned_port_keys.len(), 3);
    // The main partition will contain ports from the enclave connection aux reactors
    assert_eq!(partitioned_port_keys[main].len(), 4);
    assert_eq!(partitioned_port_keys[ping].len(), 2);
    assert_eq!(partitioned_port_keys[ping], vec![ping_i1, ping_o1]);
    assert_eq!(partitioned_port_keys[pong].len(), 2);
    assert_eq!(partitioned_port_keys[pong], vec![pong_i1, pong_o1]);

    let mut partitioned_reactions =
        build::partition_reaction_builders(env_builder.reaction_builders, &partition_map);

    assert_eq!(partitioned_reactions.len(), 3);
    // The main partition will contain the main reactor, but also the enclave connection aux reactors
    assert_eq!(partitioned_reactions[main].len(), 4);
    /*
    dbg!(&partitioned_reactions[ping]
        .iter()
        .map(|(k, v)| (k, &v.name))
        .collect::<Vec<_>>());
    dbg!(&partitioned_reactions[pong]
        .iter()
        .map(|(k, v)| (k, &v.name))
        .collect::<Vec<_>>());
    */

    let reactions_partition = partitioned_reactions.remove(main).unwrap();
    /*
    let build::RuntimeReactionParts {
        reactions: runtime_reactions,
        use_ports: reaction_use_ports,
        effect_ports: reaction_effect_ports,
        actions: reaction_actions,
        reaction_aliases,
        reaction_reactor_aliases,
    } = build::build_runtime_reactions(reactions_partition, &port_aliases, &action_aliases);
     */

    let mut partitioned_reactors =
        build::partition_reactor_builders(env_builder.reactor_builders, &partition_map);
    assert_eq!(partitioned_reactors.len(), 3);
    // The main partition will contain the main reactor, but also the enclave connection aux reactors
    assert_eq!(partitioned_reactors[main].len(), 3);
    assert_eq!(partitioned_reactors[ping].len(), 1);
    assert_eq!(partitioned_reactors[pong].len(), 1);

    let reactor_partition = partitioned_reactors.remove(main).unwrap();
    let build::RuntimeReactorParts {
        runtime_reactors,
        reactor_aliases,
        reactor_bank_indices,
    } = build::build_runtime_reactors(reactor_partition);

    reactor_aliases.iter().for_each(|(k, v)| {
        println!("{:?}: {:?}", k, v);
    });

    //let enclave_parts = env_builder.into_runtime_parts().unwrap();
}
