#![allow(dead_code)]
#![feature(map_first_last)]

mod event;
mod reaction;
mod scheduler;
mod trigger;

#[cfg(test)]
mod tests;

// Re-exports
pub use event::{Event, EventValue};
pub use reaction::{IsPresent, Port, Reaction};
pub use scheduler::{Sched, Scheduler};
pub use trigger::{QueuingPolicy, Trigger};

pub use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub type Index = u64;

type React<S> = Rc<Reaction<S>>;
type Timer<S> = Rc<RefCell<Trigger<S>>>;
type Input<T> = Rc<RefCell<Port<T>>>;
type Output<T> = Rc<RefCell<Port<T>>>;

pub trait Reactor {
    /// Invoke code that must execute before starting a new logical time round, such as initializing
    /// outputs to be absent.
    fn start_time_step(&self);

    fn start_timers<S: Sched>(&self, sched: &mut S);
}

mod turd {
    use crate::*;
    use boomerang_derive::Reactor;

    #[derive(Reactor, Debug, Default)]
    #[reactor(
        timer(name="tim1", offset = "100 msec", period = "1 sec"),
        input(name="in1", type="u32"),
        input(name="in2", type="bool"),
        output(name="out1", type="u32"),
        reaction(function="Hello::foo", triggers("tim1"), uses(), effects("out1")),
        reaction(function="Hello::bar", triggers("in1", "in2")),
        connection(from="out1", to="in1"),
        //child(reactor="Bar", name="my_bar", inputs("x.y", "y"), outputs("b")),
    )]
    pub struct Hello {
        my_i: u32,
    }

    impl Hello {
        fn foo<S: Sched>(&mut self, sched: &mut S, inputs: (), outputs: (&mut Port<u32>)) {
            let (out1) = outputs;
            self.my_i += 1;
            out1.set(self.my_i);
            println!("foo, my_i={}", self.my_i);
        }
        fn bar<S: Sched>(&mut self, sched: &mut S, inputs: (&mut Port<u32>, &mut Port<bool>), outputs: ()) {
            let (in1, int2) = inputs;
            println!("bar, in1={}", in1.get());
            if *in1.get() == 5 {
                sched.stop();
            }
        }
    }

    // impl Reactor for Foo {
    // fn start_timers<S: Sched>(&self, sched: &mut S) {}
    // }

    #[test]
    fn test() {
        type MySched = Scheduler<&'static str>;
        let mut sched = MySched::new();

        Hello::create::<MySched>(&mut sched);
        // let f = Foo::new();
        // dbg!(f);
        while sched.next() && !sched.stop_requested {}
    }
}
