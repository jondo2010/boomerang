use super::SchedulerPoint;
use std::{fmt::Debug, sync::RwLock, time::Duration};

slotmap::new_key_type! {
    pub struct ReactionKey;
}

pub trait ReactionFnTrait: FnMut(&SchedulerPoint) + Send + Sync {}
impl<T> ReactionFnTrait for T where T: FnMut(&SchedulerPoint) + Send + Sync {}
pub struct ReactionFn(Box<dyn ReactionFnTrait>);
impl ReactionFn {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(&SchedulerPoint) + Send + Sync + 'static,
    {
        Self(Box::new(f))
    }
}

impl Debug for ReactionFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ReactionFn").finish()
    }
}

impl Ord for ReactionFn {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for ReactionFn {
    fn partial_cmp(&self, _: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ordering::Equal)
    }
}

impl Eq for ReactionFn {}

impl PartialEq for ReactionFn {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct Deadline {
    deadline: Duration,
    handler: RwLock<ReactionFn>,
}

impl PartialEq for Deadline {
    fn eq(&self, other: &Self) -> bool {
        self.deadline.eq(&other.deadline)
    }
}

impl Eq for Deadline {}

impl PartialOrd for Deadline {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deadline.partial_cmp(&other.deadline)
    }
}

impl Ord for Deadline {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline.cmp(&other.deadline)
    }
}

#[derive(Debug)]
pub struct Reaction {
    name: String,
    /// Inverse priority determined by dependency analysis.
    level: usize,
    /// Reaction closure
    body: RwLock<ReactionFn>,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    deadline: Option<Deadline>,
}

impl PartialEq for Reaction {
    fn eq(&self, other: &Self) -> bool {
        self.name.eq(&other.name) && self.level.eq(&other.level)
    }
}

impl Eq for Reaction {}

impl PartialOrd for Reaction {
    fn partial_cmp(&self, _other: &Self) -> Option<std::cmp::Ordering> {
        todo!()
    }
}

impl Ord for Reaction {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        todo!()
    }
}

impl Reaction {
    pub fn new(name: String, level: usize, body: ReactionFn, deadline: Option<Deadline>) -> Self {
        Self {
            name,
            level,
            body: RwLock::new(body),
            deadline,
        }
    }

    pub fn get_level(&self) -> usize {
        self.level
    }

    pub fn trigger(&self, sched: &SchedulerPoint) {
        match self.deadline.as_ref() {
            Some(Deadline {
                deadline: _,
                handler: _,
            }) => {
                // let lag = container()->get_physical_time() - container()->get_logical_time();
                // if lag > deadline {
                // handler();
                // return;
                // }
            }
            _ => {}
        }

        let mut body = self.body.write().unwrap();
        (body.0.as_mut())(sched)
    }
}
