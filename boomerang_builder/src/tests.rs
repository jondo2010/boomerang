use boomerang_runtime::{ActionCommon, BaseAction, CommonContext, ReactionRefsExtract};
use itertools::Itertools;
use std::ptr::NonNull;

use super::*;
use crate::{port::Contained, runtime};

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
fn test_reaction_builder2() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor = env_builder.add_reactor("test_reactor", None, None, (), false);
    let p0 = reactor.add_input_port::<u32>("p0").unwrap();
    let p1 = reactor.add_output_port::<bool>("p1").unwrap();

    let _r0 = reactor
        .add_reaction(Some("test_reaction"))
        .with_trigger(p0)
        .with_effect(p1)
        .with_reaction_fn(|_ctx, _state, (p0, mut p1)| {
            *p1 = p0.map(|x| x > 0);
        })
        .finish()
        .unwrap();

    let _x = reactor.finish().unwrap();
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
fn test_reactions_without_trigger() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let x = reactor_builder
        .add_logical_action::<()>("test", None)
        .unwrap();

    let res = reactor_builder
        .add_reaction(None)
        .with_effect(x)
        .with_reaction_fn(|_ctx, _state, (_x,)| {})
        .finish();

    assert!(matches!(res, Err(BuilderError::ReactionBuilderError(_))));
}

