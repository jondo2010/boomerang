use crate::SchedulerPoint;

use super::Duration;
use super::{time::Tag, PortData, PortValue, ReactorElement};
use derive_more::Display;
use slotmap::Key;
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
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
    fn cleanup(&self, _scheduler: &SchedulerPoint) {
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
    fn startup(&self, sched: &SchedulerPoint) {
        let t0 = Tag::from(sched.get_start_time());
        if self.offset > Duration::from_secs(0) {
            sched.schedule(t0.delay(Some(self.offset)), self.action_key);
        } else {
            sched.schedule(t0, self.action_key);
        }
    }

    fn cleanup(&self, sched: &SchedulerPoint) {
        // schedule the timer again
        if self.period > Duration::from_secs(0) {
            sched.schedule_action(self.action_key.into(), (), Some(self.period));
        }
    }
}

/// ShutdownAction is a logical action that fires when the scheduler shuts down.
#[derive(Debug, Display)]
#[display(fmt = "ShutdownAction<{}>", name)]
struct ShutdownAction {
    name: String,
    action_key: BaseActionKey,
}

impl BaseAction for ShutdownAction {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_is_logical(&self) -> bool {
        true
    }

    fn get_min_delay(&self) -> Duration {
        Duration::default()
    }
}

impl ReactorElement for ShutdownAction {
    fn shutdown(&self, sched: & SchedulerPoint) {
        sched.schedule_action(self.action_key.into(), (), None);
    }
}
