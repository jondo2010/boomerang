//! Check multiport capabilities on Outputs.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/concurrent/ThreadedMultiport.lf

use boomerang::builder::prelude::*;
use boomerang::{runtime, Reaction, Reactor};

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

    impl<const WIDTH: usize> Trigger<Source<WIDTH>> for ReactionT<'_, WIDTH> {
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
    #[reactor(state = "()", reaction = "ReactionIn")]
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

    impl<const ITERS: usize> Trigger<Computation<ITERS>> for ReactionIn<'_> {
        fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
            let mut offset = 0;
            for _ in 0..ITERS {
                offset += 1;
                std::thread::sleep(std::time::Duration::from_nanos(1));
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

    impl<const WIDTH: usize, const ITERS: usize> Trigger<Destination<WIDTH, ITERS>>
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

    impl<const WIDTH: usize, const ITERS: usize> Trigger<Destination<WIDTH, ITERS>>
        for ReactionShutdown
    {
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
    #[reactor(child = "()")]
    t: [computation::Computation<ITERS>; WIDTH],
    #[reactor(child = "State{s: 0}")]
    b: destination::Destination<WIDTH, ITERS>,
    #[reactor(child = "runtime::Duration::from_secs(2)")]
    _timeout: boomerang_util::timeout::Timeout,
}

#[test]
fn threading_multiport() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<ThreadedMultiport<4, 10_000>>(
        "threaded_multiport",
        (),
        true,
        false,
    )
    .unwrap();
}

/*
reactor Source(width: int = 4) {
    timer t(0, 200 msec)
    output[width] out: int
    state s: int = 0

    reaction(t) -> out {=
        for(int i = 0; i < out_width; i++) {
        lf_set(out[i], self->s);
        }
        self->s++;
    =}
}

reactor Computation(iterations: int = 100000000) {
    input in: int
    output out: int

    reaction(in) -> out {=
        // struct timespec sleep_time = {(time_t) 0, (long)200000000};
        // struct timespec remaining_time;
        // nanosleep(&sleep_time, &remaining_time);
        int offset = 0;
        for (int i = 0; i < self->iterations; i++) {
        offset++;
        }
        lf_set(out, in->value + offset);
    =}
}

reactor Destination(width: int = 4, iterations: int = 100000000) {
    state s: int = 0
    input[width] in: int

    reaction(in) {=
        int expected = self->iterations * self->width + self->s;
        int sum = 0;
        for (int i = 0; i < in_width; i++) {
        if (in[i]->is_present) sum += in[i]->value;
        }
        printf("Sum of received: %d.\n", sum);
        if (sum != expected) {
        printf("ERROR: Expected %d.\n", expected);
        exit(1);
        }
        self->s += self->width;
    =}

    reaction(shutdown) {=
        if (self->s == 0) {
        fprintf(stderr, "ERROR: Destination received no input!\n");
        exit(1);
        }
        printf("Success.\n");
    =}
}

main reactor ThreadedMultiport(width: int = 4, iterations: int = 100000000) {
a = new Source(width=width)
t = new[width] Computation(iterations=iterations)
b = new Destination(width=width, iterations=iterations)
a.out -> t.in
t.out -> b.in
}
*/
