use boomerang_runtime::{ActionCommon, BaseAction, CommonContext};
use itertools::Itertools;

use super::*;
use crate::runtime;

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
                    period: Some(runtime::Duration::ZERO),
                    offset: Some(runtime::Duration::ZERO),
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
        .add_reaction("test", |_| runtime::reaction_closure!().into())
        .finish();

    assert!(matches!(res, Err(BuilderError::ReactionBuilderError(_))));
}

#[test]
fn test_reactions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let startup = reactor_builder.get_startup_action();

    let r0_key = reactor_builder
        .add_reaction("test", |_| runtime::reaction_closure!().into())
        .with_action(startup, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction("test", |_| runtime::reaction_closure!().into())
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

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts().unwrap();
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    let r0_key = aliases.reaction_aliases[r0_key].1;
    let r1_key = aliases.reaction_aliases[r1_key].1;

    assert_eq!(enclave.env.reactions.len(), 2);
    assert_eq!(
        enclave.graph.startup_reactions[&runtime::Duration::ZERO],
        vec![
            (runtime::Level::from(0), r0_key),
            (runtime::Level::from(1), r1_key),
        ]
    );
}

#[test]
fn test_actions1() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let action_a = reactor_builder
        .add_logical_action::<()>("a", Some(runtime::Duration::seconds(1)))
        .unwrap();
    let action_b = reactor_builder.add_logical_action::<()>("b", None).unwrap();

    // Triggered by a+b, schedules b
    let reaction_a = reactor_builder
        .add_reaction("ra", |_| runtime::reaction_closure!().into())
        .with_action(action_a, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .with_action(action_b, 1, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction("rb", |_| runtime::reaction_closure!().into())
        .with_action(action_a, 0, TriggerMode::TriggersAndEffects)
        .unwrap()
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();
    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts().unwrap();
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    let reaction_a = aliases.reaction_aliases[reaction_a].1;
    let reaction_b = aliases.reaction_aliases[reaction_b].1;
    let action_a = aliases.action_aliases[action_a.into()].1;
    let action_b = aliases.action_aliases[action_b.into()].1;

    assert_eq!(
        enclave.env.actions[action_a]
            .downcast_ref::<runtime::Action>()
            .expect("Action")
            .name(),
        "a"
    );

    // action_a is TriggersOnly on reaction_a, so should not be in the `reaction_actions`
    itertools::assert_equal(
        enclave.graph.reaction_actions[reaction_a].iter(),
        [action_b],
    );

    itertools::assert_equal(
        enclave.graph.action_triggers[action_a]
            .iter()
            .map(|&(_, r)| r),
        [reaction_a, reaction_b],
    );

    itertools::assert_equal(
        enclave.graph.reaction_actions[reaction_b].iter(),
        [action_a],
    );
}

/// Test port bindings and connections within a nested reactor.
#[test]
fn test_nested_reactor() {
    let mut env_builder = EnvBuilder::new();

    let mut outer_builder = env_builder.add_reactor("outer", None, None, (), false);
    let outer_input = outer_builder.add_input_port::<()>("input").unwrap();
    let outer_output = outer_builder.add_output_port::<()>("output").unwrap();

    let inner_reactor = outer_builder
        .add_child_with(|parent, env| {
            let mut inner_builder = env.add_reactor("inner", Some(parent), None, (), false);
            let input_port = inner_builder.add_input_port::<()>("input").unwrap();
            let output_port = inner_builder.add_output_port::<()>("output").unwrap();

            let _ = inner_builder
                .add_reaction("reaction", |_| {
                    runtime::reaction_closure!(_ctx, _state, ref_ports, mut_ports, _actions => {
                        let _input: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        let mut output: runtime::OutputRef<()> = mut_ports.partition_mut().unwrap();
                        *output = Some(());
                    })
                    .into()
                })
                .with_port(input_port, 0, TriggerMode::TriggersOnly)?
                .with_port(output_port, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            inner_builder.finish()
        })
        .unwrap();

    let _outer_reactor = outer_builder.finish().unwrap();

    let inner_input = env_builder
        .find_port_by_name("input", inner_reactor)
        .unwrap();
    let inner_output = env_builder
        .find_port_by_name("output", inner_reactor)
        .unwrap();

    env_builder
        .add_port_connection::<(), _, _>(outer_input, inner_input, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(inner_output, outer_output, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(
            outer_output,
            outer_input,
            // This connection *must* be delayed to avoid a cycle
            Some(runtime::Duration::milliseconds(1)),
            false,
        )
        .unwrap();

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts().unwrap();
    assert_eq!(enclaves.len(), 1);

    assert_eq!(
        aliases.port_aliases[outer_input.into()],
        aliases.port_aliases[inner_input],
        "inner and outer input ports should alias"
    );
    assert_eq!(
        aliases.port_aliases[outer_output.into()],
        aliases.port_aliases[inner_output],
        "inner and outer output ports should alias"
    );

    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    let inner_reactor_key = aliases.reactor_aliases[inner_reactor].1;
    assert_eq!(
        enclave.env.reactors[inner_reactor_key].name(),
        "outer::inner"
    );
}

/// Test semantics of trigger/effect/uses ports on reactions.
#[test]
fn test_reaction_ports() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder_a = env_builder.add_reactor("reactorA", None, None, (), false);
    let port_a = builder_a.add_input_port::<()>("portA").unwrap();
    let port_b = builder_a.add_output_port::<()>("portB").unwrap();
    let port_c = builder_a.add_input_port::<()>("portC").unwrap();
    let reaction_a = builder_a
        .add_reaction("reactionA", |_| runtime::reaction_closure!().into())
        .with_port(port_a, 0, TriggerMode::TriggersOnly)?
        .with_port(port_b, 0, TriggerMode::EffectsOnly)?
        .with_port(port_c, 0, TriggerMode::UsesOnly)?
        .finish()?;
    let _reactor_a = builder_a.finish()?;

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts().unwrap();
    assert_eq!(enclaves.len(), 1);
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    let reaction_a = aliases.reaction_aliases[reaction_a].1;
    let port_a = aliases.port_aliases[port_a.into()].1;
    let port_b = aliases.port_aliases[port_b.into()].1;
    let port_c = aliases.port_aliases[port_c.into()].1;

    // reactionA should "use" (be able to read from) portC
    itertools::assert_equal(
        enclave.graph.reaction_use_ports[reaction_a].iter(),
        std::iter::once(port_c),
    );

    // reactionA should "effect" (be able to write to) portB
    itertools::assert_equal(
        enclave.graph.reaction_effect_ports[reaction_a].iter(),
        std::iter::once(port_b),
    );

    // portA should trigger only reactionA
    itertools::assert_equal(
        enclave.graph.port_triggers[port_a]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        std::iter::once(&reaction_a),
    );

    // portB should not trigger any reactions
    itertools::assert_equal(
        enclave.graph.port_triggers[port_b]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        std::iter::empty::<&runtime::ReactionKey>(),
    );

    // portC should not trigger any reactions
    itertools::assert_equal(
        enclave.graph.port_triggers[port_c]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        std::iter::empty::<&runtime::ReactionKey>(),
    );

    Ok(())
}

/// Test that use-dependencies may be declared on logical actions and timers.
#[test]
fn test_dependency_use_on_logical_action() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();

    let mut builder_main = env_builder.add_reactor("main", None, None, 0u32, false);
    let clock = builder_main.add_logical_action::<u32>("clock", None)?;
    let a = builder_main.add_logical_action::<()>("a", None)?;
    let t = builder_main.add_timer(
        "t",
        TimerSpec {
            period: Some(runtime::Duration::milliseconds(2)),
            offset: None,
        },
    )?;
    let startup_action = builder_main.get_startup_action();
    let shutdown_action = builder_main.get_shutdown_action();

    // reaction(startup) -> clock, a
    let _r_startup = builder_main
        .add_reaction(
            "startup",
            |_|
            runtime::reaction_closure!(ctx, _state, inputs, outputs, actions => {
                assert_eq!(inputs.len(), 0);
                assert_eq!(outputs.len(), 0);
                assert_eq!(actions.len(), 2);

                println!("startup");
                let (mut clock, mut a): (runtime::ActionRef<u32>, runtime::ActionRef<()>) = actions.partition_mut().unwrap();

                ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(3))); // out of order on purpose
                ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(1)));
                ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(5)));

                // not scheduled on milli 1 (action is)
                ctx.schedule_action(&mut clock, 2, Some(runtime::Duration::milliseconds(2)));
                ctx.schedule_action(&mut clock, 3, Some(runtime::Duration::milliseconds(3)));
                ctx.schedule_action(&mut clock, 4, Some(runtime::Duration::milliseconds(4)));
                ctx.schedule_action(&mut clock, 5, Some(runtime::Duration::milliseconds(5)));
                // not scheduled on milli 6 (timer is)
            }).into(),
        )
        .with_action(startup_action, 0, TriggerMode::TriggersOnly)?
        .with_action(clock, 0, TriggerMode::EffectsOnly)?
        .with_action(a, 0, TriggerMode::EffectsOnly)?
        .finish()?;

    //reaction(clock) a, t {= =}
    let _r_clock = builder_main
        .add_reaction(
            "clock",
            |_| runtime::reaction_closure!(ctx, reactor, _inputs, _outputs, actions => {
                let (mut clock, mut a, mut t): (runtime::ActionRef<u32>, runtime::ActionRef<()>, runtime::ActionRef<()>) = actions.partition_mut().unwrap();
                let reactor: &mut runtime::Reactor<u32> = reactor.downcast_mut().unwrap();

                match ctx.get_action_value(&mut clock) {
                    Some(2) | Some(4) => {
                        assert!(t.is_present(ctx)); // t is there on even millis
                        assert!(!a.is_present(ctx)); //
                    }
                    Some(3) | Some(5) => {
                        assert!(!t.is_present(ctx));
                        assert!(a.is_present(ctx));
                    }
                    it => unreachable!("{:?}", it),
                }

                reactor.state += 1;
            }).into(),
        )
        .with_action(clock, 0, TriggerMode::TriggersAndUses)?
        .with_action(a, 0, TriggerMode::UsesOnly)?
        .with_action(t, 0, TriggerMode::UsesOnly)?
        .finish()?;

    // reaction(shutdown) {= =}
    let _r_shutdown = builder_main
        .add_reaction("shutdown", |_| {
            runtime::reaction_closure!(_ctx, reactor, _inputs, _outputs, _actions => {
                let reactor: &mut runtime::Reactor<u32> = reactor.downcast_mut().unwrap();
                assert_eq!(reactor.state, 4);
                println!("success");
            })
            .into()
        })
        .with_action(shutdown_action, 0, TriggerMode::TriggersOnly)?
        .finish()?;

    builder_main.finish()?;

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts()?;
    assert_eq!(enclaves.len(), 1);
    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    // r_startup should be triggered by the startup action, but the startup action should not be in its list of actions (triggers only).
    let r_startup_runtime = aliases.reaction_aliases[_r_startup].1;
    let startup_action_runtime = aliases.action_aliases[startup_action.into()].1;
    let actual = enclave.graph.action_triggers[startup_action_runtime]
        .iter()
        .map(|(_, x)| *x)
        .collect_vec();
    assert_eq!(
        actual,
        vec![r_startup_runtime],
        "startup action should trigger r_startup"
    );

    let actual = enclave.graph.reaction_actions[r_startup_runtime]
        .iter()
        .collect_vec();
    assert_eq!(
        actual,
        vec![
            aliases.action_aliases[clock.into()].1,
            aliases.action_aliases[a.into()].1,
        ],
        "r_startup should have [clock, a] as actions"
    );

    let r_clock_runtime = aliases.reaction_aliases[_r_clock].1;
    let actual = enclave.graph.action_triggers[aliases.action_aliases[clock.into()].1]
        .iter()
        .map(|(_, x)| *x)
        .collect_vec();
    assert_eq!(
        actual,
        vec![r_clock_runtime],
        "clock action should trigger r_clock"
    );

    let actual = enclave.graph.reaction_actions[r_clock_runtime]
        .iter()
        .collect_vec();
    assert_eq!(
        actual,
        vec![
            aliases.action_aliases[clock.into()].1,
            aliases.action_aliases[a.into()].1,
            aliases.action_aliases[t.into()].1,
        ],
        "r_clock should have [clock, a, t] as actions"
    );

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::seconds(1));
    let mut sched = runtime::Scheduler::new(enclave_key, enclave, config);
    sched.event_loop();

    Ok(())
}

