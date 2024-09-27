use std::{iter, time::Duration};

use runtime::BasePort;

use super::*;
use crate::runtime;

/// Test semantics of trigger/effect/uses ports on reactions.
#[test]
fn test_reaction_ports() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder_a = env_builder.add_reactor("reactorA", None, None, ());
    let port_a = builder_a.add_input_port::<()>("portA").unwrap();
    let port_b = builder_a.add_output_port::<()>("portB").unwrap();
    let port_c = builder_a.add_input_port::<()>("portC").unwrap();
    let reaction_a = builder_a
        .add_reaction("reactionA", Box::new(|_, _, _, _, _| {}))
        .with_port(port_a, 0, TriggerMode::TriggersOnly)?
        .with_port(port_b, 0, TriggerMode::EffectsOnly)?
        .with_port(port_c, 0, TriggerMode::UsesOnly)?
        .finish()?;

    let (_env, triggers, aliases) = env_builder.into_runtime_parts().unwrap();

    // reactionA should "use" (be able to read from) portC
    itertools::assert_equal(
        triggers.reaction_use_ports[aliases.reaction_aliases[reaction_a]].iter(),
        iter::once(aliases.port_aliases[port_c.into()]),
    );

    // reactionA should "effect" (be able to write to) portB
    itertools::assert_equal(
        triggers.reaction_effect_ports[aliases.reaction_aliases[reaction_a]].iter(),
        iter::once(aliases.port_aliases[port_b.into()]),
    );

    // portA should trigger only reactionA
    itertools::assert_equal(
        triggers.port_triggers[aliases.port_aliases[port_a.into()]]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        iter::once(&aliases.reaction_aliases[reaction_a]),
    );

    // portB should not trigger any reactions
    itertools::assert_equal(
        triggers.port_triggers[aliases.port_aliases[port_b.into()]]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        iter::empty::<&runtime::ReactionKey>(),
    );

    // portC should not trigger any reactions
    itertools::assert_equal(
        triggers.port_triggers[aliases.port_aliases[port_c.into()]]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        iter::empty::<&runtime::ReactionKey>(),
    );

    Ok(())
}

/// Test that use-dependencies may be declared on logical actions and timers.
#[cfg(feature = "fixme")]
#[test]
fn test_dependency_use_on_logical_action() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder_main = env_builder.add_reactor("main", None, ());
    let clock = builder_main.add_logical_action::<u32>("clock", None)?;
    let a = builder_main.add_logical_action::<()>("a", None)?;
    let t = builder_main.add_timer(
        "t",
        TimerSpec {
            period: Some(Duration::from_millis(2)),
            offset: None,
        },
    )?;
    let startup_action = builder_main.get_startup_action();
    let shutdown_action = builder_main.get_shutdown_action();

    // reaction(startup) -> clock, a {= =}
    let r_startup = builder_main
        .add_reaction(
            "startup",
            Box::new(
                move |ctx: &mut runtime::Context,
                      _state: &mut dyn runtime::ReactorState,
                      ports: &[runtime::PortRef],
                      ports_mut: &mut [runtime::PortRefMut],
                      actions: &mut [&mut runtime::Action]| {
                    assert_eq!(ports.len(), 0);
                    assert_eq!(ports_mut.len(), 0);
                    assert_eq!(actions.len(), 3);

                    dbg!(&actions);

                    // destructure the actions array into the clock and a actions
                    let [startup, clock, a]: &mut [&mut _; 3] = actions.try_into().unwrap();

                    let mut clock: runtime::ActionRef<u32> = (*clock).into();
                    let mut a: runtime::ActionRef = (*a).into();

                    ctx.schedule_action(&mut a, None, Some(Duration::from_millis(3))); // out of order on purpose
                    ctx.schedule_action(&mut a, None, Some(Duration::from_millis(1)));
                    ctx.schedule_action(&mut a, None, Some(Duration::from_millis(5)));

                    // not scheduled on milli 1 (action is)
                    ctx.schedule_action(&mut clock, Some(2), Some(Duration::from_millis(2)));
                    ctx.schedule_action(&mut clock, Some(3), Some(Duration::from_millis(3)));
                    ctx.schedule_action(&mut clock, Some(4), Some(Duration::from_millis(4)));
                    ctx.schedule_action(&mut clock, Some(5), Some(Duration::from_millis(5)));
                    // not scheduled on milli 6 (timer is)
                },
            ),
        )
        .with_action(startup_action, 0, TriggerMode::TriggersAndUses)?
        .with_action(clock, 0, TriggerMode::EffectsOnly)?
        .with_action(a, 0, TriggerMode::EffectsOnly)?
        .finish()?;

    //reaction(clock) a, t {= =}
    let r_clock = builder_main
        .add_reaction(
            "clock",
            Box::new(
                |ctx: &mut runtime::Context,
                 state,
                 inputs,
                 outputs,
                 actions: &mut [&mut runtime::Action]| {
                    let [clock, a, t]: &mut [&mut _; 3] = actions.try_into().unwrap();

                    todo!();
                    /*
                    match ctx.get(clock) {
                        Some(2) | Some(4) => {
                            assert!(ctx.is_present(t)); // t is there on even millis
                            assert!(!ctx.is_present(a)); //
                        }
                        Some(3) | Some(5) => {
                            assert!(!ctx.is_present(t));
                            assert!(ctx.is_present(a));
                        }
                        it => unreachable!("{:?}", it),
                    }
                    self.tick += 1;
                    */
                },
            ),
        )
        .with_action(clock, 0, TriggerMode::TriggersAndUses)?
        .with_action(a, 0, TriggerMode::UsesOnly)?
        .with_action(t, 0, TriggerMode::UsesOnly)?
        .finish()?;

    // reaction(shutdown) {= =}
    let r_shutdown = builder_main
        .add_reaction(
            "shutdown",
            Box::new(|_, _, _, _, _| {
                /*
                assert_eq!(self.tick, 4);
                println!("success");
                 */
            }),
        )
        .with_action(shutdown_action, 0, TriggerMode::TriggersOnly)?
        .finish()?;

    let name = "test_dependency_use_on_logical_action";
    {
        let gv = graphviz::create_full_graph(&env_builder).unwrap();
        let path = format!("{name}.dot");
        let mut f = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote full graph to {path}");
    }

    {
        let gv = graphviz::create_reaction_graph(&env_builder).unwrap();
        let path = format!("{name}_levels.dot");
        let mut f = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();
        tracing::info!("Wrote reaction graph to {path}");
    }

    let (mut env, triggers, aliases) = env_builder.into_runtime_parts().unwrap();

    // r_startup should be triggered by the startup action, but the startup action should not be in its list of actions.
    let r_startup_runtime = aliases.reaction_aliases[r_startup];
    assert!(
        triggers.action_triggers[aliases.action_aliases[startup_action.into()]]
            .iter()
            .map(|(_, x)| x)
            .contains(&r_startup_runtime),
        "startup action should trigger r_startup"
    );
    itertools::assert_equal(
        env.reactions[r_startup_runtime].iter_actions(),
        &[
            aliases.action_aliases[startup_action.into()],
            aliases.action_aliases[clock.into()],
            aliases.action_aliases[a.into()],
        ],
    );

    let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
    sched.event_loop();

    Ok(())
}

