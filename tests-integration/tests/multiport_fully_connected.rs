use boomerang::builder::prelude::*;
use boomerang::{runtime, Reaction, Reactor};

struct State {
    received: bool,
}

#[derive(Reactor)]
#[reactor(
  state = "State",
  reaction = "ReactionStartup",
  //reaction = "ReactionIn",
  //reaction = "ReactionShutdown",
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
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        //println!("Hello from node {}!", ctx.reactor().bank_index);
        // broadcast my ID to everyone
        //ctx.set(&self.out, ctx.reactor().bank_index as i32);
    }
}

//#[derive(Reaction)]
//#[reaction(reactor = "Node<NUM_NODES>")]
struct ReactionIn<'a, const NUM_NODES: usize> {
    inp: [runtime::InputRef<'a, i32>; NUM_NODES],
}

impl<const NUM_NODES: usize> Trigger<Node<NUM_NODES>> for ReactionIn<'_, NUM_NODES> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        //println!("Node {} received messages from ", ctx.reactor().bank_index);
        let mut count = 0;

        for i in 0..self.inp.len() {
            if let Some(val) = *self.inp[i] {
                state.received = true;
                count += 1;
                print!("{val}, ");
            }
        }

        //if count != ctx.reactor().num_nodes {
        //    panic!("Received fewer messages than expected!");
        //}
    }
}

/*
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

impl<'a, const NUM_NODES: usize> ::boomerang::builder::Reaction<Node<NUM_NODES>>
    for ReactionIn<'a, NUM_NODES>
{
    fn build<'builder>(
        name: &str,
        reactor: &Node<NUM_NODES>,
        builder: &'builder mut ::boomerang::builder::ReactorBuilderState,
    ) -> Result<
        ::boomerang::builder::ReactionBuilderState<'builder>,
        ::boomerang::builder::BuilderError,
    > {
        #[allow(unused_variables)]
        fn __trigger_inner<'inner, const NUM_NODES: usize>(
            ctx: &mut ::boomerang::runtime::Context,
            state: &'inner mut dyn ::boomerang::runtime::ReactorState,
            ports: &'inner [::boomerang::runtime::PortRef<'inner>],
            ports_mut: &'inner mut [::boomerang::runtime::PortRefMut<'inner>],
            actions: &'inner mut [&'inner mut ::boomerang::runtime::Action],
        ) {
            let state: &mut <Node<NUM_NODES> as ::boomerang::builder::Reactor>::State = state
                .downcast_mut()
                .expect("Unable to downcast reactor state");
            let (inp,) = ::boomerang::runtime::partition(ports)
                .expect("Unable to destructure ref ports for reaction");
            <ReactionIn<NUM_NODES> as ::boomerang::builder::Trigger<Node<NUM_NODES>>>::trigger(
                ReactionIn { inp },
                ctx,
                state,
            );
        }
        let __startup_action = builder.get_startup_action();
        let __shutdown_action = builder.get_shutdown_action();
        let mut __reaction = builder.add_reaction(name, Box::new(__trigger_inner::<NUM_NODES>));

        let x = reactor.inp.map(From::from);

        <[runtime::InputRef<'a, i32>; NUM_NODES] as ::boomerang::builder::ReactionField>::build(
            &mut __reaction,
            //reactor.inp.into(),
            x,
            0,
            ::boomerang::builder::TriggerMode::TriggersAndUses,
        )?;
        Ok(__reaction)
    }
}
