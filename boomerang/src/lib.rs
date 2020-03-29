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
        output(name="out1", type="u32"),
        reaction(function="Foo::bar", triggers("tim1"), uses(), effects("out1")),
        reaction(function="Foo::rab", triggers("in1")),
        connection(from="out1", to="in1"),
        //child(reactor="Bar", name="my_bar", inputs("x.y", "y"), outputs("b")),
    )]
    pub struct Foo {
        my_i: u32,
    }

    impl Foo {
        fn bar(&mut self, inputs: (), outputs: (&mut Port<u32>)) {}
        fn rab(&mut self, inputs: (&mut Port<u32>), outputs: ()) {}
    }

    // impl Reactor for Foo {
    // fn start_timers<S: Sched>(&self, sched: &mut S) {}
    // }

    #[test]
    fn test() {
        type MySched = Scheduler<&'static str>;
        let mut sched = MySched::new();
        //let f = Foo::new();
        //dbg!(f);
    }
}