/// Test that use-dependencies may be absent within a reaction.
#[test]
fn test_dependency_use_accessible() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);

    let source_reactor = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("Source", Some(parent), None, (), false);
            let clock = builder.add_output_port::<u32>("clock")?;
            let o1 = builder.add_output_port::<u32>("o1")?;
            let o2 = builder.add_output_port::<u32>("o2")?;
            let t1 = builder
                .add_timer(
                    "t1",
                    TimerSpec {
                        period: None,
                        offset: Some(runtime::Duration::milliseconds(35)),
                    },
                )
                .unwrap();
            let t2 = builder
                .add_timer(
                    "t2",
                    TimerSpec {
                        period: None,
                        offset: Some(runtime::Duration::milliseconds(70)),
                    },
                )
                .unwrap();
            let startup_action = builder.get_startup_action();
            let _ = builder
                .add_reaction("startup", |_| {
                    runtime::reaction_closure!(ctx, _state, _ref_ports, mut_ports, _actions => {
                        let mut clock: runtime::OutputRef<u32> = mut_ports.partition_mut().unwrap();
                        assert_eq!(clock.name(), "clock");
                        *clock = Some(0);
                        ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(140)));
                    })
                    .into()
                })
                .with_action(startup_action, 0, TriggerMode::TriggersOnly)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            let _ = builder
                .add_reaction("reaction_t1", |_| {
                    runtime::reaction_closure!(_ctx, _state, _ref_ports, mut_ports, actions => {
                        let [mut clock, mut o1]: [runtime::OutputRef<u32>; 2] =
                            mut_ports.partition_mut().unwrap();

                        assert_eq!(clock.name(), "clock");
                        *clock = Some(1);

                        assert_eq!(o1.name(), "o1");
                        *o1 = Some(10);

                        let t1: runtime::ActionRef = actions.partition_mut().unwrap();
                        assert_eq!(t1.name(), "t1");
                    })
                    .into()
                })
                .with_action(t1, 0, TriggerMode::TriggersAndUses)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .with_port(o1, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            let _ = builder
                .add_reaction("reaction_t2", |_| {
                    runtime::reaction_closure!(_ctx, _state, _ref_ports, mut_ports, _actions => {
                        let [mut clock, o2]: [runtime::OutputRef<u32>; 2] =
                            mut_ports.partition_mut().unwrap();

                        assert_eq!(clock.name(), "clock");
                        *clock = Some(2);

                        assert_eq!(o2.name(), "o2");
                        // we purposefully do not set o2
                    })
                    .into()
                })
                .with_action(t2, 0, TriggerMode::TriggersOnly)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .with_port(o2, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            builder.finish()
        })
        .unwrap();

    let sink_reactor = builder.add_child_with(|parent, env| {
        let mut builder = env.add_reactor("Sink", Some(parent), None, (), false);
        let clock = builder.add_input_port::<u32>("clock").unwrap();
        let in1 = builder.add_input_port::<u32>("in1").unwrap();
        let in2 = builder.add_input_port::<u32>("in2").unwrap();
        let _ = builder
            .add_reaction("reaction_clock", |_| {
                runtime::reaction_closure!(_ctx, _state, ref_ports, _mut_ports, _actions => {
                    let [clock, in1, in2]: [runtime::InputRef<u32>; 3] =
                        ref_ports.partition().unwrap();
                    assert_eq!(clock.name(), "clock");
                    assert_eq!(in1.name(), "o1");
                    assert_eq!(in2.name(), "o2");

                    match *clock {
                        Some(0) | Some(2) => {
                            assert_eq!(None, *in1);
                            assert_eq!(None, *in2);
                        }
                        Some(1) => {
                            assert_eq!(Some(10), *in1);
                            assert_eq!(None, *in2);
                        }
                        c => panic!("No such signal expected {:?}", c),
                    }
                })
                .into()
            })
            .with_port(clock, 0, TriggerMode::TriggersAndUses)?
            .with_port(in1, 0, TriggerMode::UsesOnly)?
            .with_port(in2, 0, TriggerMode::UsesOnly)?
            .finish()?;

        builder.finish()
    })?;

    let _main_reactor = builder.finish()?;

    let clock_source = env_builder.find_port_by_name("clock", source_reactor)?;
    let clock_sink = env_builder.find_port_by_name("clock", sink_reactor)?;
    env_builder.add_port_connection::<u32, _, _>(clock_source, clock_sink, None, false)?;

    let o1_source = env_builder.find_port_by_name("o1", source_reactor)?;
    let in1_sink = env_builder.find_port_by_name("in1", sink_reactor)?;
    env_builder.add_port_connection::<u32, _, _>(o1_source, in1_sink, None, false)?;

    let o2_source = env_builder.find_port_by_name("o2", source_reactor)?;
    let in2_sink = env_builder.find_port_by_name("in2", sink_reactor)?;
    env_builder.add_port_connection::<u32, _, _>(o2_source, in2_sink, None, false)?;

    let reaction_source_startup_key =
        env_builder.find_reaction_by_name("startup", source_reactor)?;
    let _reaction_source_t1_key =
        env_builder.find_reaction_by_name("reaction_t1", source_reactor)?;
    let _reaction_source_t2_key =
        env_builder.find_reaction_by_name("reaction_t2", source_reactor)?;
    let reaction_sink_clock_key =
        env_builder.find_reaction_by_name("reaction_clock", sink_reactor)?;

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts()?;
    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    // the Source startup reaction should trigger on startup and effect the clock port
    let runtime_reaction_source_startup_key =
        aliases.reaction_aliases[reaction_source_startup_key].1;
    let actual = enclave.graph.reaction_effect_ports[runtime_reaction_source_startup_key]
        .iter()
        .collect_vec();
    assert_eq!(
        actual,
        [aliases.port_aliases[clock_source].1],
        "Source startup reaction should have clock as effect port"
    );

    let runtime_reaction_sink_clock_key = aliases.reaction_aliases[reaction_sink_clock_key].1;

    // The clock reaction should only be triggered by the `clock` port, not the `in1` or `in2` ports.
    let actual = enclave.graph.port_triggers[aliases.port_aliases[clock_sink].1]
        .iter()
        .map(|(_, reaction_key)| *reaction_key)
        .collect_vec();
    assert_eq!(
        actual,
        [runtime_reaction_sink_clock_key],
        "clock port should trigger clock reaction"
    );

    // The clock reaction should have the `clock`, `in1`, and `in2` ports as use ports.
    let actual = enclave.graph.reaction_use_ports[runtime_reaction_sink_clock_key]
        .iter()
        .collect_vec();
    assert_eq!(
        actual,
        vec![
            aliases.port_aliases[clock_sink].1,
            aliases.port_aliases[in1_sink].1,
            aliases.port_aliases[in2_sink].1,
        ],
        "clock reaction should have clock, in1, and in2 as use ports"
    );

    // The clock reaction should not have any effect ports.
    let actual = enclave.graph.reaction_effect_ports[runtime_reaction_sink_clock_key]
        .iter()
        .collect_vec();
    assert!(
        actual.is_empty(),
        "clock reaction should not have any effect ports"
    );

    let config = runtime::Config::default().with_fast_forward(true);
    let mut sched = runtime::Scheduler::new(enclave_key, enclave, config);
    sched.event_loop();

    Ok(())
}

