use super::{scheduler::Scheduler, time::Tag, PortData, PortValue, ReactionIndex, ReactorElement};
use derive_more::Display;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fmt::{Debug, Display},
    sync::{Arc, RwLock},
    time::Duration,
};

#[derive(Display, Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub struct ActionIndex(pub usize);

pub trait BaseAction: Debug + Display + Send + Sync + ReactorElement {
    /// Get the transitive set of Reactions that are sensitive to this Action executing.
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex>;
}

impl std::cmp::PartialEq for dyn BaseAction {
    fn eq(&self, _other: &Self) -> bool {
        todo!()
    }
}

impl std::cmp::Eq for dyn BaseAction {}

impl std::cmp::PartialOrd for dyn BaseAction {
    fn partial_cmp(&self, _other: &Self) -> Option<std::cmp::Ordering> {
        todo!()
    }
}

impl std::cmp::Ord for dyn BaseAction {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        todo!()
    }
}

#[derive(Debug, Display)]
#[display(fmt = "Action <{}>, triggers={:?}", name, triggers)]
pub struct Action<T>
where
    T: PortData,
{
    name: String,
    value: PortValue<T>,
    triggers: BTreeSet<ReactionIndex>,
    min_delay: Duration,
}

impl<T> Action<T>
where
    T: PortData,
{
    pub fn new(name: &str, _logical: bool, min_delay: Duration) -> Self {
        Self {
            name: name.to_owned(),
            value: PortValue::new(None),
            triggers: BTreeSet::new(),
            min_delay,
        }
    }
}

impl<T> ReactorElement for Action<T>
where
    T: PortData,
{
    fn cleanup(&self, _scheduler: &mut Scheduler) {
        *self.value.write().unwrap() = None;
    }
}

impl<T> BaseAction for Action<T>
where
    T: PortData,
{
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex> {
        &self.triggers
    }
}

#[derive(Debug, Display)]
#[display(fmt = "{}", name)]
pub struct Timer {
    name: String,
    action_idx: ActionIndex,
    offset: Duration,
    period: Duration,
    triggers: BTreeSet<ReactionIndex>,
}

impl Timer {
    pub fn new(
        name: &str,
        action_idx: ActionIndex,
        offset: Duration,
        period: Duration,
        triggers: BTreeSet<ReactionIndex>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            action_idx,
            offset,
            period,
            triggers,
        }
    }

    pub fn new_zero(
        name: &str,
        action_idx: ActionIndex,
        triggers: BTreeSet<ReactionIndex>,
    ) -> Self {
        Timer::new(
            name,
            action_idx,
            Duration::from_secs(0),
            Duration::from_secs(0),
            triggers,
        )
    }
}

impl BaseAction for Timer {
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex> {
        &self.triggers
    }
}

impl ReactorElement for Timer {
    fn startup(&self, scheduler: &mut Scheduler) {
        let t0 = Tag::from(scheduler.get_start_time());
        if self.offset > Duration::from_secs(0) {
            scheduler.schedule(t0.delay(Some(self.offset)), self.action_idx, None);
        } else {
            scheduler.schedule(t0, self.action_idx, None);
        }
    }

    fn cleanup(&self, scheduler: &mut Scheduler) {
        // schedule the timer again
        if self.period > Duration::from_secs(0) {
            let now = Tag::from(scheduler.get_logical_time());
            let next = now.delay(Some(self.period));
            scheduler.schedule(next, self.action_idx, None);
        }
    }
}

// ----------

pub struct ActionFn(dyn Fn(&Arc<dyn BaseAction>, &mut Scheduler) -> ());

// A runtime Action
// pub struct Action {
// name: String,
// triggers: Vec<Rc<Reaction>>,
// on_startup: Option<Box<ActionFn>>,
// on_shutdown: Option<Box<ActionFn>>,
// on_cleanup: Option<Box<ActionFn>>,
// }
// impl Action {
// pub fn startup(self: &Rc<Self>, sched: &mut Scheduler) {
// if let Some(startup_fn) = self.on_startup {
// startup_fn(self, sched);
// }
// }
// pub fn shutdown(self: &Rc<Self>, sched: &mut Scheduler) {
// if let Some(shutdown_fn) = self.on_shutdown {
// shutdown_fn(self, sched);
// }
// }
// pub fn cleanup(self: &Rc<Self>, sched: &mut Scheduler) {
// if let Some(cleanup_fn) = self.on_cleanup {
// cleanup_fn(self, sched);
// }
// }
// pub fn get_triggers(&self) -> impl Iterator<item = Rc<Reaction>> {
// }
// }
//
