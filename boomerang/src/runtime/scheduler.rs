use crate::BoomerangError;

use super::{
    environment::Environment,
    time::{LogicalTime, Tag},
    ActionIndex, PortData, PortIndex, ReactionIndex,
};
use crossbeam_channel::{Receiver, Sender};
use rayon::prelude::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    sync::{Arc, RwLock},
    time::Instant,
};
use tracing::event;

type PreHandlerFn = Box<dyn Fn() -> () + Send + Sync>;
type EventMap = BTreeMap<ActionIndex, Option<PreHandlerFn>>;
type ScheduledEvent = (Tag, ActionIndex, Option<PreHandlerFn>);

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
    reaction_queue_s: Vec<Sender<ReactionIndex>>,
    reaction_queue_r: Vec<Receiver<ReactionIndex>>,

    /// Stop requested
    stop_requested: bool,
}

#[derive(Debug, Clone)]
pub struct SchedulerPoint<'a> {
    start_time: &'a Instant,
    logical_time: &'a LogicalTime,
    env: &'a Environment,
    reaction_queue_s: &'a Vec<Sender<ReactionIndex>>,
    set_ports_s: Sender<PortIndex>,
    stop_requested: Arc<RwLock<bool>>,
}

impl<'a> SchedulerPoint<'a> {
    fn new(scheduler: &'a Scheduler, env: &'a Environment, set_ports_s: Sender<PortIndex>) -> Self {
        SchedulerPoint {
            start_time: &scheduler.start_time,
            logical_time: &scheduler.logical_time,
            env,
            reaction_queue_s: &scheduler.reaction_queue_s,
            set_ports_s,
            stop_requested: Arc::new(RwLock::new(false)),
        }
    }

    pub fn get_start_time(&self) -> &Instant {
        self.start_time
    }
    pub fn get_logical_time(&self) -> &LogicalTime {
        self.logical_time
    }
    pub fn set_port<T: PortData>(&self, port_idx: PortIndex, value: T) {
        self.env.get_port(port_idx).unwrap().set(Some(value));

        // Schedule any reactions triggered by this port being set.
        for sub_reaction_idx in self.env.runtime_ports[&port_idx].get_triggers() {
            let new_reaction_level = self.env.runtime_reactions[sub_reaction_idx].get_level();
            event!(
                tracing::Level::DEBUG,
                ?sub_reaction_idx,
                "Triggerd by Port (new level {})",
                new_reaction_level
            );
            self.reaction_queue_s[new_reaction_level]
                .send(*sub_reaction_idx)
                .unwrap();
        }

        // Add this port to the list of ports that need reset
        self.set_ports_s.send(port_idx).unwrap();
    }
    pub fn get_port<T: PortData>(&self, port_idx: PortIndex) -> Option<T> {
        self.env.get_port(port_idx).unwrap().get()
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

    pub fn schedule(&self, tag: Tag, action_idx: ActionIndex, pre_handler: Option<PreHandlerFn>) {
        event!(tracing::Level::DEBUG, %action_idx, "Schedule");
        self.events_channel_s
            .send((tag, action_idx, pre_handler))
            .unwrap();
    }

    pub fn start(&mut self, mut env: Environment) -> Result<(), BoomerangError> {
        event!(tracing::Level::INFO, "Starting the scheduler...");

        env.runtime_actions
            .values()
            .for_each(|action| action.startup(self));

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
        event!(
            tracing::Level::DEBUG,
            "Advance logical time to tag [{}]",
            t_next
        );
        self.logical_time.advance_to(&t_next);

        // Execute pre-handler setup functions; this sets the values of the corresponding actions
        tag_events.values().for_each(|setup| {
            if let Some(pre_handler_fn) = setup {
                (pre_handler_fn)();
            }
        });

        // Insert all Reactions triggered by each Event/Action into the reaction_queue.
        for reaction_idx in tag_events
            .keys()
            .flat_map(|action_idx| env.runtime_actions[action_idx].get_triggers())
        {
            let reaction_level = env.runtime_reactions[reaction_idx].get_level();
            self.reaction_queue_s[reaction_level]
                .send(*reaction_idx)
                .unwrap();
        }

        // Process all Reactions in the queue in order of index
        let (clear_ports_s, clear_ports_r) = crossbeam_channel::unbounded();

        for rqueue_r in self.reaction_queue_r.iter() {
            // Pull all the ReactionIdx at this level into a set
            let reactions: BTreeSet<ReactionIndex> = rqueue_r.try_iter().collect();
            let sched_point = SchedulerPoint::new(&self, env, clear_ports_s.clone());
            reactions.par_iter().for_each(|reaction_idx| {
                let reaction = &env.runtime_reactions[&reaction_idx];
                event!(tracing::Level::DEBUG, ?reaction_idx, ?reaction, "Executing");
                reaction.trigger(&sched_point);
            });
            if *sched_point.stop_requested.read().unwrap() {
                self.stop_requested = true;
            }
        }

        // cleanup all triggered actions
        tag_events.keys().for_each(|event_idx| {
            env.runtime_actions[event_idx].cleanup(self);
        });

        // Call clean on set ports
        clear_ports_r
            .try_iter()
            .map(|port_idx| &env.runtime_ports[&port_idx])
            .for_each(|port| port.cleanup());

        Ok(run_again)
    }
}
