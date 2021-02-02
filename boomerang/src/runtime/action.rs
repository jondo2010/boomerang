use super::Duration;
use super::{scheduler::Scheduler, time::Tag, PortData, PortValue, ReactionKey, ReactorElement};
use derive_more::Display;
use slotmap::Key;
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    sync::Arc,
};

slotmap::new_key_type! {
    pub struct BaseActionKey;
}

#[derive(Clone, Copy, Derivative)]
#[derivative(Debug, Default, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub struct ActionKey<T: PortData>(slotmap::KeyData, PhantomData<T>);

impl<T: PortData> From<slotmap::KeyData> for ActionKey<T> {
    fn from(key: slotmap::KeyData) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: PortData> slotmap::Key for ActionKey<T> {
    fn data(&self) -> slotmap::KeyData {
        self.0
    }
}

impl<T: PortData> From<ActionKey<T>> for BaseActionKey {
    fn from(key: ActionKey<T>) -> Self {
        key.data().into()
    }
}

impl<T: PortData> From<BaseActionKey> for ActionKey<T> {
    fn from(key: BaseActionKey) -> Self {
        key.data().into()
    }
}

pub trait BaseAction: Debug + Display + Send + Sync + ReactorElement {
    fn get_name(&self) -> &str;
    /// Is this a logical action?
    fn get_is_logical(&self) -> bool;
    fn get_min_delay(&self) -> Duration;
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

#[derive(Debug)]
pub struct Action<T: PortData> {
    name: String,
    logical: bool,
    value: PortValue<T>,
    min_delay: Duration,
}

impl<T: PortData> Display for Action<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Action<{}> \"{}\"",
            std::any::type_name::<T>(),
            self.name,
        ))
    }
}

impl<T: PortData> Action<T> {
    pub fn new(name: &str, logical: bool, min_delay: Duration) -> Self {
        Self {
            name: name.to_owned(),
            logical,
            value: PortValue::new(None),
            min_delay,
        }
    }
}

impl<T: PortData> ReactorElement for Action<T> {
    fn cleanup(&self, _scheduler: &mut Scheduler) {
        *self.value.write().unwrap() = None;
    }
}

impl<T: PortData> BaseAction for Action<T> {
    fn get_name(&self) -> &str {
        &self.name
    }
    fn get_is_logical(&self) -> bool {
        self.logical
    }
    fn get_min_delay(&self) -> Duration {
        self.min_delay
    }
}

#[derive(Debug, Display)]
#[display(fmt = "Timer<{}>", name)]
pub struct Timer {
    name: String,
    action_key: BaseActionKey,
    offset: Duration,
    period: Duration,
}

impl Timer {
    pub fn new(name: &str, action_key: BaseActionKey, offset: Duration, period: Duration) -> Self {
        Self {
            name: name.to_owned(),
            action_key: action_key,
            offset,
            period,
        }
    }

    pub fn new_zero(name: &str, action_key: BaseActionKey) -> Self {
        Timer::new(
            name,
            action_key,
            Duration::from_secs(0),
            Duration::from_secs(0),
        )
    }
}

impl BaseAction for Timer {
    fn get_name(&self) -> &str {
        &self.name
    }
    fn get_is_logical(&self) -> bool {
        true
    }
    fn get_min_delay(&self) -> Duration {
        Duration::from_micros(0)
    }
}

impl ReactorElement for Timer {
    fn startup(&self, scheduler: &mut Scheduler) {
        let t0 = Tag::from(scheduler.get_start_time());
        if self.offset > Duration::from_secs(0) {
            scheduler.schedule(t0.delay(Some(self.offset)), self.action_key, None);
        } else {
            scheduler.schedule(t0, self.action_key, None);
        }
    }

    fn cleanup(&self, scheduler: &mut Scheduler) {
        // schedule the timer again
        if self.period > Duration::from_secs(0) {
            let now = Tag::from(scheduler.get_logical_time());
            let next = now.delay(Some(self.period));
            scheduler.schedule(next, self.action_key, None);
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
