use super::{
    environment::Environment,
    time::{LogicalTime, Tag},
    ActionKey, BaseActionKey, BasePortKey, PortData, PortKey, ReactionKey,
};
use crate::BoomerangError;
use crossbeam_channel::{Receiver, Sender};
use rayon::prelude::*;
use slotmap::Key;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    sync::{Arc, RwLock},
    time::Instant,
};
use tracing::event;

type PreHandlerFn = Box<dyn Fn() -> () + Send + Sync>;
type EventMap = BTreeMap<BaseActionKey, Option<PreHandlerFn>>;
type ScheduledEvent = (Tag, BaseActionKey, Option<PreHandlerFn>);

pub struct Scheduler {
    /// Physical time the Scheduler was started
    start_time: Instant,

    /// Current logical time
    logical_time: LogicalTime,

    events_channel_s: Sender<ScheduledEvent>,
    events_channel_r: Receiver<ScheduledEvent>,

    /// Ordered queue of events
    event_queue: BTreeMap<Tag, EventMap>,

    /// Reaction Queue organized by level bins
    // reaction_queue: Vec<BTreeSet<ReactionIndex>>,
    reaction_queue_s: Vec<Sender<ReactionKey>>,
    reaction_queue_r: Vec<Receiver<ReactionKey>>,

    /// Stop requested
    stop_requested: bool,
}

#[derive(Debug, Clone)]
pub struct SchedulerPoint<'a> {
    scheduler: &'a Scheduler,
    env: &'a Environment,
    set_ports_s: Sender<BasePortKey>,
    stop_requested: Arc<RwLock<bool>>,
}

impl<'a> SchedulerPoint<'a> {
    fn new(
        scheduler: &'a Scheduler,
        env: &'a Environment,
        set_ports_s: Sender<BasePortKey>,
    ) -> Self {
        SchedulerPoint {
            scheduler,
            env,
            set_ports_s,
            stop_requested: Arc::new(RwLock::new(false)),
        }
    }

    pub fn get_start_time(&self) -> &Instant {
        self.scheduler.get_start_time()
    }
    pub fn get_logical_time(&self) -> &LogicalTime {
        self.scheduler.get_logical_time()
    }
    pub fn set_port<T: PortData>(&self, port_key: PortKey<T>, value: T) {
        self.env.get_port(port_key).unwrap().set(Some(value));

        // Schedule any reactions triggered by this port being set.
        let port_key = port_key.data().into();

        for sub_reaction_key in self.env.port_triggers[port_key].keys() {
            let new_reaction_level = self.env.reactions[sub_reaction_key].get_level();
            self.scheduler.reaction_queue_s[new_reaction_level]
                .send(sub_reaction_key)
                .unwrap();
        }

        // Add this port to the list of ports that need reset
        self.set_ports_s.send(port_key).unwrap();
    }
    pub fn get_port<T: PortData>(&self, port_key: PortKey<T>) -> Option<T> {
        self.env.get_port(port_key).unwrap().get()
    }

    pub fn schedule_action<T: PortData>(
        &self,
        action_key: ActionKey<T>,
        _value: T,
        delay: super::Duration,
    ) {
        let action = &self.env.actions[action_key.data().into()];
        // TODO set value
        if action.get_is_logical() {
            let tag =
                Tag::from(self.get_logical_time()).delay(Some(delay + action.get_min_delay()));
            self.scheduler.schedule(tag, action_key.data().into(), None);
        }
    }
    pub fn shutdown(&self) {
        event!(tracing::Level::INFO, "Schduler shutdown requested...");
        // all reactors shutdown()
        // scheduler _stop = true
        *self.stop_requested.write().unwrap() = true;
    }
}

impl Debug for Scheduler {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl Scheduler {
    pub fn new(max_level: usize) -> Self {
        let (events_channel_s, events_channel_r) = crossbeam_channel::unbounded();

        let (reaction_queue_s, reaction_queue_r) =
            std::iter::repeat_with(crossbeam_channel::unbounded)
                .take(max_level + 1)
                .unzip();

        Scheduler {
            start_time: Instant::now(),
            logical_time: LogicalTime::new(),
            events_channel_s,
            events_channel_r,
            event_queue: BTreeMap::new(),
            reaction_queue_s,
            reaction_queue_r,
            stop_requested: false,
        }
    }

    pub fn get_start_time(&self) -> &Instant {
        &self.start_time
    }

    pub fn get_logical_time(&self) -> &LogicalTime {
        &self.logical_time
    }

