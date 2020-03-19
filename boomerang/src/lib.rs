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

mod turd {
    use crate::*;
    use boomerang_derive::Reactor;

    #[derive(Reactor, Debug)]
    pub struct Foo<S>
    where
        S: Sched,
        <S as Sched>::Value: std::fmt::Debug,
    {
        my_i: u32,
        //#[reactor(input)]
        // in1: S::Input,
        // int1: Rc<RefCell<Port<bool>>>,

        #[reactor(output)]
        out1: Output<u32>,

        #[reactor(timer(
            offset = "Duration::from_millis(100)",
            period = "Duration::from_millis(1000)"
        ))]
        // tim1: S::Timer,
        tim1: Rc<RefCell<Trigger<S>>>,

        #[reactor(reaction((tim1) -> output))]
        hello: React<S>,
    }

    #[test]
    fn test() {
        type MySched = Scheduler<&'static str>;
        let mut sched = MySched::new();
        let f = Foo::<MySched>::new();
        dbg!(f);
    }
}
