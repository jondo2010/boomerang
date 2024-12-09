//! In this pattern, each node can send direct messages to individual other nodes
//!
//! Ported from <https://github.com/lf-lang/lingua-franca/blob/master/test/Rust/src/multiport/FullyConnectedAddressable.lf>

use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(
    state = "bool",
    reaction = "ReactionStartup<NUM_NODES>",
    reaction = "ReactionInput<NUM_NODES>",
    reaction = "ReactionShutdown"
)]
struct Node<const NUM_NODES: usize> {
    input: [TypedPortKey<i32, Input>; NUM_NODES],
    output: [TypedPortKey<i32, Output>; NUM_NODES],
}

#[derive(Reaction)]
#[reaction(reactor = "Node<NUM_NODES>", triggers(startup))]
struct ReactionStartup<'a, const NUM_NODES: usize> {
    output: [runtime::OutputRef<'a, i32>; NUM_NODES],
}

impl<const NUM_NODES: usize> runtime::Trigger<bool> for ReactionStartup<'_, NUM_NODES> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut bool) {
        let bank_index = ctx.get_bank_index().unwrap();
        println!("Hello from node {}!", bank_index);
        // send my ID only to my right neighbour
        *self.output[(bank_index + 1) % NUM_NODES] = Some(bank_index as i32);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Node<NUM_NODES>")]
struct ReactionInput<'a, const NUM_NODES: usize> {
    input: [runtime::InputRef<'a, i32>; NUM_NODES],
}

impl<const NUM_NODES: usize> runtime::Trigger<bool> for ReactionInput<'_, NUM_NODES> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut bool) {
        let bank_index = ctx.get_bank_index().unwrap();

        //received = true;
        *state = true;

        let mut count = 0;
        let mut result = 0;
        let mut nodes = vec![];
        for port in self.input {
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
    }
}

#[derive(Reaction)]
#[reaction(
    reactor = "Node<NUM_NODES>",
    bound = "const NUM_NODES: usize",
    triggers(shutdown)
)]
struct ReactionShutdown;

impl runtime::Trigger<bool> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut bool) {
        assert!(*state, "Error: received no input!");
    }
}

#[derive(Reactor)]
#[reactor(
    state = (),
    connection(from = "nodes1.output", to = "transposed(nodes2.input)"),
    connection(from = "transposed(nodes2.output)", to = "nodes1.input")
)]
struct Main<const NUM_NODES: usize = 4> {
    #[reactor(child(state = false))]
    nodes1: [Node<NUM_NODES>; NUM_NODES],
    #[reactor(child(state = false))]
    nodes2: [Node<NUM_NODES>; NUM_NODES],
}

#[test]
fn fully_connected_addressable() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<Main>(
        "fully_connected_addressable",
        (),
        config,
    )
    .unwrap();
}
