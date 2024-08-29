use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::{collections::BinaryHeap, time::Duration};

use crate::{
    event::{PhysicalEvent, ScheduledEvent},
    keepalive, Action, Env, Instant, LogicalAction, ReactionSet, ReactionTriggerCtx, Tag,
    TriggerMap,
};

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

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    fn startup(&mut self) {
        self.start_time = Instant::now();

        let tag = Tag::new(Duration::ZERO, 0);

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to process.
        let reaction_set =
            ReactionSet::from_iter(self.trigger_map.startup_reactions.clone().into_iter());

        tracing::info!(tag = %tag, ?reaction_set, "Starting the execution.");
        self.process_tag(tag, reaction_set);
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self) {
        tracing::info!("Shutting down.");

        // Signal to any waiting threads that the scheduler is shutting down.
        self.shutdown_tx.shutdown();

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

        tracing::info!(
            "---- Elapsed logical time: {:?}",
            self.shutdown_tag.unwrap().get_offset()
        );
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

                self.process_tag(event.tag, event.reactions);

                if event.terminal {
                    // Break out of the event loop;
                    break;
                }
            } else if let Some(event) = self.receive_event() {
                self.event_queue
                    .push(event.into_scheduled(&self.trigger_map));
            } else {
                tracing::debug!("No more events in queue. -> Terminate!");
                // Shutdown event will be processed at the next event loop iteration
                let tag = Tag::now(self.start_time);
                self.shutdown_tag = Some(tag);
                self.event_queue
                    .push(ScheduledEvent::shutdown(tag, &self.trigger_map));
            }
        } // loop

        self.shutdown();
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
                if let Some(shutdown_tag) = res.scheduled_shutdown {
                    // if the new shutdown tag is earlier than the current shutdown tag, update the shutdown tag and
                    // schedule a shutdown event
                    if self.shutdown_tag.map(|t| shutdown_tag < t).unwrap_or(true) {
                        self.shutdown_tag = Some(shutdown_tag);
                        self.event_queue
                            .push(ScheduledEvent::shutdown(shutdown_tag, &self.trigger_map));
                    }
                }

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
