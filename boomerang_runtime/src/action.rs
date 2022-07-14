use crate::SchedulerPoint;

use super::Duration;
use super::{time::Tag, PortData, ReactorElement};
use derive_more::Display;
use std::{
    fmt::{Debug, Display},
    sync::RwLock,
};

slotmap::new_key_type! {
    pub struct ActionKey;
}

pub trait BaseAction<S: SchedulerPoint>: Debug + Display + Send + Sync + ReactorElement<S> {
    fn get_name(&self) -> &str;
    /// Is this a logical action?
    fn get_is_logical(&self) -> bool;
    fn get_min_delay(&self) -> Duration;
}

#[derive(Debug)]
pub struct Action<T: PortData> {
    name: String,
    logical: bool,
    value: RwLock<(T, bool)>,
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
            value: RwLock::new((T::default(), false)),
            min_delay,
        }
    }
}

impl<S: SchedulerPoint, T: PortData> ReactorElement<S> for Action<T> {
    fn cleanup(&self, _scheduler: &S, _key: ActionKey) {
        self.value.write().unwrap().1 = false;
    }
}

impl<S: SchedulerPoint, T: PortData> BaseAction<S> for Action<T> {
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
    offset: Duration,
    period: Duration,
}

impl Timer {
    pub fn new(name: &str, offset: Duration, period: Duration) -> Self {
        Self {
            name: name.to_owned(),
            offset,
            period,
        }
    }

    pub fn new_zero(name: &str) -> Self {
        Timer::new(name, Duration::from_secs(0), Duration::from_secs(0))
    }
}

impl<S: SchedulerPoint> BaseAction<S> for Timer {
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

impl<S: SchedulerPoint> ReactorElement<S> for Timer {
    fn startup(&self, sched: &S, key: ActionKey) {
        let t0 = Tag::from(sched.get_start_time());
        if self.offset > Duration::from_secs(0) {
            sched.schedule(t0.delay(Some(self.offset)), key);
        } else {
            sched.schedule(t0, key);
        }
    }

    fn cleanup(&self, sched: &S, key: ActionKey) {
        // schedule the timer again
        if self.period > Duration::from_secs(0) {
            sched.schedule_action(key, (), Some(self.period));
        }
    }
}

/// ShutdownAction is a logical action that fires when the scheduler shuts down.
#[derive(Debug, Display)]
#[display(fmt = "ShutdownAction<{}>", name)]
pub struct ShutdownAction {
    name: String,
}

impl ShutdownAction {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }
}

impl<S: SchedulerPoint> BaseAction<S> for ShutdownAction {
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

impl<S: SchedulerPoint> ReactorElement<S> for ShutdownAction {
    fn shutdown(&self, sched: &S, key: ActionKey) {
        let tag = Tag::from(sched.get_logical_time()).delay(None);
        sched.schedule(tag, key);
    }
}
