#![allow(dead_code)]
#![feature(map_first_last)]

pub mod builder;
mod event;
mod reaction;
mod scheduler;
mod trigger;

// Re-exports
pub use event::{Event, EventValue};
pub use reaction::{IsPresent, OutputTrigger, Port, Reaction};
pub use scheduler::{Reactor, Sched, Scheduler};
pub use trigger::{QueuingPolicy, Trigger};

pub use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub type Index = u64;

pub type InPort<T> = Rc<RefCell<Port<T>>>;
pub type OutPort<T> = Rc<RefCell<Port<T>>>;

trait VarTrait: std::any::Any + Clone + Sync {}

fn build_delay_pair<S: Sched, T>(out: Rc<RefCell<Port<T>>>, offset: Duration) {
    let __out = out.clone();
    let __trig = Rc::new(Trigger::<S>::new(
        vec![],
        Some(offset),
        None,
        false,
        QueuingPolicy::NONE,
    ));
    let sender = move |sched: &mut S| {};
}
