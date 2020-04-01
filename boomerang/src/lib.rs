#![allow(dead_code)]
#![feature(map_first_last)]

mod event;
mod reaction;
mod scheduler;
mod trigger;

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
