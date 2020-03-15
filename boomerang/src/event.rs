use derive_more::Display;
use std::{cell::RefCell, cmp::Ordering, rc::Rc};

use super::{Instant, Sched, Trigger};

pub trait EventValue: std::fmt::Debug + Eq + Copy + Clone {}

impl<V> EventValue for V where V: std::fmt::Debug + Eq + Copy + Clone {}

/// Event activation record to push onto the event queue.
#[derive(Debug)]
pub struct Event<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    /// Time of release.
    pub time: Instant,
    /// Associated trigger.
    pub trigger: Rc<RefCell<Trigger<V, S>>>,
    /// Pointer to malloc'd value (or None)
    pub value: Rc<RefCell<Option<V>>>,
}

impl<V, S> std::fmt::Display for Event<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Event<time: {:?}, trigger: {:p}, value: {:?}>",
            self.time,
            self.trigger.as_ptr(),
            self.value
        )
    }
}

impl<V, S> PartialEq for Event<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.trigger.as_ptr() == other.trigger.as_ptr()
    }
}

impl<V, S> Eq for Event<V, S>
where
    V: EventValue,
    S: Sched<V>,
{
}

impl<V, S> PartialOrd for Event<V, S> 
where
    V: EventValue,
    S: Sched<V>,
{
    fn partial_cmp(&self, other: &Event<V, S>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<V, S> Ord for Event<V, S> 
where
    V: EventValue,
    S: Sched<V>,
{
    fn cmp(&self, other: &Event<V, S>) -> Ordering {
        other.time.cmp(&self.time)
    }
}

impl<V, S> Event<V, S> 
where
    V: EventValue,
    S: Sched<V>,
{
    pub fn new(time: Instant, trigger: Rc<RefCell<Trigger<V, S>>>, value: Option<V>) -> Self {
        Event {
            time,
            trigger,
            value: Rc::new(RefCell::new(value)),
        }
    }
}
