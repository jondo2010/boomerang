#![allow(dead_code)]
#![feature(map_first_last)]

mod event;
mod reaction;
mod reactor;
mod scheduler;
mod trigger;

#[cfg(test)]
mod tests;

// Re-exports
pub use event::{Event, EventValue};
pub use reaction::{IsPresent, Port, Reaction};
pub use scheduler::{Sched, Scheduler};
pub use trigger::{QueuingPolicy, Trigger};
pub use reactor::{Reactor};

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

mod turd {
    use crate::*;
    use boomerang_derive::Reactor;

    #[derive(Reactor, Debug, Default)]
    #[reactor(
        timer(name="tim1", offset = "Duration::from_millis(100)", period = "Duration::from_millis(1000)"),
        reactor(reaction(triggers = [tim1],  output)),
        child(reactor="Bar", name="my_bar", inputs(x.y), outputs("b")),
    )]
    pub struct Foo
    {
        my_i: u32,
        //#[reactor(input)]
        // in1: S::Input,
        // int1: Rc<RefCell<Port<bool>>>,
        #[reactor(output)]
        out1: Output<u32>,
    }

    #[test]
    fn test() {
        type MySched = Scheduler<&'static str>;
        let mut sched = MySched::new();
        let f = Foo::new();
        dbg!(f);
    }
}
