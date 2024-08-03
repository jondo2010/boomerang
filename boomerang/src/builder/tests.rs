use std::{io::sink, time::Duration};

use super::*;
use crate::runtime;

#[test]
fn test1() {
    // The triggers field can be a comma-separated list of input ports, output ports of contained reactors, timers, actions, or the special events startup, shutdown, and reset (explained here>). There must be at least one trigger for each reaction.
    // The uses field, which is optional, specifies input ports (or output ports of contained reactors) that do not trigger execution of the reaction but may be read by the reaction.
    // The effects field, which is also optional, is a comma-separated lists of output ports ports, input ports of contained reactors, or actions.

    let mut env_builder = EnvBuilder::new();
    let mut builder_a = env_builder.add_reactor("reactorA", None, ());
    let port_a = builder_a.add_port::<()>("portA", PortType::Input).unwrap();
    let port_b = builder_a.add_port::<()>("portB", PortType::Output).unwrap();
    let reaction_a = builder_a
        .add_reaction("reactionA", Box::new(|_, _, _, _, _| {}))
        .with_trigger_port(port_a, 0)
        .with_effect_port(port_b, 0)
        .finish()
        .unwrap();
}

/// Test that use-dependencies may be declared on logical actions and timers.
#[test]
fn test_dependency_use_on_logical_action() {
    let mut env_builder = EnvBuilder::new();
    let mut builder_main = env_builder.add_reactor("main", None, ());
    let clock = builder_main
        .add_logical_action::<u32>("clock", None)
        .unwrap();
    let a = builder_main.add_logical_action::<()>("a", None).unwrap();
    let t = builder_main
        .add_timer("t", Some(Duration::from_millis(2)), None)
        .unwrap();
    let startup_action = builder_main.get_startup_action();
    let shutdown_action = builder_main.get_shutdown_action();

    // reaction(startup) -> clock, a {= =}
    let r_startup = builder_main
        .add_reaction(
            "startup",
            Box::new(
                move |ctx: &mut runtime::Context,
                      state: &mut dyn runtime::ReactorState,
                      inputs: &[runtime::IPort],
                      outputs: &mut [runtime::OPort],
                      actions: &mut [&mut runtime::Action]| {
                    assert_eq!(inputs.len(), 0);
                    assert_eq!(outputs.len(), 0);
                    assert_eq!(actions.len(), 2);
                    /*
                    ctx.schedule(a, after!(3 ms)); // out of order on purpose
                    ctx.schedule(a, after!(1 ms));
                    ctx.schedule(a, after!(5 ms));

                    // not scheduled on milli 1 (action is)
                    ctx.schedule_with_v(clock, Some(2), after!(2 ms));
                    ctx.schedule_with_v(clock, Some(3), after!(3 ms));
                    ctx.schedule_with_v(clock, Some(4), after!(4 ms));
                    ctx.schedule_with_v(clock, Some(5), after!(5 ms));
                    // not scheduled on milli 6 (timer is)
                         */
                },
            ),
        )
        .with_trigger_action(startup_action, 0)
        .with_effect_action(clock, 0)
        .with_effect_action(a, 0)
        .finish()
        .unwrap();

    //reaction(clock) a, t {= =}
    let r_clock = builder_main
        .add_reaction(
            "clock",
            Box::new(|_, _, _, _, _| {
                /*
                match ctx.get(clock) {
                  Some(2) | Some(4) => {
                    assert!(ctx.is_present(t));   // t is there on even millis
                    assert!(!ctx.is_present(a)); //
                  },
                  Some(3) | Some(5) => {
                    assert!(!ctx.is_present(t));
                    assert!(ctx.is_present(a));
                  },
                  it => unreachable!("{:?}", it)
                }
                self.tick += 1;
                     */
            }),
        )
        .with_trigger_action(clock, 0)
        .with_uses_action(a, 0)
        .with_uses_action(t, 0)
        .finish()
        .unwrap();

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
        .with_trigger_action(shutdown_action, 0)
        .finish()
        .unwrap();

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

    println!("{env_builder:#?}");
    let (env, _) = env_builder.try_into().unwrap();
    runtime::util::print_debug_info(&env);
}

