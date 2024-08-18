use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use derive_more::Display;
use std::{collections::BinaryHeap, time::Duration};

use crate::{
    keepalive, Action, ActionKey, Env, Instant, LevelReactionKey, LogicalAction, ReactionSet,
    ReactionTriggerCtx, Tag, TriggerMap,
};

#[derive(Debug, Display, Clone)]
#[display(fmt = "L[tag={},terminal={}]", tag, terminal)]
pub struct ScheduledEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The set of Reactions to be executed at this tag.
    pub(crate) reactions: ReactionSet,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl Eq for ScheduledEvent {}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.terminal == other.terminal
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag
            .cmp(&other.tag)
            .then(self.terminal.cmp(&other.terminal))
            .reverse()
    }
}

#[derive(Debug, Display, Clone)]
#[display(fmt = "P[tag={},terminal={}]", tag, terminal)]
pub struct PhysicalEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The key of the action that triggered this event.
    pub(crate) key: ActionKey,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl PhysicalEvent {
    /// Create a trigger event.
    pub(crate) fn trigger(key: ActionKey, tag: Tag) -> Self {
        Self {
            tag,
            key,
            terminal: false,
        }
    }

    /// Create a shutdown event.
    pub(crate) fn shutdown(tag: Tag) -> Self {
        Self {
            tag,
            key: ActionKey::default(),
            terminal: true,
        }
    }

    /// Convert the physical event to a scheduled event.
    pub(crate) fn into_scheduled(self, triggers: &TriggerMap) -> ScheduledEvent {
        let downstream = triggers.action_triggers[self.key].iter().copied();
        ScheduledEvent {
            tag: self.tag,
            reactions: ReactionSet::from_iter(downstream),
            terminal: self.terminal,
        }
    }
}

#[derive(Debug)]
pub struct Scheduler<'env> {
    /// The environment state
    env: &'env mut Env,
    /// The trigger map
    trigger_map: TriggerMap,
    /// Whether to skip wall-clock synchronization
    fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events
    keep_alive: bool,
    /// Asynchronous events sender
    event_tx: Sender<PhysicalEvent>,
    /// Asynchronous events receiver
    event_rx: Receiver<PhysicalEvent>,
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent>,
    /// Initial wall-clock time.
    start_time: Instant,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
    /// Shutdown channel
    shutdown_tx: keepalive::Sender,
}

impl<'env> Scheduler<'env> {
    pub fn new(
        env: &'env mut Env,
        trigger_map: TriggerMap,
        fast_forward: bool,
        keep_alive: bool,
    ) -> Self {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (shutdown_tx, _) = keepalive::channel();

        Self {
            env,
            trigger_map,
            fast_forward,
            keep_alive,
            event_tx,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Instant::now(),
            shutdown_tag: None,
            shutdown_tx,
        }
    }

