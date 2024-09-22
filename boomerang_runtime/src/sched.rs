use crossbeam_channel::{Receiver, RecvTimeoutError};
use std::{collections::BinaryHeap, time::Duration};

use crate::{
    env::InnerEnv,
    event::{PhysicalEvent, ScheduledEvent},
    keepalive,
    key_set::KeySetView,
    Context, Env, Instant, Level, ReactionGraph, ReactionKey, ReactionSet, ReactionSetLimits,
    ReactionTriggerCtx, Tag,
};

#[derive(Debug)]
struct EventQueue {
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent>,
    /// Recycled ReactionSets to avoid allocations
    free_reaction_sets: Vec<ReactionSet>,
    /// Limits for the reaction sets
    reaction_set_limits: ReactionSetLimits,
}

impl EventQueue {
    fn new(reaction_set_limits: ReactionSetLimits) -> Self {
        Self {
            event_queue: BinaryHeap::new(),
            free_reaction_sets: Vec::new(),
            reaction_set_limits,
        }
    }

    /// Push an event into the event queue
    ///
    /// A free event is pulled from the `free_events` vector and then modified with the provided function.
    fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        let mut reaction_set = self.next_reaction_set();
        reaction_set.extend_above(reactions);
        let event = ScheduledEvent {
            tag,
            reactions: reaction_set,
            terminal,
        };
        self.event_queue.push(event);
    }

    /// Get a free [`ReactionSet`] or create a new one if none are available.
    fn next_reaction_set(&mut self) -> ReactionSet {
        self.free_reaction_sets
            .pop()
            .map(|mut reaction_set| {
                reaction_set.clear();
                reaction_set
            })
            .unwrap_or_else(|| ReactionSet::new(&self.reaction_set_limits))
    }

    /// If the event queue still has events on it, report that.
    fn shutdown(&mut self) {
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
    }
}