#[test]
fn test_enclave_partitioning() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("world", None, None, (), false);

    let hello1 = reactor_builder
        .add_child_with(|builder_reactor_key, builder| {
            let mut reactor =
                builder.add_reactor("hello1", Some(builder_reactor_key), None, (), false);
            let startup = reactor.get_startup_action();
            let _ = reactor
                .add_reaction("startup", |_| {
                    runtime::reaction_closure!(_ctx, _state, _inputs, _outputs, _actions => {
                        println!("Hello, world!");
                    })
                    .into()
                })
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
                .add_reaction("startup", |_| {
                    runtime::reaction_closure!(_ctx, _state, _inputs, _outputs, _actions => {
                        println!("Hello, enclave!");
                    })
                    .into()
                })
                .with_action(startup, 0, TriggerMode::TriggersOnly)
                .unwrap()
                .finish()
                .unwrap();
            reactor.finish()
        })
        .unwrap();

    let world = reactor_builder.finish().unwrap();

    let builder_parts = env_builder.into_runtime_parts().unwrap();
    assert_eq!(builder_parts.enclaves.len(), 2, "Expected 2 enclaves");

    let (world_enclave, world_key) = builder_parts.aliases.reactor_aliases[world];
    let (hello1_enclave, hello1_key) = builder_parts.aliases.reactor_aliases[hello1];
    let (hello2_enclave, hello2_key) = builder_parts.aliases.reactor_aliases[hello2];

    assert_eq!(
        world_enclave, hello1_enclave,
        "Expected world and hello1 in same enclave"
    );
    assert_eq!(
        builder_parts.enclaves[world_enclave]
            .env
            .reactors
            .keys()
            .collect::<Vec<_>>(),
        vec![world_key, hello1_key],
        "Expected only the world and hello1 reactors in the first enclave"
    );
    assert_eq!(
        builder_parts.enclaves[hello2_enclave]
            .env
            .reactors
            .keys()
            .collect::<Vec<_>>(),
        vec![hello2_key],
        "Expected only the hello2 reactor in the second enclave"
    )
}

