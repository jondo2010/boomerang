use super::*;
use crate::{runtime, Error};
use std::{cell::RefCell, collections::BTreeSet, fmt::Debug, time::Duration};

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct ActionBuilder<'a> {
    name: String,
    triggers: RefCell<BTreeSet<&'a ReactionBuilder<'a>>>,
    schedulers: RefCell<BTreeSet<&'a ReactionBuilder<'a>>>,
    /* on_startup: Option<Box<runtime::ActionFn>>,
     * jon_shutdown: Option<Box<runtime::ActionFn>>,
     * on_cleanup: Option<Box<runtime::ActionFn>>, */
}

// impl Debug for ActionBuilder {
// fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
// f.debug_struct("ActionBuilder")
// .field("triggers", &self.triggers)
// .field("schedulers", &self.schedulers)
// .field("on_startup", &self.on_startup.map(|_| "Fn"))
// .field("on_shutdown", &self.on_shutdown.map(|_| "Fn"))
// .field("on_cleanup", &self.on_cleanup.map(|_| "Fn"))
// .finish()
// }
// }

impl<'a> ActionBuilder<'a> {
    /// Create a new Startup Action
    /// This is equivalent to a Timer Action with zero offset and period.
    pub fn new_startup_action(name: &str) -> Self {
        Self::new_timer_action(name, Duration::default(), Duration::default())
    }

    /// Create a new Shutdown Action
    ///     On shutdown() - schedule this action with zero offset
    pub fn new_shutdown_action(name: &str) -> Self {
        // let shutdown_fn = Box::new(|action: &Rc<dyn runtime::BaseAction>, sched: &mut
        // runtime::Scheduler| { let t = runtime::Tag::from_logical_time(sched.
        // get_logical_time()).delay_zero(); sched.schedule_sync(t, action, None)
        // });

        Self {
            name: name.to_string(),
            triggers: RefCell::new(BTreeSet::new()),
            schedulers: RefCell::new(BTreeSet::new()),
            /* on_startup: None,
             * on_shutdown: None,
             * on_cleanup: None, */
        }
    }

    /// Create a new Timer Action
    ///     On startup() - schedule the action with possible offset
    ///     On cleanup() - reschedule if the duration is non-zero
    pub fn new_timer_action(name: &str, offset: Duration, period: Duration) -> Self {
        // let startup_fn: Option<Box<runtime::ActionFn>> = Some(Box::new(
        // move |action: &Rc<runtime::Action>, sched: &mut runtime::Scheduler| {
        // let t0 = runtime::Tag::from_physical_time(sched.get_start_time());
        // if offset > Duration::from_secs(0) {
        // sched.schedule_sync(t0.delay(offset), action, None);
        // } else {
        // sched.schedule_sync(t0, action, None);
        // }
        // },
        // ));
        //
        // let cleanup_fn: Option<Box<runtime::ActionFn>> = if period > Duration::from_secs(0) {
        // Some(Box::new(
        // move |action: &Rc<runtime::Action>, sched: &mut runtime::Scheduler| {
        // schedule the timer again
        // let now = runtime::Tag::from_logical_time(sched.get_logical_time());
        // let next = now.delay(period);
        // sched.schedule_sync(next, action, None);
        // },
        // ))
        // } else {
        // None
        // };
        ActionBuilder {
            name: name.to_string(),
            triggers: RefCell::new(BTreeSet::new()),
            schedulers: RefCell::new(BTreeSet::new()),
            /* on_startup: None,
             * on_shutdown: None,
             * on_cleanup: None, */
        }
    }

    pub fn register_trigger(&self, reaction: &'a ReactionBuilder<'a>) {
        // TODO assert
        //"Action triggers must belong to the same reactor as the triggered reaction"
        self.triggers.borrow_mut().insert(reaction);
    }

    pub fn register_scheduler(&self, reaction: &'a ReactionBuilder<'a>) {
        // VALIDATE(is_logical(), "only logical action can be scheduled by a reaction!");
        // the reaction must belong to the same reactor as this action
        //"Scheduable actions must belong to the same reactor as the triggered reaction");
        self.schedulers.borrow_mut().insert(reaction);
    }
}