    /// Return an `Iterator` of reactions sensitive to `Startup` actions.
    fn iter_startup_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.env.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Startup = action {
                Some(self.trigger_map.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    /// Return an `Iterator` of reactions sensitive to `Shutdown` actions.
    fn iter_shutdown_events(&self) -> impl Iterator<Item = &[LevelReactionKey]> {
        self.env.actions.iter().filter_map(|(action_key, action)| {
            if let Action::Shutdown { .. } = action {
                Some(self.trigger_map.action_triggers[action_key].as_slice())
            } else {
                None
            }
        })
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    fn startup(&mut self) {
        self.start_time = Instant::now();

        let tag = Tag::new(Duration::ZERO, 0);

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to process.
        let reaction_set = self
            .iter_startup_events()
            .flat_map(|downstream_reactions| downstream_reactions.iter().copied())
            .collect();

        tracing::info!(tag = %tag, ?reaction_set, "Starting the execution.");
        self.process_tag(tag, reaction_set);
    }

    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self, shutdown_tag: Tag, _reactions: Option<ReactionSet>) {
        tracing::info!("Shutting down.");

        // Signal to any waiting threads that the scheduler is shutting down.
        self.shutdown_tx.shutdown();

        let reaction_set = self
            .iter_shutdown_events()
            .flat_map(|downstream_reactions| downstream_reactions.iter().copied())
            .collect();

        self.process_tag(shutdown_tag, reaction_set);

        // If the event queue still has events on it, report that.
        if !self.event_queue.is_empty() {
            tracing::warn!(
                "---- There are {} unprocessed future events on the event queue.",
                self.event_queue.len()
            );
            let event = self.event_queue.peek().unwrap();
            tracing::warn!(
                "---- The first future event has timestamp {:?} after start time.",
                event.tag.get_offset()
            );
        }

        tracing::info!("---- Elapsed logical time: {:?}", shutdown_tag.get_offset());
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = Instant::now().checked_duration_since(self.start_time);
        tracing::info!("---- Elapsed physical time: {:?}", physical_elapsed);

        tracing::info!("Scheduler has been shut down.");
    }

    #[tracing::instrument(skip(self))]
    fn cleanup(&mut self, current_tag: Tag) {
        for action in self.env.actions.values_mut() {
            if let Action::Logical(LogicalAction { values, .. }) = action {
                // Clear action values at the current tag
                values.remove(current_tag);
            }
        }

        for port in self.env.ports.values_mut() {
            port.cleanup();
        }
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event(&mut self) -> Option<PhysicalEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(Instant::now()) {
                tracing::debug!(timeout = ?timeout, "Waiting for async event.");
                self.event_rx.recv_timeout(timeout).ok()
            } else {
                tracing::debug!("Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.keep_alive {
            tracing::debug!("Waiting indefinitely for async event.");
            self.event_rx.recv().ok()
        } else {
            None
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn event_loop(&mut self) {
        self.startup();
        loop {
            // Push pending events into the queue
            for event in self.event_rx.try_iter() {
                self.event_queue
                    .push(event.into_scheduled(&self.trigger_map));
            }

            if let Some(event) = self.event_queue.pop() {
                tracing::debug!(event = %event, reactions = ?event.reactions, "Handling event");

                if !self.fast_forward {
                    let target = event.tag.to_logical_time(self.start_time);
                    if let Some(async_event) = self.synchronize_wall_clock(target) {
                        // Woken up by async event
                        if async_event.tag < event.tag {
                            // Re-insert both events to order them
                            self.event_queue.push(event);
                            self.event_queue.push(async_event);
                            continue;
                        } else {
                            self.event_queue.push(async_event);
                        }
                    }
                }

                if self
                    .shutdown_tag
                    .as_ref()
                    .map(|shutdown_tag| shutdown_tag == &event.tag)
                    .unwrap_or(event.terminal)
                {
                    self.shutdown_tag = Some(event.tag);
                    break;
                }
                self.process_tag(event.tag, event.reactions);
            } else if let Some(event) = self.receive_event() {
                self.event_queue
                    .push(event.into_scheduled(&self.trigger_map));
            } else {
                tracing::debug!("No more events in queue. -> Terminate!");
                break;
            }
        } // loop

        let shutdown_tag = self
            .shutdown_tag
            .unwrap_or_else(|| Tag::now(self.start_time));
        self.shutdown(shutdown_tag, None);
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self), fields(target = ?target))]
    fn synchronize_wall_clock(&self, target: Instant) -> Option<ScheduledEvent> {
        let now = Instant::now();

        if now < target {
            let advance = target - now;
            tracing::debug!(advance = ?advance, "Need to sleep");

            match self.event_rx.recv_timeout(advance) {
                Ok(event) => {
                    tracing::debug!(event = %event, "Sleep interrupted by async event");
                    return Some(event.into_scheduled(&self.trigger_map));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let remaining = target.checked_duration_since(Instant::now());
                    if let Some(remaining) = remaining {
                        tracing::debug!(remaining = ?remaining,
                            "Sleep interrupted disconnect, sleeping for remaining",
                        );
                        std::thread::sleep(remaining);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }

        if now > target {
            let delay = now - target;
            tracing::warn!(delay = ?delay, "running late");
        }

        None
    }

    /// Process the reactions at this tag in increasing order of level.
    ///
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[tracing::instrument(skip(self, reaction_set), fields(tag = %tag))]
    pub fn process_tag(&mut self, tag: Tag, mut reaction_set: ReactionSet) {
        let bump = bumpalo::Bump::new();

        while let Some((level, reaction_keys)) = reaction_set.next() {
            tracing::trace!(level=?level, reaction_keys = ?reaction_keys, "Iter");

            // Safety: reaction_keys in the same level are guaranteed to be independent of each other.
            let iter_ctx = unsafe { self.env.iter_reaction_ctx(&bump, reaction_keys.iter()) };

            #[cfg(feature = "parallel")]
            use rayon::prelude::ParallelIterator;

            #[cfg(feature = "parallel")]
            let iter_ctx = rayon::prelude::ParallelBridge::par_bridge(iter_ctx);

            let iter_ctx_res = iter_ctx.map(|trigger_ctx| {
                let ReactionTriggerCtx {
                    reaction,
                    reactor,
                    actions,
                    inputs,
                    outputs,
                } = trigger_ctx;

                tracing::trace!(
                    "    Executing {reactor_name}/{reaction_name}.",
                    reaction_name = reaction.get_name(),
                    reactor_name = reactor.get_name()
                );

                reaction.trigger(
                    self.start_time,
                    tag,
                    reactor,
                    actions,
                    inputs,
                    outputs,
                    self.event_tx.clone(),
                    self.shutdown_tx.new_receiver(),
                )
            });

            iter_ctx_res.for_each(|res| {
                // Update any earlier shutdown requested
                self.shutdown_tag = res
                    .scheduled_shutdown
                    .iter()
                    .chain(self.shutdown_tag.iter())
                    .min()
                    .copied();

                // Submit events to the event queue for all scheduled actions
                self.event_queue
                    .extend(res.scheduled_actions.iter().map(|&(action_key, tag)| {
                        let downstream = self.trigger_map.action_triggers[action_key].iter();
                        ScheduledEvent {
                            tag,
                            reactions: ReactionSet::from_iter(downstream.copied()),
                            terminal: false,
                        }
                    }));
            });

            // Collect all the reactions that are triggered by the ports
            self.env.ports.iter().for_each(|(port_key, port)| {
                if port.is_set() {
                    let downstream = self.trigger_map.port_triggers[port_key]
                        .iter()
                        .filter(|(trigger_level, _)| *trigger_level > level)
                        .copied();
                    reaction_set.extend_above(downstream, level);
                }
            });
        }

        self.cleanup(tag);
    }
}
