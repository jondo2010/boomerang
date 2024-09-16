use boomerang::builder::prelude::*;
use boomerang::{runtime, Reaction, Reactor};

struct State {
    received: bool,
}

#[derive(Reactor)]
#[reactor(
  state = "State",
  //reaction = "ReactionStartup"
)]
struct Node<const NUM_NODES: usize> {
    inp: [TypedPortKey<i32, Input>; NUM_NODES],
    out: TypedPortKey<i32, Output>,
    //reaction_in: TypedReactionKey<ReactionIn<'static>>,
    //reaction_shutdown: TypedReactionKey<ReactionShutdown>,
}

#[derive(Reaction)]
#[reaction(
    reactor = "Node<NUM_NODES>",
    //bound = "const NUM_NODES: usize",
    triggers(startup)
)]
struct ReactionStartup<'a> {
    out: runtime::OutputRef<'a, i32>,
}

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionStartup<'_> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        //println!("Hello from node {}!", ctx.reactor().bank_index);
        // broadcast my ID to everyone
        //ctx.set(&self.out, ctx.reactor().bank_index as i32);
    }
}

/*
#[derive(Reaction)]
struct ReactionIn<'a> {
    inp: &'a [runtime::Port<i32>],
}

impl Trigger for ReactionIn<'_> {
    type Reactor = Node<4>;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut State) {
        /*
        println!("Node {} received messages from ", ctx.reactor().bank_index);
        let mut count = 0;
        for i in 0..self.inp.len() {
            if self.inp[i].is_present {
                state.received = true;
                count += 1;
                print!("{}, ", self.inp[i].value);
            }
        }
        println!();
        if count != ctx.reactor().num_nodes {
            panic!("Received fewer messages than expected!");
        }
        */
    }
}

#[derive(Reaction)]
struct ReactionShutdown;

impl Trigger for ReactionShutdown {
    type Reactor = Node<4>;

    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut State) {
        assert!(state.received, "Received no input!");
    }
}

#[derive(Reactor)]
#[reactor(
state = (),
)]
struct MainReactor<const NUM_NODES: usize> {
    nodes: [Node<NUM_NODES>; NUM_NODES],
}
 */

/*
reactor Node(num_nodes: size_t = 4, bank_index: int = 0) {
    input[num_nodes] in: int
    output out: int

    state received: bool = false

    reaction(startup) -> out {=
      lf_print("Hello from node %d!", self->bank_index);
      // broadcast my ID to everyone
      lf_set(out, self->bank_index);
    =}

    reaction(in) {=
      printf("Node %d received messages from ", self->bank_index);
      size_t count = 0;
      for (int i = 0; i < in_width; i++) {
        if (in[i]->is_present) {
          self->received = true;
          count++;
          printf("%d, ", in[i]->value);
        }
      }
      printf("\n");
      if (count != self->num_nodes) {
        lf_print_error_and_exit("Received fewer messages than expected!");
      }
    =}

    reaction(shutdown) {=
      if (!self->received) {
        lf_print_error_and_exit("Received no input!");
      }
    =}
  }

  main reactor(num_nodes: size_t = 4) {
    nodes = new[num_nodes] Node(num_nodes=num_nodes)
    (nodes.out)+ -> nodes.in
  }
*/
