use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use derive_more::Display;
use std::{collections::BinaryHeap, time::Duration};
use tracing::{info, trace, warn};

use crate::{Env, Instant, ReactionSet, ReactionTriggerCtx, Tag};

#[derive(Debug, Display, Clone)]
#[display(fmt = "[tag={},terminal={}]", tag, terminal)]
pub struct ScheduledEvent {
    pub(crate) tag: Tag,
    pub(crate) reactions: ReactionSet,
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

#[derive(Debug)]
pub struct Scheduler {
    /// The environment state
    env: Env,
    /// Whether to skip wall-clock synchronization
    fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events
    keep_alive: bool,
    /// Asynchronous events sender
    event_tx: Sender<ScheduledEvent>,
    /// Asynchronous events receiver
    event_rx: Receiver<ScheduledEvent>,
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent>,
    /// Initial wall-clock time.
    start_time: Instant,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
}

impl Scheduler {
    pub fn new(env: Env, fast_forward: bool, keep_alive: bool) -> Self {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        Self {
            env,
            fast_forward,
            keep_alive,
            event_tx,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Instant::now(),
            shutdown_tag: None,
        }
    }

    /// Execute startup of the Scheduler.
    #[cfg_attr(feature = "profiling", profiling::function)]
    #[tracing::instrument(skip(self))]
    fn startup(&mut self) {
        self.start_time = Instant::now();

        let tag = Tag::new(Duration::ZERO, 0);

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to process.
        let reaction_set = self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_startup_events())
            .flatten()
            .copied()
            .collect();

        info!(tag = %tag, ?reaction_set, "Starting the execution.");
        self.process_tag(tag, reaction_set);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    #[tracing::instrument(skip(self))]
    fn cleanup(&mut self, current_tag: Tag) {
        for reactor in self.env.reactors.values_mut() {
            reactor.cleanup(current_tag);
        }

        for port in self.env.ports.values_mut() {
            port.cleanup();
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn shutdown(&mut self, shutdown_tag: Tag, _reactions: Option<ReactionSet>) {
        info!(tag = %shutdown_tag, "Shutting down.");
        let reaction_set = self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_shutdown_events())
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

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event(&mut self) -> Option<ScheduledEvent> {
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

    #[cfg_attr(feature = "profiling", profiling::function)]
    #[tracing::instrument(skip(self))]
    pub fn event_loop(&mut self) {
        self.startup();
        loop {
            // Push pending events into the queue
            for event in self.event_rx.try_iter() {
                self.event_queue.push(event);
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
                self.event_queue.push(event);
            } else {
                trace!("No more events in queue. -> Terminate!");
                break;
            }

            #[cfg(feature = "profiling")]
            profiling::finish_frame!();
        } // loop

        let shutdown_tag = self
            .shutdown_tag
            .unwrap_or_else(|| Tag::now(self.start_time));
        self.shutdown(shutdown_tag, None);
    }

    // Wait until the wall-clock time is reached
    #[cfg_attr(feature = "profiling", profiling::function)]
    #[tracing::instrument(skip(self), fields(target = ?target))]
    fn synchronize_wall_clock(&self, target: Instant) -> Option<ScheduledEvent> {
        let now = Instant::now();

        if now < target {
            let advance = target - now;
            tracing::debug!(advance = ?advance, "Need to sleep");

            match self.event_rx.recv_timeout(advance) {
                Ok(event) => {
                    tracing::debug!(event = %event, "Sleep interrupted by async event");
                    return Some(event);
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
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[cfg_attr(feature = "profiling", profiling::function)]
    #[tracing::instrument(skip(self), fields(tag = %tag, reaction_set = ?reaction_set))]
    pub fn process_tag(&mut self, tag: Tag, mut reaction_set: ReactionSet) {
        while let Some((level, reaction_keys)) = reaction_set.next() {
            tracing::info!("Level{level} with {} Reaction(s)", reaction_keys.len());

            #[cfg(feature = "parallel")]
            use rayon::prelude::{ParallelBridge, ParallelIterator};

            #[cfg(feature = "parallel")]
            let iter_ctx = self
                .env
                .iter_reaction_ctx(reaction_keys.iter())
                .par_bridge();

            #[cfg(not(feature = "parallel"))]
            let iter_ctx = self.env.iter_reaction_ctx(reaction_keys.iter());

            let inner_ctxs = iter_ctx
                .map(|trigger_ctx| {
                    let ReactionTriggerCtx {
                        reaction,
                        reactor,
                        inputs,
                        outputs,
                    } = trigger_ctx;

                    let reaction_name = reaction.get_name();
                    let reactor_name = reactor.get_name();
                    trace!("    Executing {reactor_name}/{reaction_name}.",);

                    //TODO: Plumb these iterators through into the generated reaction code.
                    let inputs = inputs.collect::<Vec<_>>();
                    let mut outputs = outputs.collect::<Vec<_>>();

                    let mut ctx = reaction.trigger(
                        self.start_time,
                        tag,
                        reactor,
                        inputs.as_slice(),
                        outputs.as_mut_slice(),
                        self.event_tx.clone(),
                    );

                    // Queue downstream reactions triggered by any ports that were set.
                    for port in outputs.into_iter() {
                        if port.is_set() {
                            ctx.enqueue_now(port.get_downstream());
                        }
                    }

                    ctx.internal
                })
                .collect::<Vec<_>>();

            for ctx in inner_ctxs.into_iter() {
                reaction_set.extend_above(ctx.reactions.into_iter(), level);

                for evt in ctx.scheduled_events.into_iter() {
                    self.event_queue.push(evt);
                }
            }
        }

        self.cleanup(tag);
    }
}
