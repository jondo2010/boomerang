//! test a connection from a multiport to another multiport of the same reactor

use boomerang::prelude::*;

#[reactor]
fn Node<const NUM_NODES: usize>(
    //#[state] size: usize,
    #[input] input: [i32; NUM_NODES],
    #[output] output: [i32; NUM_NODES],
) -> impl Reactor {
    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(output)
        .with_reaction_fn(|_ctx, _state, (_startup, mut output)| {
            for (i, out) in output.iter_mut().enumerate() {
                **out = Some(i as i32);
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("In"))
        .with_trigger(input)
        .with_reaction_fn(|_ctx, _state, (input,)| {
            let count = input.iter().filter(|x| x.is_some()).count();
            assert_eq!(count, NUM_NODES);
            println!("success");
        })
        .finish()?;
}

#[reactor]
fn Main<const NUM_NODES: usize>() -> impl Reactor {
    let nodes: [_; NUM_NODES] =
        builder.add_child_reactors(Node::<NUM_NODES>(), "nodes", Default::default(), false)?;
    builder.connect_ports(
        nodes.iter().flat_map(|child| child.output.iter().copied()),
        nodes.iter().flat_map(|child| child.input.iter().copied()),
        None,
        false,
    )?;
}

#[test]
fn action_values() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        Main::<4>(),
        "connection_to_self_multiport",
        (),
        config,
    )
    .unwrap();
}
