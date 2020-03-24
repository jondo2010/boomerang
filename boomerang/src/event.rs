use std::{cell::RefCell, cmp::Ordering, rc::Rc};

use super::{Instant, Sched, Trigger};

pub trait EventValue: std::fmt::Debug + Eq + Copy + Clone {}

impl<V> EventValue for V where V: std::fmt::Debug + Eq + Copy + Clone {}

/// Event activation record to push onto the event queue.
#[derive(Debug)]
pub struct Event<S>
where
    S: Sched,
{
    /// Time of release.
    pub time: Instant,
    /// Associated trigger.
    pub trigger: Rc<Trigger<S>>,
    /// Pointer to malloc'd value (or None)
    pub value: Rc<RefCell<Option<S::Value>>>,
}

impl<S> std::fmt::Display for Event<S>
where
    S: Sched,
    <S as Sched>::Value: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Event<time: {:?}, trigger: {:p}, value: {:?}>",
            self.time, self.trigger, self.value
        )
    }
}

impl<S> PartialEq for Event<S>
where
    S: Sched,
{
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
            && (self.trigger.as_ref() as *const Trigger<S>
                == other.trigger.as_ref() as *const Trigger<S>)
    }
}

impl<S> Eq for Event<S> where S: Sched {}

impl<S> PartialOrd for Event<S>
where
    S: Sched,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<S> Ord for Event<S>
where
    S: Sched,
{
    fn cmp(&self, other: &Self) -> Ordering {
        other.time.cmp(&self.time)
    }
}

impl<S> Event<S>
where
    S: Sched,
{
    pub fn new(time: Instant, trigger: Rc<Trigger<S>>, value: Option<S::Value>) -> Self {
        Event {
            time,
            trigger,
            value: Rc::new(RefCell::new(value)),
        }
    }
}
