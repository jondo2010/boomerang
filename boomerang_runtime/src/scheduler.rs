use super::Env;
use super::{
    time::{LogicalTime, Tag},
    ActionKey, Duration, Instant, PortData, PortKey, ReactionKey,
};
use crate::{DepInfo, RuntimeError};
use crossbeam_channel::{Receiver, Sender};
use crossbeam_utils::atomic::AtomicCell;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use tracing::event;

type PreHandlerFn = Box<dyn Fn() -> () + Send + Sync>;
type EventMap = BTreeMap<ActionKey, Option<PreHandlerFn>>;
type ScheduledEvent = (Tag, ActionKey, Option<PreHandlerFn>);

pub enum RunMode {
    /// In Normal mode, exit when no more events have been scheduled.
    Normal,
    /// In RunForever mode, asynchronously wait for new events.
    RunForever,
    /// In RunFor mode, shutdown after a fixed period, even if events are waiting.
    RunFor(Duration),
}

pub struct Config {
    /// The configured RunMode to use
    run_mode: RunMode,
    /// Run as fast as possible, without time synchronization.
    fast_forward: bool,
}

impl Config {
    pub fn new(run_mode: RunMode, fast_forward: bool) -> Self {
        Self {
            run_mode,
            fast_forward,
        }
    }
}

pub trait SchedulerPoint // : Send + Sync + 'static
{
    fn get_start_time(&self) -> Instant;
    fn get_logical_time(&self) -> Instant;
    fn get_physical_time(&self) -> Instant;
    fn get_elapsed_logical_time(&self) -> Duration;
    fn get_elapsed_physical_time(&self) -> Duration;
    /// Apply a closure to the port data.
    /// The closure `F` should conform to `FnOnce(value: &T, is_set: bool)`.
    /// `is_set` indicates that the port was set within the last scheduling round.
    fn get_port_with<T: PortData, F: FnOnce(&T, bool)>(&self, port_key: PortKey, f: F);

    /// Apply a closure to the port data mutably.
    /// The closure `F` should conform to `FnOnce(value: &mut T, is_set: bool) -> bool`.
    /// `is_set` indicates that the port was set within the last scheduling round.
    /// `F` should return `true` if the port value was set.
    fn get_port_with_mut<T: PortData, F: FnOnce(&mut T, bool) -> bool>(
        &self,
        port_key: PortKey,
        f: F,
    );
    fn schedule_action<T: PortData>(
        &self,
        action_key: ActionKey,
        _value: T,
        delay: Option<super::Duration>,
    );
    fn schedule(&self, tag: Tag, action_key: ActionKey);
    fn shutdown(&self);
}

impl SchedulerPoint for Scheduler {
    fn get_start_time(&self) -> Instant {
        self.get_start_time()
    }

    fn get_logical_time(&self) -> Instant {
        self.get_logical_time().get_time_point()
    }

    fn get_physical_time(&self) -> Instant {
        self.get_physical_time()
    }

    fn get_elapsed_logical_time(&self) -> Duration {
        self.get_elapsed_logical_time()
    }

    fn get_elapsed_physical_time(&self) -> Duration {
        self.get_physical_time()
            .saturating_duration_since(self.get_start_time())
    }

    fn get_port_with<T: PortData, F: FnOnce(&T, bool)>(&self, port_key: PortKey, f: F) {
        let port = self.env.get_port::<T>(port_key).unwrap();
        port.get_with(f);
    }

    fn get_port_with_mut<T: PortData, F: FnOnce(&mut T, bool) -> bool>(
        &self,
        port_key: PortKey,
        f: F,
    ) {
        let port = self.env.get_port::<T>(port_key).unwrap();
        if port.get_with_mut(f) {
            // Schedule any reactions triggered by this port being set.
            let port_key = port_key.into();

            for sub_reaction_key in self.dep_info.port_triggers[port_key].keys() {
                let new_reaction_level = self.dep_info.reaction_levels[sub_reaction_key];
                event!(
                    tracing::Level::DEBUG,
                    "set({:?}) triggering {:?} at level {}",
                    port_key,
                    sub_reaction_key,
                    new_reaction_level
                );
                self.reaction_queue_s[new_reaction_level]
                    .send(sub_reaction_key)
                    .unwrap();
            }

            // Add this port to the list of ports that need to be cleared
            self.clear_ports_s.send(port_key).unwrap();
        }
    }

    fn schedule_action<T: PortData>(
        &self,
        action_key: ActionKey,
        _value: T,
        delay: Option<super::Duration>,
    ) {
        todo!();
        /*
        let action = &self.env.actions[action_key];
        // TODO set value
        if action.get_is_logical() {
            let delay = delay.unwrap_or_default();
            let tag =
                Tag::from(self.get_logical_time()).delay(Some(delay + action.get_min_delay()));
            event!(
                tracing::Level::DEBUG,
                "Scheduling action \"{}\"",
                action.get_name()
            );
            self.schedule(tag, action_key, None);
        }
        */
    }

