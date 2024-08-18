use std::{iter, time::Duration};

use itertools::Itertools;

use super::*;
use crate::runtime;

/// Test semantics of trigger/effect/uses ports on reactions.
#[test]
fn test_reaction_ports() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder_a = env_builder.add_reactor("reactorA", None, ());
    let port_a = builder_a.add_port::<()>("portA", PortType::Input).unwrap();
    let port_b = builder_a.add_port::<()>("portB", PortType::Output).unwrap();
    let port_c = builder_a.add_port::<()>("portC", PortType::Input).unwrap();
    let reaction_a = builder_a
        .add_reaction("reactionA", Box::new(|_, _, _, _, _| {}))
        .with_trigger_port(port_a, 0)?
        .with_effect_port(port_b, 0)?
        .with_uses_port(port_c, 0)?
        .finish()?;

    let (env, triggers, aliases) = env_builder.into_runtime_parts().unwrap();

    // reactionA should "use" (be able to read from) portA and portC
    itertools::assert_equal(
        env.reactions[aliases.reaction_aliases[reaction_a]].iter_use_ports(),
        &[
            aliases.port_aliases[port_a.into()],
            aliases.port_aliases[port_c.into()],
        ],
    );

    // reactionA should "effect" (be able to write to) portB
    itertools::assert_equal(
        env.reactions[aliases.reaction_aliases[reaction_a]].iter_effect_ports(),
        &[aliases.port_aliases[port_b.into()]],
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
#[test]
fn test_dependency_use_on_logical_action() -> anyhow::Result<()> {
    let mut env_builder = EnvBuilder::new();
    let mut builder_main = env_builder.add_reactor("main", None, ());
    let clock = builder_main.add_logical_action::<u32>("clock", None)?;
    let a = builder_main.add_logical_action::<()>("a", None)?;
    let t = builder_main.add_timer("t", Some(Duration::from_millis(2)), None)?;
    let startup_action = builder_main.get_startup_action();
    let shutdown_action = builder_main.get_shutdown_action();

    // reaction(startup) -> clock, a {= =}
    let r_startup = builder_main
        .add_reaction(
            "startup",
            Box::new(
                move |ctx: &mut runtime::Context,
                      _state: &mut dyn runtime::ReactorState,
                      inputs: &[runtime::IPort],
                      outputs: &mut [runtime::OPort],
                      actions: &mut [&mut runtime::Action]| {
                    assert_eq!(inputs.len(), 0);
                    assert_eq!(outputs.len(), 0);
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
        .with_trigger_action(startup_action, 0)?
        .with_effect_action(clock, 0)?
        .with_effect_action(a, 0)?
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
        .with_trigger_action(clock, 0)?
        .with_uses_action(a, 0)?
        .with_uses_action(t, 0)?
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
        .with_trigger_action(shutdown_action, 0)?
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

    let mut builder = env_builder.add_reactor("main", None, ());

    let source_reactor = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("Source", Some(parent), ());
            let clock = builder.add_port::<u32>("clock", PortType::Output).unwrap();
            let o1 = builder.add_port::<u32>("o1", PortType::Output).unwrap();
            let o2 = builder.add_port::<u32>("o2", PortType::Output).unwrap();
            let t1 = builder
                .add_timer("t1", Some(Duration::from_millis(35)), None)
                .unwrap();
            let t2 = builder
                .add_timer("t2", Some(Duration::from_millis(70)), None)
                .unwrap();
            let startup_action = builder.get_startup_action();
            let _ = builder
                .add_reaction(
                    "startup",
                    Box::new(
                        move |ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              _actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 0);
                            let clock = outputs[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(0);

                            ctx.schedule_shutdown(Some(Duration::from_millis(140)));
                        },
                    ),
                )
                .with_trigger_action(startup_action, 0)?
                .with_effect_port(clock, 0)?
                .finish()?;

            let _ = builder
                .add_reaction(
                    "reaction_t1",
                    Box::new(
                        move |_ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 1);
                            let clock = outputs[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(1);

                            //ctx.set(o1, 10);
                            let o1 = outputs[1]
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
                .with_trigger_action(t1, 0)?
                .with_effect_port(clock, 0)?
                .with_effect_port(o1, 0)?
                .finish()?;

            let _ = builder
                .add_reaction(
                    "reaction_t2",
                    Box::new(
                        move |_ctx: &mut runtime::Context,
                              _state: &mut dyn runtime::ReactorState,
                              _inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              _actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 2);
                            let clock = outputs[0]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(clock.get_name(), "clock");
                            **clock = Some(2);

                            let o2 = outputs[1]
                                .as_any_mut()
                                .downcast_mut::<runtime::Port<u32>>()
                                .unwrap();
                            assert_eq!(o2.get_name(), "o2");
                            // we purposefully do not set o2
                        },
                    ),
                )
                .with_trigger_action(t2, 0)?
                .with_effect_port(clock, 0)?
                .with_effect_port(o2, 0)?
                .finish()?;

            builder.finish()
        })
        .unwrap();

    let sink_reactor = builder.add_child_with(|parent, env| {
        let mut builder = env.add_reactor("Sink", Some(parent), ());
        let clock = builder.add_port::<u32>("clock", PortType::Input).unwrap();
        let in1 = builder.add_port::<u32>("in1", PortType::Input).unwrap();
        let in2 = builder.add_port::<u32>("in2", PortType::Input).unwrap();
        let _ = builder
            .add_reaction(
                "reaction_clock",
                Box::new(
                    move |_ctx: &mut runtime::Context,
                          _state: &mut dyn runtime::ReactorState,
                          inputs: &[runtime::IPort],
                          _outputs: &mut [runtime::OPort],
                          _actions: &mut [&mut runtime::Action]| {
                        let clock = inputs[0]
                            .as_any()
                            .downcast_ref::<runtime::Port<u32>>()
                            .unwrap();
                        assert_eq!(clock.get_name(), "clock");

                        let in1 = inputs[1]
                            .as_any()
                            .downcast_ref::<runtime::Port<u32>>()
                            .unwrap();
                        assert_eq!(in1.get_name(), "o1");

                        let in2 = inputs[2]
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
            .with_trigger_port(clock, 0)?
            .with_uses_port(in1, 0)?
            .with_uses_port(in2, 0)?
            .finish()?;

        builder.finish()
    })?;

    let _main_reactor = builder.finish()?;

    let clock_source = env_builder.get_port("clock", source_reactor)?;
    let clock_sink = env_builder.get_port("clock", sink_reactor)?;
    env_builder.bind_port(clock_source, clock_sink)?;

    let o1_source = env_builder.get_port("o1", source_reactor)?;
    let in1_sink = env_builder.get_port("in1", sink_reactor)?;
    env_builder.bind_port(o1_source, in1_sink)?;

    let o2_source = env_builder.get_port("o2", source_reactor)?;
    let in2_sink = env_builder.get_port("in2", sink_reactor)?;
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

    let reaction_clock_key = env_builder.get_reaction("reaction_clock", sink_reactor)?;

    let (mut env, triggers, aliases) = env_builder.into_runtime_parts()?;

    let runtime_reaction_clock_key = aliases.reaction_aliases[reaction_clock_key];
    let reaction_clock = &env.reactions[runtime_reaction_clock_key];

    let runtime_clock_port = aliases.port_aliases[clock_sink];

    // The clock reaction should only be triggered by the `clock` port, not the `in1` or `in2` ports.
    itertools::assert_equal(
        triggers.port_triggers[runtime_clock_port]
            .iter()
            .map(|(_, reaction_key)| reaction_key),
        &[runtime_reaction_clock_key],
    );

    // The clock reaction should have the `clock`, `in1`, and `in2` ports as use ports.
    itertools::assert_equal(
        reaction_clock.iter_use_ports(),
        &[
            aliases.port_aliases[clock_source],
            aliases.port_aliases[in1_sink],
            aliases.port_aliases[in2_sink],
        ],
    );

    // The clock reaction should not have any effect ports.
    itertools::assert_equal(reaction_clock.iter_effect_ports(), &[]);

    let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
    sched.event_loop();

    Ok(())
}