/// Test that use-dependencies may be absent within a reaction.
#[test]
fn test_dependency_use_accessible() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let mut builder = env_builder.add_reactor("main", None, None, ());

    let source_reactor = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("Source", Some(parent), None, ());
            let clock = builder.add_output_port::<u32>("clock")?;
            let o1 = builder.add_output_port::<u32>("o1")?;
            let o2 = builder.add_output_port::<u32>("o2")?;
            let t1 = builder
                .add_timer(
                    "t1",
                    TimerSpec {
                        period: Some(Duration::from_millis(35)),
                        offset: None,
                    },
                )
                .unwrap();
            let t2 = builder
                .add_timer(
                    "t2",
                    TimerSpec {
                        period: Some(Duration::from_millis(70)),
                        offset: None,
                    },
                )
                .unwrap();
            let startup_action = builder.get_startup_action();
            let _ = builder
                .add_reaction(
                    "startup",
                    Box::new(
                        move |ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _ports: &[runtime::PortRef],
                              ports_mut: &mut [runtime::PortRefMut],
                              _actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 0);
                            let clock = ports_mut[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(0);

                            ctx.schedule_shutdown(Some(Duration::from_millis(140)));
                        },
                    ),
                )
                .with_action(startup_action, 0, TriggerMode::TriggersOnly)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            let _ = builder
                .add_reaction(
                    "reaction_t1",
                    Box::new(
                        move |_ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _ports: &[runtime::PortRef],
                              ports_mut: &mut [runtime::PortRefMut],
                              actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 1);
                            let clock = ports_mut[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(1);

                            //ctx.set(o1, 10);
                            let o1 = ports_mut[1]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(o1.get_name(), "o1");
                            **o1 = Some(10);

                            let t1 = &actions[0];
                            tracing::debug!(?t1, "t1");
                        },
                    ),
                )
                .with_action(t1, 0, TriggerMode::TriggersAndUses)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .with_port(o1, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            let _ = builder
                .add_reaction(
                    "reaction_t2",
                    Box::new(
                        move |_ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _ports: &[runtime::PortRef],
                              ports_mut: &mut [runtime::PortRefMut],
                              _actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 2);
                            let clock = ports_mut[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(2);

                            let o2 = ports_mut[1]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(o2.get_name(), "o2");
                            // we purposefully do not set o2
                        },
                    ),
                )
                .with_action(t2, 0, TriggerMode::TriggersOnly)?
                .with_port(clock, 0, TriggerMode::EffectsOnly)?
                .with_port(o2, 0, TriggerMode::EffectsOnly)?
                .finish()?;

            builder.finish()
        })
        .unwrap();

    let sink_reactor = builder.add_child_with(|parent, env| {
        let mut builder = env.add_reactor("Sink", Some(parent), None, ());
        let clock = builder.add_input_port::<u32>("clock").unwrap();
        let in1 = builder.add_input_port::<u32>("in1").unwrap();
        let in2 = builder.add_input_port::<u32>("in2").unwrap();
        let _ = builder
            .add_reaction(
                "reaction_clock",
                Box::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          ports: &[runtime::PortRef],
                          _ports_mut: &mut [runtime::PortRefMut],
                          _actions: &mut [&mut runtime::Action]| {
                        let clock = ports[0]
                            .as_any()
                            .downcast_ref::<runtime::Port<u32>>()
                            .unwrap();
                        assert_eq!(clock.get_name(), "clock");

                        let in1 = ports[1]
                            .as_any()
                            .downcast_ref::<runtime::Port<u32>>()
                            .unwrap();
                        assert_eq!(in1.get_name(), "o1");

                        let in2 = ports[2]
                            .as_any()
                            .downcast_ref::<runtime::Port<u32>>()
                            .unwrap();
                        assert_eq!(in2.get_name(), "o2");

                        match **clock {
                            Some(0) | Some(2) => {
                                assert_eq!(None, **in1);
                                assert_eq!(None, **in2);
                            }
                            Some(1) => {
                                assert_eq!(Some(10), **in1);
                                assert_eq!(None, **in2);
                            }
                            c => panic!("No such signal expected {:?}", c),
                        }
                    },
                ),
            )
            .with_port(clock, 0, TriggerMode::TriggersAndUses)?
            .with_port(in1, 0, TriggerMode::UsesOnly)?
            .with_port(in2, 0, TriggerMode::UsesOnly)?
            .finish()?;

        builder.finish()
    })?;

    let _main_reactor = builder.finish()?;

    let clock_source = env_builder.find_port_by_name("clock", source_reactor)?;
    let clock_sink = env_builder.find_port_by_name("clock", sink_reactor)?;
    env_builder.bind_port(clock_source, clock_sink)?;

    let o1_source = env_builder.find_port_by_name("o1", source_reactor)?;
    let in1_sink = env_builder.find_port_by_name("in1", sink_reactor)?;
    env_builder.bind_port(o1_source, in1_sink)?;

    let o2_source = env_builder.find_port_by_name("o2", source_reactor)?;
    let in2_sink = env_builder.find_port_by_name("in2", sink_reactor)?;
    env_builder.bind_port(o2_source, in2_sink)?;

    /*
    reactor Source {
      reaction(startup) -> clock {= ctx.set(clock, 0); =}

      reaction(t1) -> clock, o1 {= ctx.set(clock, 1); ctx.set(o1, 10) =}

      // has a dependency but doesn't use it
      reaction(t2) -> clock, o2 {= ctx.set(clock, 2); =}
    }

    reactor Sink {
      input clock: u32
      input in1: u32
      input in2: u32

      reaction(clock) in1, in2 {= =}
    }
    */

    let reaction_source_startup_key =
        env_builder.find_reaction_by_name("startup", source_reactor)?;
    let _reaction_source_t1_key =
        env_builder.find_reaction_by_name("reaction_t1", source_reactor)?;
    let _reaction_source_t2_key =
        env_builder.find_reaction_by_name("reaction_t2", source_reactor)?;
    let reaction_sink_clock_key =
        env_builder.find_reaction_by_name("reaction_clock", sink_reactor)?;

    let (mut env, triggers, aliases) = env_builder.into_runtime_parts()?;

    // the Source startup reaction should trigger on startup and effect the clock port
    let runtime_reaction_source_startup_key = aliases.reaction_aliases[reaction_source_startup_key];
    itertools::assert_equal(
        triggers.reaction_effect_ports[runtime_reaction_source_startup_key].iter(),
        [aliases.port_aliases[clock_source]],
    );

    let runtime_reaction_sink_clock_key = aliases.reaction_aliases[reaction_sink_clock_key];

    // The clock reaction should only be triggered by the `clock` port, not the `in1` or `in2` ports.
    itertools::assert_equal(
        triggers.port_triggers[aliases.port_aliases[clock_sink]]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        &[runtime_reaction_sink_clock_key],
    );

    // The clock reaction should have the `clock`, `in1`, and `in2` ports as use ports.
    itertools::assert_equal(
        triggers.reaction_use_ports[runtime_reaction_sink_clock_key].iter(),
        [
            aliases.port_aliases[clock_source],
            aliases.port_aliases[in1_sink],
            aliases.port_aliases[in2_sink],
        ],
    );

    // The clock reaction should not have any effect ports.
    itertools::assert_equal(
        triggers.reaction_effect_ports[runtime_reaction_sink_clock_key].iter(),
        [],
    );

    let mut sched = runtime::Scheduler::new(env, triggers, true, false);
    sched.event_loop();

    Ok(())
}
