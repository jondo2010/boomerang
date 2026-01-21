//! In this pattern, each node can send direct messages to individual other nodes
//!
//! Ported from <https://github.com/lf-lang/lingua-franca/blob/master/test/Rust/src/multiport/FullyConnectedAddressable.lf>

use boomerang::prelude::*;

#[reactor]
fn Node<const NUM_NODES: usize>(
    #[state] received: bool,
    #[input] input: [i32; NUM_NODES],
    #[output] output: [i32; NUM_NODES],
) -> impl Reactor {
    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(output)
        .with_reaction_fn(|ctx, _state, (_startup, mut output)| {
            let bank_index = ctx.get_bank_index().unwrap();
            println!("Hello from node {}!", bank_index);
            // send my ID only to my right neighbour
            *output[(bank_index + 1) % NUM_NODES] = Some(bank_index as i32);
        })
        .finish()?;

    builder
        .add_reaction(Some("Input"))
        .with_trigger(input)
        .with_reaction_fn(|ctx, state, (input,)| {
            let bank_index = ctx.get_bank_index().unwrap();

            //received = true;
            state.received = true;

            let mut count = 0;
            let mut result = 0;
            let mut nodes = vec![];
            for port in input {
                if let Some(v) = *port {
                    count += 1;
                    result = v;
                    nodes.push(v.to_string());
                }
            }

            println!(
                "Node {bank_index} received messages from {}",
                nodes.join(" ")
            );

            let expected = if bank_index == 0 {
                ctx.get_bank_total().unwrap_or_default() - 1
            } else {
                bank_index - 1
            };
            assert!(
                count == 1 && result as usize == expected,
                "ERROR: received an unexpected message!"
            );
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert!(state.received, "Received no input!");
        })
        .finish()?;
}

#[reactor]
fn Main<const NUM_NODES: usize = 4>() -> impl Reactor {
    let nodes1: [_; NUM_NODES] =
        builder.add_child_reactors(Node::<NUM_NODES>(), "nodes1", Default::default(), false)?;
    let nodes2: [_; NUM_NODES] =
        builder.add_child_reactors(Node::<NUM_NODES>(), "nodes2", Default::default(), false)?;

    builder.connect_ports(
        nodes1.iter().flat_map(|child| child.output.iter()).copied(),
        nodes2
            .iter()
            .map(|child| child.input.iter())
            .flatten_transposed()
            .copied(),
        None,
        false,
    )?;

    builder.connect_ports(
        nodes1
            .iter()
            .map(|child| child.output.iter())
            .flatten_transposed()
            .copied(),
        nodes1.iter().flat_map(|child| child.input.iter()).copied(),
        None,
        false,
    )?;
}

#[test]
fn fully_connected_addressable() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        Main::<4>(),
        "fully_connected_addressable",
        (),
        config,
    )
    .unwrap();
}