#[derive(Debug)]
pub struct Scheduler<'env> {
    /// The environment state
    env: InnerEnv<'env>,
    /// The reaction graph containing all static dependency and relationship information
    reaction_graph: ReactionGraph,
    /// Whether to skip wall-clock synchronization
    fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events
    keep_alive: bool,
    /// Asynchronous events receiver
    event_rx: Receiver<PhysicalEvent>,
    /// Event queue
    events: EventQueue,
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
        reaction_graph: ReactionGraph,
        fast_forward: bool,
        keep_alive: bool,
    ) -> Self {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (shutdown_tx, shutdown_rx) = keepalive::channel();
        let events = EventQueue::new(reaction_graph.reaction_set_limits.clone());

        let start_time = Instant::now();

        // Build contexts for each reaction
        let contexts = reaction_graph
            .reaction_reactors
            .iter()
            .map(|(reaction_key, reactor_key)| {
                let bank_index = reaction_graph.reactor_bank_indices[*reactor_key];
                let ctx = Context::new(
                    start_time,
                    bank_index,
                    event_tx.clone(),
                    shutdown_rx.clone(),
                );
                (reaction_key, ctx)
            })
            .collect();

        let inner_env = InnerEnv { env, contexts };

        Self {
            env: inner_env,
            reaction_graph,
            fast_forward,
            keep_alive,
            event_rx,
            events,
            start_time,
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
        let mut reaction_set = self.events.next_reaction_set();
        reaction_set.extend_above(self.reaction_graph.startup_reactions.iter().copied());

        tracing::info!(tag = %tag, ?reaction_set, "Starting the execution.");
        self.process_tag(tag, reaction_set.view());
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self) {
        tracing::info!("Shutting down.");

        // Signal to any waiting threads that the scheduler is shutting down.
        self.shutdown_tx.shutdown();
        self.events.shutdown();

        tracing::info!(
            "---- Elapsed logical time: {:?}",
            self.shutdown_tag.unwrap().get_offset()
        );
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = Instant::now().checked_duration_since(self.start_time);
        tracing::info!("---- Elapsed physical time: {:?}", physical_elapsed);

        tracing::info!("Scheduler has been shut down.");
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
            for physical_event in self.event_rx.try_iter() {
                self.events.push_event(
                    physical_event.tag,
                    physical_event.downstream_reactions(&self.reaction_graph),
                    physical_event.terminal,
                );
            }

            if let Some(mut event) = self.events.event_queue.pop() {
                tracing::debug!(event = %event, reactions = ?event.reactions, "Handling event");

                if !self.fast_forward {
                    let target = event.tag.to_logical_time(self.start_time);
                    if let Some(async_event) = self.synchronize_wall_clock(target) {
                        // Woken up by async event
                        if async_event.tag < event.tag {
                            // Re-insert both events to order them
                            self.events.event_queue.push(event);
                            self.events.event_queue.push(async_event);
                            continue;
                        } else {
                            self.events.event_queue.push(async_event);
                        }
                    }
                }

                self.process_tag(event.tag, event.reactions.view());

                // Return the ReactionSet to the free pool
                self.events.free_reaction_sets.push(event.reactions);

                if event.terminal {
                    // Break out of the event loop;
                    break;
                }
            } else if let Some(event) = self.receive_event() {
                self.events.push_event(
                    event.tag,
                    event.downstream_reactions(&self.reaction_graph),
                    event.terminal,
                );
            } else {
                tracing::debug!("No more events in queue. -> Terminate!");
                // Shutdown event will be processed at the next event loop iteration
                let tag = Tag::now(self.start_time);
                self.shutdown_tag = Some(tag);
                self.events.push_event(
                    tag,
                    self.reaction_graph.shutdown_reactions.iter().copied(),
                    true,
                );
            }
        } // loop

        self.shutdown();
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self), fields(target = ?target))]
    fn synchronize_wall_clock(&mut self, target: Instant) -> Option<ScheduledEvent> {
        let now = Instant::now();

        if now < target {
            let advance = target - now;
            tracing::debug!(advance = ?advance, "Need to sleep");

            match self.event_rx.recv_timeout(advance) {
                Ok(event) => {
                    tracing::debug!(event = %event, "Sleep interrupted by async event");
                    let mut reactions = self.events.next_reaction_set();
                    reactions.extend_above(event.downstream_reactions(&self.reaction_graph));
                    return Some(ScheduledEvent {
                        tag: event.tag,
                        reactions,
                        terminal: event.terminal,
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let remaining: Option<Duration> = target.checked_duration_since(Instant::now());
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
    #[tracing::instrument(skip(self, reaction_view), fields(tag = %tag))]
    pub fn process_tag(&mut self, tag: Tag, reaction_view: KeySetView<ReactionKey>) {
        let bump = bumpalo::Bump::new();

        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            tracing::trace!(level=?level, "Iter");

            // Safety: reaction_keys in the same level are guaranteed to be independent of each other.
            let iter_ctx = unsafe {
                self.env
                    .iter_reaction_ctx(&self.reaction_graph, &bump, reaction_keys)
            };

            #[cfg(feature = "parallel")]
            use rayon::prelude::ParallelIterator;

            #[cfg(feature = "parallel")]
            let iter_ctx = rayon::prelude::ParallelBridge::par_bridge(iter_ctx);

            let iter_ctx_res = iter_ctx.map(|trigger_ctx| {
                let ReactionTriggerCtx {
                    context,
                    reaction,
                    reactor,
                    actions,
                    ref_ports,
                    mut_ports,
                } = trigger_ctx;

                tracing::trace!(
                    "    Executing {reactor_name}/{reaction_name}.",
                    reaction_name = reaction.get_name(),
                    reactor_name = reactor.get_name()
                );

                context.reset_for_reaction(tag);

                reaction.trigger(context, reactor, actions, ref_ports, mut_ports);

                &context.trigger_res
            });

            for res in iter_ctx_res {
                if let Some(shutdown_tag) = res.scheduled_shutdown {
                    // if the new shutdown tag is earlier than the current shutdown tag, update the shutdown tag and
                    // schedule a shutdown event
                    if self.shutdown_tag.map(|t| shutdown_tag < t).unwrap_or(true) {
                        self.shutdown_tag = Some(shutdown_tag);
                        self.events.push_event(
                            shutdown_tag,
                            self.reaction_graph.shutdown_reactions.iter().copied(),
                            true,
                        );
                    }
                }

                // Submit events to the event queue for all scheduled actions
                for &(action_key, tag) in res.scheduled_actions.iter() {
                    let downstream = self.reaction_graph.action_triggers[action_key]
                        .iter()
                        .copied();
                    self.events.push_event(tag, downstream, false);
                }
            }

            // Collect all the reactions that are triggered by the ports
            let downstream = self
                .env
                .iter_set_ports()
                .flat_map(|(port_key, _)| self.reaction_graph.port_triggers[port_key].iter());

            if let Some(mut next_levels) = next_levels {
                next_levels.extend_above(downstream.copied());
            }
        });

        self.env.reset_ports();
    }
}
