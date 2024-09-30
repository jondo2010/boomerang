use boomerang::prelude::*;

#[derive(Clone)]
struct State {
    received: bool,
}

#[derive(Reactor)]
#[reactor(
    state = "State",
    reaction = "ReactionStartup",
    reaction = "ReactionIn<NUM_NODES>",
    reaction = "ReactionShutdown"
)]
struct Node<const NUM_NODES: usize> {
    inp: [TypedPortKey<i32, Input>; NUM_NODES],
    out: TypedPortKey<i32, Output>,
}

#[derive(Reaction)]
#[reaction(
    reactor = "Node<NUM_NODES>",
    bound = "const NUM_NODES: usize",
    triggers(startup)
)]
struct ReactionStartup<'a> {
    out: runtime::OutputRef<'a, i32>,
}

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionStartup<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, _state: &mut State) {
        println!("Hello from node {}!", ctx.get_bank_index().unwrap());
        // broadcast my ID to everyone
        *self.out = ctx.get_bank_index().map(|x| x as i32);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Node<NUM_NODES>")]
struct ReactionIn<'a, const NUM_NODES: usize> {
    inp: [runtime::InputRef<'a, i32>; NUM_NODES],
}

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionIn<'_, NUM_NODES> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut State) {
        let mut count = 0;
        let mut vals = vec![];
        for p in &self.inp {
            if let Some(val) = **p {
                state.received = true;
                count += 1;
                vals.push(val.to_string());
            }
        }

        println!(
            "Node {} received messages from {}.",
            ctx.get_bank_index().unwrap(),
            vals.join(" "),
        );

        assert_eq!(
            Some(count),
            ctx.get_bank_total(),
            "Received fewer messages than expected!"
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

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionShutdown {
    fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
        assert!(state.received, "Received no input!");
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "nodes.out", to = "nodes.inp", broadcast)
)]
struct MainReactor<const NUM_NODES: usize = 4> {
    #[reactor(child = "State{received: false}")]
    nodes: [Node<NUM_NODES>; NUM_NODES],
}

#[test]
fn mutliport_fully_connected() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<MainReactor>(
        "multiport_fully_connected",
        (),
        config,
    )
    .unwrap();
}