pub struct PingPong {
    pub env_builder: EnvBuilder,
    pub main: BuilderReactorKey,
    pub ping: BuilderReactorKey,
    pub pong: BuilderReactorKey,
    pub ping_input: BuilderPortKey,
    pub ping_output: BuilderPortKey,
    pub pong_input: BuilderPortKey,
    pub pong_output: BuilderPortKey,
}

/// Create a simple ping-pong system with two child enclaves
pub fn create_ping_pong() -> PingPong {
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
                        period: Some(runtime::Duration::milliseconds(1)),
                        offset: None,
                    },
                )
                .unwrap();
            let i1 = builder.add_input_port::<()>("i1")?;
            let o1 = builder.add_output_port::<()>("o1")?;

            let _ = builder
                .add_reaction("reaction_t1", |_| {
                    runtime::reaction_closure!(_ctx, _reactor, _ref_ports, mut_ports, _actions => {
                        let mut o1: runtime::OutputRef<()> = mut_ports.partition_mut().unwrap();
                        *o1 = Some(());
                    })
                    .into()
                })
                .with_action(t1, 0, TriggerMode::TriggersOnly)?
                .with_port(o1, 0, TriggerMode::EffectsOnly)?
                .finish()?;
            let _ = builder
                .add_reaction("reaction_i1", |_| {
                    runtime::reaction_closure!(_ctx, _reactor, ref_ports, _mut_ports, _actions => {
                        let _i1: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        println!("{greeting}");
                    })
                    .into()
                })
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
    let main = builder.finish().unwrap();

    let ping_i1 = env_builder.find_port_by_name("i1", ping).unwrap();
    let ping_o1 = env_builder.find_port_by_name("o1", ping).unwrap();
    let pong_i1 = env_builder.find_port_by_name("i1", pong).unwrap();
    let pong_o1 = env_builder.find_port_by_name("o1", pong).unwrap();

    env_builder
        .add_port_connection::<(), _, _>(ping_o1, pong_i1, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(
            pong_o1,
            ping_i1,
            Some(runtime::Duration::milliseconds(1)),
            false,
        )
        .unwrap();

    PingPong {
        env_builder,
        main,
        ping,
        pong,
        ping_input: ping_i1,
        ping_output: ping_o1,
        pong_input: pong_i1,
        pong_output: pong_o1,
    }
}