    fn schedule(&self, tag: Tag, action_key: ActionKey) {
        self.schedule(tag, action_key, None);
    }

    fn shutdown(&self) {
        event!(tracing::Level::INFO, "Schduler shutdown requested...");
        // all reactors shutdown()
        self.stop();
    }
}

pub struct Scheduler {
    config: Config,

    dep_info: DepInfo,
    env: Env,

    /// Physical time the Scheduler was started
    start_time: Instant,

    /// Current logical time
    logical_time: LogicalTime,

    events_channel_s: Sender<ScheduledEvent>,
    events_channel_r: Receiver<ScheduledEvent>,

    /// Ordered queue of events
    event_queue: BTreeMap<Tag, EventMap>,

    /// Reaction Queue organized by level bins
    reaction_queue_s: Vec<Sender<ReactionKey>>,
    reaction_queue_r: Vec<Receiver<ReactionKey>>,

    clear_ports_s: Sender<PortKey>,
    clear_ports_r: Receiver<PortKey>,

    /// Stop requested or triggered by end conditions
    should_stop: AtomicCell<bool>,
}

impl Scheduler {
    pub fn new(env: Env, dep_info: DepInfo) -> Self {
        Self::with_config(
            env,
            dep_info,
            Config {
                run_mode: RunMode::Normal,
                fast_forward: false,
            },
        )
    }

    pub fn with_config(env: Env, dep_info: DepInfo, config: Config) -> Self {
        let (events_channel_s, events_channel_r) = crossbeam_channel::unbounded();
        let (reaction_queue_s, reaction_queue_r) =
            std::iter::repeat_with(crossbeam_channel::unbounded)
                .take(dep_info.max_level() + 1)
                .unzip();
        let (clear_ports_s, clear_ports_r) = crossbeam_channel::unbounded();

        Scheduler {
            config,
            env,
            dep_info,
            start_time: Instant::now(),
            logical_time: LogicalTime::new(),
            events_channel_s,
            events_channel_r,
            event_queue: BTreeMap::new(),
            reaction_queue_s,
            reaction_queue_r,
            clear_ports_s,
            clear_ports_r,
            should_stop: AtomicCell::new(false),
        }
    }

    pub fn get_start_time(&self) -> Instant {
        self.start_time
    }

