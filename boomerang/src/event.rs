use std::{cell::RefCell, cmp::Ordering, rc::Rc};

use crate::trigger::Trigger;
use crate::{Index, Instant};

pub trait EventValue: std::fmt::Debug + Eq + Copy + Clone {}

impl<T> EventValue for T where T: std::fmt::Debug + Eq + Copy + Clone {}

/// Event activation record to push onto the event queue.
#[derive(Debug)]
pub struct Event<T: EventValue> {
    /// Time of release.
    pub time: Instant,
    /// Associated trigger.
    pub trigger: Rc<RefCell<Trigger<T>>>,
    /// Pointer to malloc'd value (or None)
    pub value: Rc<RefCell<Option<T>>>,
}

impl<T: EventValue> std::fmt::Display for Event<T> {
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

impl<T: EventValue> PartialEq for Event<T> {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.trigger == other.trigger
    }
}

impl<T: EventValue> Eq for Event<T> {}

impl<T: EventValue> PartialOrd for Event<T> {
    fn partial_cmp(&self, other: &Event<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: EventValue> Ord for Event<T> {
    fn cmp(&self, other: &Event<T>) -> Ordering {
        other.time.cmp(&self.time)
    }
}

impl<T: EventValue> Event<T> {
    pub fn new(time: Instant, trigger: Rc<RefCell<Trigger<T>>>, value: Option<T>) -> Self {
        Event {
            time,
            trigger,
            value: Rc::new(RefCell::new(value)),
        }
    }
}