    pub fn schedule(&self, tag: Tag, action_key: BaseActionKey, pre_handler: Option<PreHandlerFn>) {
        event!(
            tracing::Level::DEBUG,
            ?action_key,
            "Schedule ({:?})",
            tag.difference(&Tag::from(&self.start_time)),
        );
        self.events_channel_s
            .send((tag, action_key, pre_handler))
            .unwrap();
    }

    pub fn start(&mut self, mut env: Environment) -> Result<(), BoomerangError> {
        event!(tracing::Level::INFO, "Starting the scheduler...");

        for action in env.actions.values() {
            action.startup(self);
        }

        while self.next(&mut env)? {}

        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop_requested = true;
    }

    fn get_next_tagged_events(&mut self) -> Option<(Tag, EventMap, bool)> {
        // Take all available events on the channel and push them into the queue.
        for (tag, action_idx, pre_handler) in self.events_channel_r.try_iter() {
            self.event_queue
                .entry(tag)
                .or_insert(EventMap::new())
                .insert(action_idx, pre_handler);
        }

        // shutdown if there are no more events in the queue
        if self.event_queue.is_empty() && !self.stop_requested {
            if false
            // _environment->run_forever()
            {
                // wait for a new asynchronous event
                // cv_schedule.wait(lock, [this]() { return !event_queue.empty(); });
            } else {
                event!(
                    tracing::Level::DEBUG,
                    "No more events in queue. -> Terminate!"
                );
                //_environment->sync_shutdown();

                // The shutdown call might schedule shutdown reactions. If none was scheduled, we
                // simply return.
                if self.event_queue.is_empty() {
                    return None;
                }
            }
        }

        let event_entry = self.event_queue.first_entry().expect("Empty Event Queue!");

        let (t_next, run_again) = if self.stop_requested {
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
            /*
            if !environment->fast_fwd_execution() {
                // wait until the next tag or until a new event is inserted
                // asynchronously into the queue
                let status = cv_schedule.wait_until(lock, t_next.time_point());
                // Start over if the event queue was modified
                if (status == std::cv_status::no_timeout) {
                    return true;
                }
                // continue otherwise
            }
            */
            (t_next.to_owned(), true)
        };

        // Retrieve all events with tag equal to current logical time from the queue
        let tag_events = event_entry.remove();

        Some((t_next, tag_events, run_again))
    }

    fn next(&mut self, env: &mut Environment) -> Result<bool, BoomerangError> {
        let next_events = self.get_next_tagged_events();
        if next_events.is_none() {
            return Ok(false);
        }
        let (t_next, tag_events, run_again) = next_events.unwrap();

        // advance logical time
        let dt = t_next.difference(&Tag::from(&self.logical_time));
        event!(tracing::Level::DEBUG, "Advance logical time by [{:?}]", dt,);
        self.logical_time.advance_to(&t_next);

        // Execute pre-handler setup functions; this sets the values of the corresponding actions
        tag_events.values().for_each(|setup| {
            if let Some(pre_handler_fn) = setup {
                (pre_handler_fn)();
            }
        });

        // Insert all Reactions triggered by each Event/Action into the reaction_queue.
        for reaction_key in tag_events
            .keys()
            .flat_map(|&action_key| env.action_triggers[action_key].keys())
        {
            let reaction_level = env.reactions[reaction_key].get_level();
            self.reaction_queue_s[reaction_level]
                .send(reaction_key)
                .unwrap();
        }

        // Process all Reactions in the queue in order of index
        let (clear_ports_s, clear_ports_r) = crossbeam_channel::unbounded();

        for rqueue_r in self.reaction_queue_r.iter() {
            // Pull all the ReactionIdx at this level into a set
            let reactions: BTreeSet<ReactionKey> = rqueue_r.try_iter().collect();
            let sched_point = SchedulerPoint::new(&self, env, clear_ports_s.clone());
            reactions.par_iter().for_each(|&reaction_idx| {
                let reaction = &env.reactions[reaction_idx];
                event!(tracing::Level::DEBUG, ?reaction_idx, "Executing {}", reaction.get_name());
                reaction.trigger(&sched_point);
            });
            if *sched_point.stop_requested.read().unwrap() {
                self.stop_requested = true;
            }
        }

        // cleanup all triggered actions
        tag_events.keys().for_each(|&event_idx| {
            env.actions[event_idx].cleanup(self);
        });

        // Call clean on set ports
        clear_ports_r
            .try_iter()
            .map(|port_key| &env.ports[port_key])
            .for_each(|port| port.cleanup());

        Ok(run_again)
    }
}