#[test]
fn test_enclave2() {
    let PingPong {
        env_builder,
        main: _,
        ping: _,
        pong: _,
        ping_input: _,
        ping_output: _,
        pong_input: _,
        pong_output: _,
    } = create_ping_pong();

    let BuilderRuntimeParts {
        enclaves,
        aliases: _,
    } = env_builder.into_runtime_parts().unwrap();
    assert_eq!(enclaves.len(), 3);

    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(runtime::Duration::milliseconds(3));

    let _envs = runtime::execute_enclaves(enclaves.into_iter(), config);
}

/// Test binding of ports between two child reactors
#[test]
fn test_port_binding() {
    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, (), false);

    let child1 = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("child1", Some(parent), None, (), false);
            let i1 = builder.add_input_port::<()>("i1").unwrap();
            let o1 = builder.add_output_port::<()>("o1").unwrap();
            let _ = builder
                .add_reaction("reaction", |_| {
                    runtime::reaction_closure!(_ctx, _state, ref_ports, mut_ports, _actions => {
                        let i1: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        let mut o1: runtime::OutputRef<()> = mut_ports.partition_mut().unwrap();
                        *o1 = *i1;
                    })
                    .into()
                })
                .with_port(i1, 0, TriggerMode::TriggersAndUses)?
                .with_port(o1, 0, TriggerMode::EffectsOnly)?
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let child2a = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("child2a", Some(parent), None, (), false);
            let i2 = builder.add_input_port::<()>("i2a").unwrap();
            let _ = builder
                .add_reaction("reaction", |_| {
                    runtime::reaction_closure!(_ctx, _state, ref_ports, _mut_ports, _actions => {
                        let i2: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        assert_eq!(*i2, Some(()));
                    })
                    .into()
                })
                .with_port(i2, 0, TriggerMode::TriggersAndUses)?
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let child2b = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("child2b", Some(parent), None, (), false);
            let i2 = builder.add_input_port::<()>("i2b").unwrap();
            let _ = builder
                .add_reaction("reaction", |_| {
                    runtime::reaction_closure!(_ctx, _state, ref_ports, _mut_ports, _actions => {
                        let i2: runtime::InputRef<()> = ref_ports.partition().unwrap();
                        assert_eq!(*i2, Some(()));
                    })
                    .into()
                })
                .with_port(i2, 0, TriggerMode::TriggersAndUses)?
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let startup_key = builder.get_startup_action();
    let _main = builder.finish().unwrap();

    let i1 = env_builder.find_port_by_name("i1", child1).unwrap();
    let o1 = env_builder.find_port_by_name("o1", child1).unwrap();
    let i2a = env_builder.find_port_by_name("i2a", child2a).unwrap();
    let i2b = env_builder.find_port_by_name("i2b", child2b).unwrap();

    let _ = env_builder
        .add_reaction("start", _main, |_| {
            runtime::reaction_closure!(_ctx, _state, _ref_ports, _mut_ports, _actions => {
                println!("start");
                let mut i1: runtime::OutputRef<()> = _mut_ports.partition_mut().unwrap();
                *i1 = Some(());
            })
            .into()
        })
        .with_action(startup_key, 0, TriggerMode::TriggersOnly)
        .unwrap()
        .with_port(i1, 0, TriggerMode::EffectsOnly)
        .unwrap()
        .finish()
        .unwrap();

    env_builder
        .add_port_connection::<(), _, _>(o1, i2a, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(o1, i2b, None, false)
        .unwrap();

    let BuilderRuntimeParts { enclaves, aliases } = env_builder.into_runtime_parts().unwrap();
    assert_eq!(enclaves.len(), 1);
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    assert_eq!(enclave.env.reactors.len(), 4);

    let _i1 = aliases.port_aliases[i1].1;
    let o1 = aliases.port_aliases[o1].1;
    let i2a = aliases.port_aliases[i2a].1;
    let _i2b = aliases.port_aliases[i2b].1;

    // Port o1 should alias to Port i2
    assert_eq!(enclave.env.ports.len(), 2);
    assert_eq!(o1, i2a);
}
