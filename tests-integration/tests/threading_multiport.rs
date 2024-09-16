//! Check multiport capabilities on Outputs.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/concurrent/ThreadedMultiport.lf

//timeout: 2 sec

use boomerang::builder::prelude::*;
use boomerang::{runtime, Reaction, Reactor};

struct State {
    s: i32,
}

#[derive(Reactor)]
#[reactor(
    state = "State",
    //reaction = "ReactionT<WIDTH>"
)]
struct Source<const WIDTH: usize> {
    #[reactor(timer(period = "200 msec"))]
    t: TimerActionKey,
    out: [OutputPortKey<i32>; WIDTH],
}

#[derive(Reaction)]
#[reaction(reactor = "Source<WIDTH>", triggers(action = "t"))]
struct ReactionT<'a, const WIDTH: usize> {
    //out: [runtime::OutputRef<'a, i32>; WIDTH],
}

impl<const WIDTH: usize> Trigger<Source<WIDTH>> for ReactionT<'_, WIDTH> {
    fn trigger(mut self, ctx: &mut runtime::Context, state: &mut State) {
        for i in 0..self.out.len() {
            //ctx.set(&self.out[i], state.s);
        }
        state.s += 1;
    }
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
