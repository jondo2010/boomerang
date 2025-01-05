//! Check multiport capabilities on Outputs.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/concurrent/ThreadedMultiport.lf

use boomerang::prelude::*;

#[derive(Debug)]
pub struct State {
    s: i32,
}

mod source {
    use super::*;

    #[derive(Reactor)]
    #[reactor(state = "State", reaction = "ReactionT<WIDTH>")]
    pub struct Source<const WIDTH: usize> {
        #[reactor(timer(period = "200 msec"))]
        t: TimerActionKey,
        pub out: [TypedPortKey<i32, Output>; WIDTH],
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Source<WIDTH>", triggers(action = "t"))]
    struct ReactionT<'a, const WIDTH: usize> {
        out: [runtime::OutputRef<'a, i32>; WIDTH],
    }

    impl<const WIDTH: usize> runtime::Trigger<State> for ReactionT<'_, WIDTH> {
        fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut State) {
            for o in self.out.iter_mut() {
                **o = Some(state.s);
            }
            state.s += 1;
        }
    }
}

mod computation {
    use super::*;

    #[derive(Reactor, Debug)]
    #[reactor(state = "usize", reaction = "ReactionIn")]
    pub struct Computation<const ITERS: usize> {
        pub in_: TypedPortKey<i32, Input>,
        pub out: TypedPortKey<i32, Output>,
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Computation<ITERS>", bound = "const ITERS: usize")]
    struct ReactionIn<'a> {
        in_: runtime::InputRef<'a, i32>,
        out: runtime::OutputRef<'a, i32>,
    }

    impl runtime::Trigger<usize> for ReactionIn<'_> {
        fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut usize) {
            let mut offset = 0;
            for _ in 0..*state {
                offset += 1;
                //std::thread::sleep(std::time::Duration::nanosecondss(1));
            }
            *self.out = self.in_.map(|x| x + offset);
        }
    }
}

mod destination {
    use super::*;

    #[derive(Reactor)]
    #[reactor(
        state = "State",
        reaction = "ReactionIn<WIDTH, ITERS>",
        reaction = "ReactionShutdown"
    )]
    pub struct Destination<const WIDTH: usize, const ITERS: usize = 100_000_000> {
        pub in_: [TypedPortKey<i32, Input>; WIDTH],
    }

    #[derive(Reaction)]
    #[reaction(reactor = "Destination<WIDTH, ITERS>")]
    struct ReactionIn<'a, const WIDTH: usize, const ITERS: usize> {
        in_: [runtime::InputRef<'a, i32>; WIDTH],
    }

    impl<const WIDTH: usize, const ITERS: usize> runtime::Trigger<State>
        for ReactionIn<'_, WIDTH, ITERS>
    {
        fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
            let expected = ITERS as i32 * WIDTH as i32 + state.s;
            let sum = self.in_.iter().filter_map(|x| x.as_ref()).sum::<i32>();
            println!("Sum of received: {}.", sum);
            assert_eq!(sum, expected, "Expected {}.", expected);
            state.s += WIDTH as i32;
        }
    }

    #[derive(Reaction)]
    #[reaction(
        reactor = "Destination<WIDTH, ITERS>",
        bound = "const WIDTH: usize",
        bound = "const ITERS: usize",
        triggers(shutdown)
    )]
    struct ReactionShutdown;

    impl runtime::Trigger<State> for ReactionShutdown {
        fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
            assert!(state.s > 0, "ERROR: Destination received no input!");
            println!("Success.");
        }
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "a.out", to = "t.in_"),
    connection(from = "t.out", to = "b.in_")
)]
struct ThreadedMultiport<const WIDTH: usize = 4, const ITERS: usize = 100_000_000> {
    #[reactor(child = "State{s: 0}")]
    a: source::Source<WIDTH>,
    #[reactor(child = "ITERS")]
    t: [computation::Computation<ITERS>; WIDTH],
    #[reactor(child = "State{s: 0}")]
    b: destination::Destination<WIDTH, ITERS>,
}

#[test]
fn threading_multiport() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(2));
    let _ = boomerang_util::runner::build_and_test_reactor::<ThreadedMultiport<4, 10_000>>(
        "threaded_multiport",
        (),
        config,
    )
    .unwrap();
}
