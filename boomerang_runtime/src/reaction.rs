use super::SchedulerPoint;
use std::{fmt::Debug, sync::RwLock, time::Duration};

slotmap::new_key_type! {
    pub struct ReactionKey;
}

pub trait ReactionFn<S>: FnMut(&S) + Send + Sync
where
    S: SchedulerPoint,
{
}

impl<S, F> ReactionFn<S> for F
where
    S: SchedulerPoint,
    F: FnMut(&S) + Send + Sync,
{
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Deadline<S>
where
    S: SchedulerPoint,
{
    deadline: Duration,
    #[derivative(PartialEq = "ignore")]
    #[derivative(Debug = "ignore")]
    handler: RwLock<Box<dyn ReactionFn<S>>>,
}

#[derive(Derivative)]
#[derivative(Debug, PartialEq)]
pub struct Reaction<S>
where
    S: SchedulerPoint,
{
    name: String,
    /// Inverse priority determined by dependency analysis.
    level: usize,
    /// Reaction closure
    #[derivative(PartialEq = "ignore")]
    #[derivative(Debug = "ignore")]
    body: RwLock<Box<dyn ReactionFn<S>>>,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    deadline: Option<Deadline<S>>,
}

impl<S> Reaction<S>
where
    S: SchedulerPoint,
{
    pub fn new<F>(name: String, level: usize, body: F, deadline: Option<Deadline<S>>) -> Self
    where
        F: ReactionFn<S> + 'static,
    {
        Self {
            name,
            level,
            body: RwLock::new(Box::new(body)),
            deadline,
        }
    }

    pub fn get_level(&self) -> usize {
        self.level
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn trigger<'b>(&self, sched: &'b S) {
        match self.deadline.as_ref() {
            Some(Deadline { deadline, handler }) => {
                let lag = sched.get_physical_time() - *sched.get_logical_time();
                if lag > *deadline {
                    (handler.write().unwrap())(sched);
                }
            }
            _ => {}
        }

        (self.body.write().unwrap())(sched);
    }
}

#[test]
fn test_new() {
    let _ = Reaction::new(
        "test".into(),
        0,
        |sched: &crate::Scheduler| {
            let _x = sched.get_start_time();
        },
        None,
    );
}