#[test]
fn test_reactions_startup_shutdown() {
    let mut env_builder = EnvBuilder::new();
    let mut reactor_builder = env_builder.add_reactor("test_reactor", None, None, (), false);

    let r0_key = reactor_builder
        .add_reaction(Some("test"))
        .with_startup_trigger()
        .with_reaction_fn(|_ctx, _state, (_startup,)| {})
        .finish()
        .unwrap();

    let r1_key = reactor_builder
        .add_reaction(Some("test"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, _state, (_shutdown,)| {})
        .finish()
        .unwrap();

    let startup_action = reactor_builder.get_startup_action();
    let shutdown_action = reactor_builder.get_shutdown_action();

    let _reactor_key = reactor_builder.finish().unwrap();

    assert_eq!(env_builder.reactor_builders.len(), 1);
    assert_eq!(env_builder.reaction_builders.len(), 2);
    assert_eq!(
        env_builder.reaction_builders.keys().collect::<Vec<_>>(),
        vec![r0_key, r1_key]
    );

    assert_eq!(
        env_builder.reaction_builders[r0_key]
            .action_relations
            .iter()
            .next(),
        Some((startup_action.into(), &TriggerMode::TriggersAndUses)),
        "Startup reaction should have the startup action as a trigger"
    );

    assert_eq!(
        env_builder.reaction_builders[r1_key]
            .action_relations
            .iter()
            .next(),
        Some((shutdown_action.into(), &TriggerMode::TriggersAndUses)),
        "Shutdown reaction should have the shutdown action as a trigger"
    );

    env_builder.validate_reactions().unwrap();

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    let r0_key = aliases.reaction_aliases[r0_key].1;
    let r1_key = aliases.reaction_aliases[r1_key].1;

    let startup_key = aliases.action_aliases[startup_action.into()].1;
    let shutdown_key = aliases.action_aliases[shutdown_action.into()].1;

    assert_eq!(enclave.env.reactions.len(), 2);
    assert_eq!(
        enclave.graph.reaction_actions[r0_key]
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![startup_key]
    );
    assert_eq!(
        enclave.graph.reaction_actions[r1_key]
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![shutdown_key]
    );
    assert_eq!(
        enclave.graph.startup_actions,
        vec![(startup_key, runtime::Tag::ZERO)]
    );
    assert_eq!(enclave.graph.shutdown_actions, vec![shutdown_key])
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
        .add_reaction(Some("ra"))
        .with_trigger(action_a)
        .with_effect(action_b)
        .with_reaction_fn(|_ctx, _state, (_a, mut b)| {
            _ctx.schedule_action(&mut b, (), None);
        })
        .finish()
        .unwrap();

    // Triggered by a, schedules a
    let reaction_b = reactor_builder
        .add_reaction(Some("rb"))
        .with_trigger(action_a)
        .with_reaction_fn(|_ctx, _state, (_a,)| {})
        .finish()
        .unwrap();

    let _reactor_key = reactor_builder.finish().unwrap();
    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
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

    itertools::assert_equal(
        enclave.graph.reaction_actions[reaction_a].iter().copied(),
        [action_a, action_b],
    );

    itertools::assert_equal(
        enclave.graph.action_triggers[action_a]
            .iter()
            .map(|&(_, r)| r),
        [reaction_a, reaction_b],
    );

    itertools::assert_equal(
        enclave.graph.reaction_actions[reaction_b].iter().copied(),
        [action_a],
    );
}

#[test]
fn reaction_refs_extract_reports_missing() {
    // Build empty immutable ports; mutable ports and actions get dummy entries to satisfy iterator assumptions.
    let mut ports: Vec<NonNull<dyn runtime::BasePort>> = Vec::new();

    let mut dummy_mut_port: Box<dyn runtime::BasePort> =
        Box::new(runtime::Port::<()>::new("dummy", runtime::PortKey::from(0)));
    let mut ports_mut = vec![NonNull::from(&mut *dummy_mut_port)];

    let mut dummy_action: Box<dyn runtime::BaseAction> = Box::new(runtime::Action::<()>::new(
        "dummy_action",
        runtime::ActionKey::from(0),
        None,
        true,
    ));
    let mut actions = vec![NonNull::from(&mut *dummy_action)];

    let mut refs = runtime::ReactionRefs {
        ports: runtime::Refs::new(&mut ports),
        ports_mut: runtime::RefsMut::new(&mut ports_mut),
        actions: runtime::RefsMut::new(&mut actions),
    };

    let res = TypedPortKey::<u32, Input, Local>::extract(&mut refs);

    assert!(matches!(res, Err(runtime::ReactionRefsError::Missing { kind }) if kind == "input port"));
}

#[test]
fn reaction_refs_extract_reports_type_mismatch() {
    // Provide a bool port but request an input u32 port.
    let bool_port: Box<dyn runtime::BasePort> =
        Box::new(runtime::Port::<bool>::new("pb", runtime::PortKey::from(0)));
    let mut ports = vec![NonNull::from(&*bool_port)];

    let mut dummy_mut_port: Box<dyn runtime::BasePort> =
        Box::new(runtime::Port::<()>::new("dummy", runtime::PortKey::from(1)));
    let mut ports_mut = vec![NonNull::from(&mut *dummy_mut_port)];

    let mut dummy_action: Box<dyn runtime::BaseAction> = Box::new(runtime::Action::<()>::new(
        "dummy_action",
        runtime::ActionKey::from(0),
        None,
        true,
    ));
    let mut actions = vec![NonNull::from(&mut *dummy_action)];

    let mut refs = runtime::ReactionRefs {
        ports: runtime::Refs::new(&mut ports),
        ports_mut: runtime::RefsMut::new(&mut ports_mut),
        actions: runtime::RefsMut::new(&mut actions),
    };

    let res = TypedPortKey::<u32, Input, Local>::extract(&mut refs);

    assert!(matches!(res, Err(runtime::ReactionRefsError::TypeMismatch { kind, expected, found })
        if kind == "input port" && expected == std::any::type_name::<u32>() && found == std::any::type_name::<bool>()
    ));
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
                .add_reaction(Some("reaction"))
                .with_trigger(input_port)
                .with_effect(output_port)
                .with_reaction_fn(|_ctx, _state, (_input, mut output)| {
                    *output = Some(());
                })
                .finish()
                .unwrap();

            inner_builder.finish()
        })
        .unwrap();

    let _outer_reactor = outer_builder.finish().unwrap();

    let inner_input = env_builder
        .find_port_by_name::<(), Input>("input", inner_reactor)
        .unwrap();
    let inner_output = env_builder
        .find_port_by_name::<(), Output>("output", inner_reactor)
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

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
    assert_eq!(enclaves.len(), 1);

    assert_eq!(
        aliases.port_aliases[outer_input.into()],
        aliases.port_aliases[inner_input.into()],
        "inner and outer input ports should alias"
    );
    assert_eq!(
        aliases.port_aliases[outer_output.into()],
        aliases.port_aliases[inner_output.into()],
        "inner and outer output ports should alias"
    );

    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    let inner_reactor_key = aliases.reactor_aliases[inner_reactor].1;
    assert_eq!(
        enclave.env.reactors[inner_reactor_key].name(),
        "outer/inner"
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
        .add_reaction(Some("reactionA"))
        .with_trigger(port_a)
        .with_effect(port_b)
        .with_use(port_c)
        .with_reaction_fn(|_ctx, _state, (_port_a, mut _port_b, _port_c)| {})
        .finish()?;

    let _reactor_a = builder_a.finish()?;

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
    assert_eq!(enclaves.len(), 1);
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    let reaction_a = aliases.reaction_aliases[reaction_a].1;
    let port_a = aliases.port_aliases[port_a.into()].1;
    let port_b = aliases.port_aliases[port_b.into()].1;
    let port_c = aliases.port_aliases[port_c.into()].1;

    // reactionA should "use" (be able to read from) portC
    itertools::assert_equal(
        enclave.graph.reaction_use_ports[reaction_a].iter().copied(),
        [port_a, port_c],
    );

    // reactionA should "effect" (be able to write to) portB
    itertools::assert_equal(
        enclave.graph.reaction_effect_ports[reaction_a].iter().copied(),
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

    // reaction(startup) -> clock, a
    let _r_startup = builder_main
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(clock)
        .with_effect(a)
        .with_reaction_fn(|ctx, _state, (_startup, mut clock, mut a)| {
            println!("startup");
            ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(3))); // out of order on purpose
            ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(1)));
            ctx.schedule_action(&mut a, (), Some(runtime::Duration::milliseconds(5)));

            // not scheduled on milli 1 (action is)
            ctx.schedule_action(&mut clock, 2, Some(runtime::Duration::milliseconds(2)));
            ctx.schedule_action(&mut clock, 3, Some(runtime::Duration::milliseconds(3)));
            ctx.schedule_action(&mut clock, 4, Some(runtime::Duration::milliseconds(4)));
            ctx.schedule_action(&mut clock, 5, Some(runtime::Duration::milliseconds(5)));
            // not scheduled on milli 6 (timer is)
        })
        .finish()?;

    //reaction(clock) a, t {= =}
    let _r_clock = builder_main
        .add_reaction(Some("clock"))
        .with_trigger(clock)
        .with_use(a)
        .with_use(t)
        .with_reaction_fn(|ctx, state, (mut clock, mut a, mut t)| {
            println!("clock");
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
            *state += 1;
        })
        .finish()?;

    // reaction(shutdown) {= =}
    let _r_shutdown = builder_main
        .add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert_eq!(*state, 4);
            println!("success");
        })
        .finish()?;

    builder_main.finish()?;

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default())?;
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
        .copied()
        .collect_vec();
    assert_eq!(
        actual,
        vec![
            aliases.action_aliases[startup_action.into()].1,
            aliases.action_aliases[clock.into()].1,
            aliases.action_aliases[a.into()].1,
        ],
        "r_startup should have [startup, clock, a] as actions"
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
        .copied()
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
                .add_reaction(Some("startup"))
                .with_trigger(startup_action)
                .with_effect(clock)
                .with_reaction_fn(|ctx, _state, (_startup, mut clock)| {
                    assert_eq!(clock.name(), "clock");
                    *clock = Some(0);
                    ctx.schedule_shutdown(Some(runtime::Duration::milliseconds(140)));
                })
                .finish()?;

            let _ = builder
                .add_reaction(Some("reaction_t1"))
                .with_trigger(t1)
                .with_effect(clock)
                .with_effect(o1)
                .with_reaction_fn(|_ctx, _state, (t1, mut clock, mut o1)| {
                    assert_eq!(clock.name(), "clock");
                    *clock = Some(1);
                    assert_eq!(o1.name(), "o1");
                    *o1 = Some(10);
                    assert_eq!(t1.name(), "t1");
                })
                .finish()?;

            let _ = builder
                .add_reaction(Some("reaction_t2"))
                .with_trigger(t2)
                .with_effect(clock)
                .with_effect(o2)
                .with_reaction_fn(|_ctx, _state, (_t2, mut clock, o2)| {
                    assert_eq!(clock.name(), "clock");
                    *clock = Some(2);
                    assert_eq!(o2.name(), "o2");
                    // we purposefully do not set o2
                })
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
            .add_reaction(Some("reaction_clock"))
            .with_trigger(clock)
            .with_use(in1)
            .with_use(in2)
            .with_reaction_fn(|_ctx, _state, (clock, in1, in2)| {
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
            .finish()?;

        builder.finish()
    })?;

    let _main_reactor = builder.finish()?;

    let clock_source = env_builder
        .find_port_by_name::<u32, Output>("clock", source_reactor)
        .unwrap();
    let clock_sink = env_builder
        .find_port_by_name::<u32, Input>("clock", sink_reactor)
        .unwrap();
    env_builder.add_port_connection::<u32, _, _>(clock_source, clock_sink, None, false)?;

    let o1_source = env_builder
        .find_port_by_name::<u32, Output>("o1", source_reactor)
        .unwrap();
    let in1_sink = env_builder
        .find_port_by_name::<u32, Input>("in1", sink_reactor)
        .unwrap();
    env_builder.add_port_connection::<u32, _, _>(o1_source, in1_sink, None, false)?;

    let o2_source = env_builder
        .find_port_by_name::<u32, Output>("o2", source_reactor)
        .unwrap();
    let in2_sink = env_builder
        .find_port_by_name::<u32, Input>("in2", sink_reactor)
        .unwrap();
    env_builder.add_port_connection::<u32, _, _>(o2_source, in2_sink, None, false)?;

    let reaction_source_startup_key =
        env_builder.find_reaction_by_name("startup", source_reactor)?;
    let _reaction_source_t1_key =
        env_builder.find_reaction_by_name("reaction_t1", source_reactor)?;
    let _reaction_source_t2_key =
        env_builder.find_reaction_by_name("reaction_t2", source_reactor)?;
    let reaction_sink_clock_key =
        env_builder.find_reaction_by_name("reaction_clock", sink_reactor)?;

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default())?;
    let (enclave_key, enclave) = enclaves.into_iter().next().unwrap();

    // the Source startup reaction should trigger on startup and effect the clock port
    let runtime_reaction_source_startup_key =
        aliases.reaction_aliases[reaction_source_startup_key].1;
    let actual = enclave.graph.reaction_effect_ports[runtime_reaction_source_startup_key]
        .iter()
        .copied()
        .collect_vec();
    assert_eq!(
        actual,
        [aliases.port_aliases[clock_source.into()].1],
        "Source startup reaction should have clock as effect port"
    );

    let runtime_reaction_sink_clock_key = aliases.reaction_aliases[reaction_sink_clock_key].1;

    // The clock reaction should only be triggered by the `clock` port, not the `in1` or `in2` ports.
    let actual = enclave.graph.port_triggers[aliases.port_aliases[clock_sink.into()].1]
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
        .copied()
        .collect_vec();
    assert_eq!(
        actual,
        vec![
            aliases.port_aliases[clock_sink.into()].1,
            aliases.port_aliases[in1_sink.into()].1,
            aliases.port_aliases[in2_sink.into()].1,
        ],
        "clock reaction should have clock, in1, and in2 as use ports"
    );

    // The clock reaction should not have any effect ports.
    let actual = enclave.graph.reaction_effect_ports[runtime_reaction_sink_clock_key]
        .iter()
        .copied()
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
                .add_reaction(Some("startup"))
                .with_trigger(startup)
                .with_reaction_fn(|_ctx, _state, (_startup,)| {
                    println!("Hello, world!");
                })
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
                .add_reaction(Some("startup"))
                .with_trigger(startup)
                .with_reaction_fn(|_ctx, _state, (_startup,)| {
                    println!("Hello, world!");
                })
                .finish()
                .unwrap();
            reactor.finish()
        })
        .unwrap();

    let world = reactor_builder.finish().unwrap();

    let builder_parts = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
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
    pub ping_input: TypedPortKey<(), Input, Contained>,
    pub ping_output: TypedPortKey<(), Output, Contained>,
    pub pong_input: TypedPortKey<(), Input, Contained>,
    pub pong_output: TypedPortKey<(), Output, Contained>,
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
                .add_reaction(Some("reaction_t1"))
                .with_trigger(t1)
                .with_effect(o1)
                .with_reaction_fn(|_ctx, _state, (_t1, mut o1)| {
                    *o1 = Some(());
                })
                .finish()?;

            let _ = builder
                .add_reaction(Some("reaction_i1"))
                .with_trigger(i1)
                .with_reaction_fn(move |_ctx, _state, (i1,)| {
                    assert_eq!(*i1, Some(()));
                    println!("{greeting}");
                })
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

    let ping_i1 = env_builder
        .find_port_by_name::<(), Input>("i1", ping)
        .unwrap();
    let ping_o1 = env_builder
        .find_port_by_name::<(), Output>("o1", ping)
        .unwrap();
    let pong_i1 = env_builder
        .find_port_by_name::<(), Input>("i1", pong)
        .unwrap();
    let pong_o1 = env_builder
        .find_port_by_name::<(), Output>("o1", pong)
        .unwrap();

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
        ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
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
                .add_reaction(Some("reaction"))
                .with_trigger(i1)
                .with_effect(o1)
                .with_reaction_fn(|_ctx, _state, (i1, mut o1)| {
                    *o1 = *i1;
                })
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let child2a = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("child2a", Some(parent), None, (), false);
            let i2 = builder.add_input_port::<()>("i2a").unwrap();
            let _ = builder
                .add_reaction(Some("reaction"))
                .with_trigger(i2)
                .with_reaction_fn(|_ctx, _state, (i2,)| {
                    assert_eq!(*i2, Some(()));
                })
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let child2b = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("child2b", Some(parent), None, (), false);
            let i2 = builder.add_input_port::<()>("i2b").unwrap();
            let _ = builder
                .add_reaction(Some("reaction"))
                .with_trigger(i2)
                .with_reaction_fn(|_ctx, _state, (i2,)| {
                    assert_eq!(*i2, Some(()));
                })
                .finish()?;
            builder.finish()
        })
        .unwrap();

    let startup_key = builder.get_startup_action();
    let _main = builder.finish().unwrap();

    let i1 = env_builder
        .find_port_by_name::<(), Input>("i1", child1)
        .unwrap();
    let o1 = env_builder
        .find_port_by_name::<(), Output>("o1", child1)
        .unwrap();
    let i2a = env_builder
        .find_port_by_name::<(), Input>("i2a", child2a)
        .unwrap();
    let i2b = env_builder
        .find_port_by_name::<(), Input>("i2b", child2b)
        .unwrap();

    let _ = ReactorBuilderState::from_pre_existing(_main, &mut env_builder)
        .add_reaction(Some("start"))
        .with_trigger(startup_key)
        .with_effect(i1)
        .with_reaction_fn(|_ctx, _state: &mut (), (_startup, mut i1)| {
            println!("start");
            *i1 = Some(());
        })
        .finish()
        .unwrap();

    env_builder
        .add_port_connection::<(), _, _>(o1, i2a, None, false)
        .unwrap();
    env_builder
        .add_port_connection::<(), _, _>(o1, i2b, None, false)
        .unwrap();

    let BuilderRuntimeParts {
        enclaves, aliases, ..
    } = env_builder.into_runtime_parts(&runtime::Config::default()).unwrap();
    assert_eq!(enclaves.len(), 1);
    let (_enclave_key, enclave) = enclaves.into_iter().next().unwrap();
    assert_eq!(enclave.env.reactors.len(), 4);

    let _i1 = aliases.port_aliases[i1.into()].1;
    let o1 = aliases.port_aliases[o1.into()].1;
    let i2a = aliases.port_aliases[i2a.into()].1;
    let _i2b = aliases.port_aliases[i2b.into()].1;

    // Port o1 should alias to Port i2
    assert_eq!(enclave.env.ports.len(), 2);
    assert_eq!(o1, i2a);
}