    pub fn get_logical_time(&self) -> LogicalTime {
        todo!();
        //self.logical_time
    }
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.logical_time
            .get_time_point()
            .saturating_duration_since(self.start_time)
    }

    pub fn get_physical_time(&self) -> Instant {
        Instant::now()
    }

    pub fn get_elapsed_physical_time(&self) -> Duration {
        self.get_physical_time()
            .saturating_duration_since(self.start_time)
    }

    pub fn schedule(&self, tag: Tag, action_key: ActionKey, pre_handler: Option<PreHandlerFn>) {
        /*
        event!(
            tracing::Level::DEBUG,
            ?action_key,
            "Schedule ({:?})",
            tag.difference(&Tag::from(&self.start_time)),
        );
        */
        self.events_channel_s
            .send((tag, action_key, pre_handler))
            .unwrap();
    }

    pub fn start(&mut self) -> Result<(), RuntimeError> {
        event!(tracing::Level::INFO, "Starting the scheduler...");
        self.env.startup(self);
        self.run()
    }

    pub fn stop(&self) {
        self.env.shutdown(&self);
        self.should_stop.store(true);
    }

    fn run(&mut self) -> Result<(), RuntimeError> {
        todo!();
        /*
        while let Some((t_next, tag_events, run_again)) = self.get_next_tagged_events() {
            let dt = t_next.difference(&Tag::from(&self.logical_time));
            event!(tracing::Level::DEBUG, "Advance logical time by [{:?}]", dt,);
            self.logical_time.advance_to(&t_next);
            self.next(tag_events)?;
            if !run_again {
                break;
            }
        }
        */
        Ok(())
    }

    /// Take all available events on the channel and push them into the queue.
    fn feed_event_queue(&mut self) {
        for (tag, action_idx, pre_handler) in self.events_channel_r.try_iter() {
            self.event_queue
                .entry(tag)
                .or_insert(EventMap::new())
                .insert(action_idx, pre_handler);
        }
    }

    fn get_next_tagged_events(&mut self) -> Option<(Tag, EventMap, bool)> {
        self.feed_event_queue();

        // shutdown if there are no more events in the queue
        if self.event_queue.is_empty() && !self.should_stop.load() {
            if matches!(self.config.run_mode, RunMode::RunForever) {
                // wait for a new asynchronous event
                // cv_schedule.wait(lock, [this]() { return !event_queue.empty(); });
            } else {
                event!(
                    tracing::Level::DEBUG,
                    "No more events in queue. -> Terminate!"
                );
                self.env.shutdown(&self);

                // The shutdown call might schedule shutdown reactions.
                self.feed_event_queue();

                // If none was scheduled, we simply return.
                if self.event_queue.is_empty() {
                    return None;
                }

                self.should_stop.store(true);
            }
        }

        let event_entry = self.event_queue.first_entry().expect("Empty Event Queue!");

        let (t_next, run_again) = if self.should_stop.load() {
            event!(tracing::Level::INFO, "Shutting down the scheduler");
            let t_next = Tag::from(&self.logical_time).delay(None);
            if t_next != *event_entry.key() {
                return None;
            }
            event!(
                tracing::Level::DEBUG,
                "Schedule the last round of reactions including all termination reactions"
            );

            (t_next, false)
        } else {
            // Collect events of the next tag
            let t_next = event_entry.key();

            // synchronize with physical time if not in fast forward mode
            if !self.config.fast_forward {
            /*
                // wait until the next tag or until a new event is inserted
                // asynchronously into the queue
                let status = cv_schedule.wait_until(lock, t_next.time_point());
                // Start over if the event queue was modified
                if (status == std::cv_status::no_timeout) {
                    return true;
                }
                // continue otherwise
            */
            }
            (t_next.to_owned(), true)
        };

        // Retrieve all events with tag equal to current logical time from the queue
        let tag_events = event_entry.remove();

        Some((t_next, tag_events, run_again))
    }

    fn trigger_reactions(&self, reaction_keys: BTreeSet<ReactionKey>) {
        reaction_keys.par_iter().for_each(|&key| {
            let reaction = &self.env.reactions[key];
            event!(
                tracing::Level::DEBUG,
                "Executing reaction [{}]",
                reaction.get_name()
            );
            //reaction.trigger(self);
        });
    }

    fn next(&self, tag_events: EventMap) -> Result<(), RuntimeError> {
        if let RunMode::RunFor(run_for) = self.config.run_mode {
            if self.get_elapsed_logical_time() >= run_for {
                self.should_stop.store(true);
            }
        }

        // Execute pre-handler setup functions; this sets the values of the corresponding actions
        tag_events.values().for_each(|setup| {
            if let Some(pre_handler_fn) = setup {
                (pre_handler_fn)();
            }
        });

        // Insert all Reactions triggered by each Event/Action into the reaction_queue.
        for reaction_key in tag_events
            .keys()
            .flat_map(|&action_key| self.dep_info.action_triggers[action_key].keys())
        {
            let reaction_level = self.dep_info.reaction_levels[reaction_key];
            self.reaction_queue_s[reaction_level]
                .send(reaction_key)
                .unwrap();
        }

        // Process all Reactions in the queue in order of index
        for rqueue_r in self.reaction_queue_r.iter() {
            // Pull all the ReactionIdx at this level into a set
            let rqueue_keys: BTreeSet<ReactionKey> = rqueue_r.try_iter().collect();
            self.trigger_reactions(rqueue_keys);
        }

        // cleanup all triggered actions
        tag_events.keys().for_each(|&event_idx| {
            self.env.actions[event_idx].cleanup(&self, event_idx);
        });

        // Call clean on set ports
        self.clear_ports_r
            .try_iter()
            .map(|port_key| &self.env.ports[port_key])
            .for_each(|port| port.cleanup());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossbeam_utils::atomic::AtomicCell;
    use slotmap::{SecondaryMap, SlotMap};
    use std::sync::Arc;

    use crate::{BaseAction, Reaction, ShutdownAction};

    #[test]
    fn test_shutdown() {
        let shutdown = Arc::new(AtomicCell::new(false));
        let inner = shutdown.clone();

        let mut reactions: SlotMap<ReactionKey, Reaction> = SlotMap::with_key();
        let reaction_key = reactions.insert(Reaction::new(
            "shutdown_reaction".to_owned(),
            move |_: &Scheduler| {
                println!("shutdown_reaction()");
                inner.store(true);
            },
            None,
        ));

        let mut actions: SlotMap<ActionKey, Arc<dyn BaseAction>> = SlotMap::with_key();
        let action_key = actions.insert(Arc::new(ShutdownAction::new("shutdown_action")));

        let mut action_triggers: SecondaryMap<ActionKey, SecondaryMap<ReactionKey, ()>> =
            SecondaryMap::new();
        action_triggers.insert(action_key, vec![(reaction_key, ())].into_iter().collect());

        let env = Env {
            ports: SlotMap::with_key(),
            actions,
            reactions,
        };

        let dep_info = DepInfo {
            action_triggers,
            port_triggers: SecondaryMap::new(),
            reaction_levels: SecondaryMap::new(),
        };

        let mut sched = Scheduler::new(env, dep_info);
        sched.start().unwrap();

        assert!(shutdown.load());
    }
}
