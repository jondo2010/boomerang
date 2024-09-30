//! test a connection from a multiport to another multiport of the same reactor

use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(
    state = "usize",
    reaction = "ReactionStartup<NUM_NODES>",
    reaction = "ReactionIn<NUM_NODES>"
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

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionStartup<'_, NUM_NODES> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut usize) {
        for (i, out) in self.output.iter_mut().enumerate() {
            **out = Some(i as i32);
        }
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Node<NUM_NODES>")]
struct ReactionIn<'a, const NUM_NODES: usize> {
    input: [runtime::InputRef<'a, i32>; NUM_NODES],
}

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionIn<'_, NUM_NODES> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut usize) {
        let count = self.input.iter().filter(|x| x.is_some()).count();
        assert_eq!(count, NUM_NODES);
        println!("success")
    }
}

#[derive(Reactor)]
#[reactor(state = "()", connection(from = "nodes.output", to = "nodes.input"))]
struct Main<const NUM_NODES: usize> {
    #[reactor(child = "NUM_NODES")]
    nodes: Node<NUM_NODES>,
}

#[test]
fn action_values() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<Main<4>>(
        "connection_to_self_multiport",
        (),
        config,
    )
    .unwrap();
}