/// Test that use-dependencies may be absent within a reaction.
#[test]
fn test_dependency_use_accessible() {
    let mut env_builder = EnvBuilder::new();

    let mut builder = env_builder.add_reactor("main", None, ());

    let source_reactor = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("Source", Some(parent), ());
            let clock = builder.add_port::<()>("clock", PortType::Output).unwrap();
            let o1 = builder.add_port::<()>("o1", PortType::Output).unwrap();
            let o2 = builder.add_port::<()>("o2", PortType::Output).unwrap();
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
                              state: &mut dyn runtime::ReactorState,
                              inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 0);
                        },
                    ),
                )
                .with_trigger_action(startup_action, 0)
                .with_effect_port(clock, 0)
                .finish()
                .unwrap();
            let _ = builder
                .add_reaction(
                    "reaction_t1",
                    Box::new(
                        move |ctx: &mut runtime::Context,
                              state: &mut dyn runtime::ReactorState,
                              inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 1);
                            //ctx.set(o1, 10);
                        },
                    ),
                )
                .with_trigger_action(t1, 0)
                .with_effect_port(clock, 0)
                .with_effect_port(o1, 0)
                .finish()
                .unwrap();
            let _ = builder
                .add_reaction(
                    "reaction_t2",
                    Box::new(
                        move |ctx: &mut runtime::Context,
                              state: &mut dyn runtime::ReactorState,
                              inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              actions: &mut [&mut runtime::Action]| {
                            //ctx.set(clock, 2);
                        },
                    ),
                )
                .with_trigger_action(t2, 0)
                .with_effect_port(clock, 0)
                .with_effect_port(o2, 0)
                .finish()
                .unwrap();

            builder.finish()
        })
        .unwrap();

    let sink_reactor = builder
        .add_child_with(|parent, env| {
            let mut builder = env.add_reactor("Sink", Some(parent), ());
            let clock = builder.add_port::<()>("clock", PortType::Input).unwrap();
            let in1 = builder.add_port::<u32>("in1", PortType::Input).unwrap();
            let in2 = builder.add_port::<u32>("in2", PortType::Input).unwrap();
            let _ = builder
                .add_reaction(
                    "reaction_clock",
                    Box::new(
                        move |ctx: &mut runtime::Context,
                              state: &mut dyn runtime::ReactorState,
                              inputs: &[runtime::IPort],
                              outputs: &mut [runtime::OPort],
                              actions: &mut [&mut runtime::Action]| {
                            /*
                            match ctx.get(clock) {
                                Some(0) | Some(2) => {
                                    assert_eq!(None, ctx.get(in1));
                                    assert_eq!(None, ctx.get(in2));
                                }
                                Some(1) => {
                                    assert_eq!(Some(10), ctx.get(in1));
                                    assert_eq!(None, ctx.get(in2));
                                }
                                c => panic!("No such signal expected {:?}", c),
                            }
                            */
                        },
                    ),
                )
                .with_trigger_port(clock, 0)
                .with_uses_port(in1, 0)
                .with_uses_port(in2, 0)
                .finish()
                .unwrap();

            builder.finish()
        })
        .unwrap();

    let _main_reactor = builder.finish().unwrap();

    let clock_source = env_builder.get_port("clock", source_reactor).unwrap();
    let clock_sink = env_builder.get_port("clock", sink_reactor).unwrap();
    env_builder.bind_port(clock_source, clock_sink).unwrap();

    let o1_source = env_builder.get_port("o1", source_reactor).unwrap();
    let in1_sink = env_builder.get_port("in1", sink_reactor).unwrap();
    env_builder.bind_port(o1_source, in1_sink).unwrap();

    let o2_source = env_builder.get_port("o2", source_reactor).unwrap();
    let in2_sink = env_builder.get_port("in2", sink_reactor).unwrap();
    env_builder.bind_port(o2_source, in2_sink).unwrap();

    /*
    reactor Source {
      reaction(startup) -> clock {=
        ctx.set(clock, 0);
      =}

      reaction(t1) -> clock, o1 {=
        ctx.set(clock, 1); ctx.set(o1, 10)
      =}

      // has a dependency but doesn't use it
      reaction(t2) -> clock, o2 {=
        ctx.set(clock, 2);
      =}
    }

    reactor Sink {
      input clock: u32
      input in1: u32
      input in2: u32

      reaction(clock) in1, in2 {= =}
    }

         */

    let reaction_clock_key = env_builder.get_reaction("reaction_clock", sink_reactor).unwrap();

    println!("{env_builder:#?}");
    let (env, aliases) = env_builder.try_into().unwrap();
    runtime::util::print_debug_info(&env);

    let runtime_reaction_clock_key = aliases.reaction_aliases.get(reaction_clock_key).unwrap();
    let reaction_clock = &env.reactions[*runtime_reaction_clock_key];

    let x = reaction_clock.iter_use_ports().collect::<Vec<_>>();
    println!("found: {}", rr.get_name());
    
}
