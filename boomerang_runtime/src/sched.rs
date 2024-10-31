use std::{collections::BinaryHeap, pin::Pin, time::Duration};

use crossbeam_channel::RecvTimeoutError;

use crate::{
    build_reaction_contexts,
    env::Enclave,
    event::{AsyncEvent, ScheduledEvent},
    keepalive,
    key_set::KeySetView,
    store::Store,
    Env, Level, ReactionGraph, ReactionKey, ReactionSet, ReactionSetLimits, Tag, Timestamp,
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

    /// Peek the tag of the next event in the queue
    fn peek_tag(&self) -> Option<Tag> {
        self.event_queue.peek().map(|event| event.tag)
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

#[derive(Debug, Clone)]
pub struct Config {
    /// Whether to skip wall-clock synchronization (execute as fast as possible)
    pub fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events.
    /// If `false`, the scheduler will terminate when there are no more events to process.
    pub keep_alive: bool,
    /// The size of the physical event queue.
    pub physical_event_q_size: usize,
    /// Stop the scheduler after a certain amount of time has passed.
    pub timeout: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fast_forward: false,
            keep_alive: false,
            physical_event_q_size: 1024,
            timeout: None,
        }
    }
}

impl Config {
    pub fn with_fast_forward(mut self, fast_forward: bool) -> Self {
        self.fast_forward = fast_forward;
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Set the capacity of the physical event queue.
    ///
    /// If the queue is full, this call will block until there is space available.
    pub fn with_queue_size(mut self, physical_event_q_size: usize) -> Self {
        self.physical_event_q_size = physical_event_q_size;
        self
    }

    /// Set a timeout for the scheduler.
    /// The scheduler will terminate after the given duration has passed.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug)]
struct LogicalTimeBarrier {
    /// The last released tag
    released_tag: Tag,
    /// Receiver for upstream released tags
    upstream_rx: crate::Receiver<Tag>,
}

impl LogicalTimeBarrier {
    pub fn release_tag(&mut self, tag: Tag) {
        tracing::trace!(tag = %tag, "Release tag");
        assert!(
            tag >= self.released_tag,
            "Cannot release a tag earlier than the last released tag"
        );
        self.released_tag = tag;
    }

    #[inline]
    /// Try to acquire the given tag without blocking.
    pub fn try_acquire_tag(&mut self, tag: Tag) -> bool {
        // First check if this tag is already released
        if tag <= self.released_tag {
            return true;
        }
        // Check if there are any upstream tags that have been released
        if let Ok(released_tag) = self.upstream_rx.try_recv() {
            self.release_tag(released_tag);
            return tag <= self.released_tag;
        }
        false
    }

    /// Acquire the given tag, blocking until it is released, or an [`AsyncEvent`] is received.
    ///
    /// If an async event is received, it is returned to the caller. A return value of `None` indicates that the tag has been released.
    #[inline]
    pub fn acquire_tag(
        &mut self,
        tag: Tag,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Option<AsyncEvent> {
        tracing::trace!(tag = %tag, "Acquiring tag");

        if self.try_acquire_tag(tag) {
            return None;
        }

        // Block until the tag is released
        loop {
            crossbeam_channel::select! {
                recv(self.upstream_rx) -> msg => {
                    if let Ok(released_tag) = msg {
                        self.release_tag(released_tag);
                        if tag <= self.released_tag {
                            return None;
                        }
                    }
                }
                recv(event_rx) -> msg => {
                    return msg.ok();
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Scheduler {
    /// The scheduler config
    config: Config,
    /// The reactor runtime store
    store: Pin<Box<Store>>,
    /// The reaction graph containing all static dependency and relationship information
    reaction_graph: ReactionGraph,
    /// Asynchronous events receiver
    event_rx: crate::Receiver<AsyncEvent>,
    /// Event queue
    events: EventQueue,
    /// Initial physical time.
    start_time: Timestamp,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
    /// Shutdown channel
    shutdown_tx: keepalive::Sender,
    /// Logical time barriers for each upstream source
    upstream_barriers: Vec<LogicalTimeBarrier>,

    /// The senders for downstream tags
    downstream_tx: Vec<crate::Sender<Tag>>,
}

impl Scheduler {
    /// Create a new Scheduler instance.
    ///
    /// The Scheduler will be initialized with the provided environment and reaction graph.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing all the runtime data structures.
    /// * `reaction_graph` - The reaction graph containing all static dependency and relationship information.
    pub fn new(enclave: Enclave, config: Config) -> Self {
        let Enclave {
            env,
            graph,
            event_tx,
            event_rx,
            downstream_tx,
            upstream_rx,
            shutdown_tx,
            shutdown_rx,
        } = enclave;

        let start_time = Timestamp::now();

        let upstream_barriers = upstream_rx
            .into_iter()
            .map(|upstream_rx| LogicalTimeBarrier {
                released_tag: Tag::new(Duration::ZERO, 0),
                upstream_rx,
            })
            .collect();

        if let Some(timeout) = config.timeout {
            let shutdown_event = AsyncEvent::Shutdown { delay: timeout };
            event_tx.send(shutdown_event).unwrap();
        }

        // Find the maximum level in the reaction graph
        let max_level = graph
            .action_triggers
            .values()
            .chain(graph.port_triggers.values())
            .flat_map(|level_reactions| level_reactions.iter().map(|(level, _)| level))
            .max()
            .copied()
            .unwrap_or_default();

        let reaction_set_limits = ReactionSetLimits {
            max_level,
            num_keys: env.reactions.len(),
        };
        let events = EventQueue::new(reaction_set_limits);

        // Build contexts for each reaction
        let contexts = build_reaction_contexts(&graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);

        Self {
            config,
            store,
            reaction_graph: graph,
            event_rx,
            events,
            start_time,
            shutdown_tag: None,
            shutdown_tx,
            upstream_barriers,
            downstream_tx,
        }
    }

    /// Handle an asynchronous event from the event queue
    fn handle_async_event(
        event: AsyncEvent,
        tag: Tag,
        start_time: Timestamp,
        events: &mut EventQueue,
        store: &mut Pin<Box<Store>>,
        reaction_graph: &ReactionGraph,
    ) {
        let reactions = event.downstream_reactions(reaction_graph);
        match event {
            AsyncEvent::Logical { delay, key, value } => {
                let tag = tag.delay(delay);
                events.push_event(tag, reactions, false);
                store.push_action_value(key, tag, value);
            }
            AsyncEvent::Physical { time, key, value } => {
                let tag = Tag::from_physical_time(start_time, time);
                events.push_event(tag, reactions, false);
                store.push_action_value(key, tag, value);
            }
            AsyncEvent::Shutdown { delay } => {
                events.push_event(tag.delay(delay), reactions, true);
                //self.shutdown_tag = Some(tag);
            }
        }
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    fn startup(&mut self) -> Tag {
        self.start_time = Timestamp::now();

        // Logical time starts at 0
        let tag = Tag::new(Duration::ZERO, 1);

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to process.
        self.events.push_event(
            tag,
            self.reaction_graph.startup_reactions.iter().copied(),
            false,
        );

        tracing::info!(tag = %tag, "Starting the execution.");
        tag
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
        let physical_elapsed = Timestamp::now().checked_duration_since(self.start_time);
        tracing::info!("---- Elapsed physical time: {:?}", physical_elapsed);

        tracing::info!("Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event(&mut self) -> Option<AsyncEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(Timestamp::now()) {
                tracing::debug!(timeout = ?timeout, "Waiting for async event.");
                self.event_rx.recv_timeout(timeout).ok()
            } else {
                tracing::debug!("Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.config.keep_alive {
            tracing::debug!("Waiting indefinitely for async event.");
            self.event_rx.recv().ok()
        } else {
            None
        }
    }

    /// Release the current tag to downstream reactors
    fn release_tag(&self, current_tag: Tag) {
        tracing::trace!(tag = %current_tag, "Releasing tag downstream");
        for sender in &self.downstream_tx {
            sender.send(current_tag).unwrap();
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn event_loop(&mut self) {
        let mut current_tag = self.startup().decrement();

        loop {
            // Release the current tag to downstream reactors
            self.release_tag(current_tag);

            // Push pending events into the queue
            for async_event in self.event_rx.try_iter() {
                Self::handle_async_event(
                    async_event,
                    current_tag,
                    self.start_time,
                    &mut self.events,
                    &mut self.store,
                    &self.reaction_graph,
                );
            }

            if let Some(next_tag) = self.events.peek_tag() {
                if !self.config.fast_forward {
                    let target = next_tag.to_logical_time(self.start_time);
                    if self.synchronize_wall_clock(target, current_tag) {
                        // Woken up by async event
                        continue;
                    }
                }

                // Wait until all upstream barriers are released
                for barrier in self.upstream_barriers.iter_mut() {
                    if let Some(async_event) = barrier.acquire_tag(next_tag, &self.event_rx) {
                        Self::handle_async_event(
                            async_event,
                            current_tag,
                            self.start_time,
                            &mut self.events,
                            &mut self.store,
                            &self.reaction_graph,
                        );
                    }
                }
            }

            if let Some(mut event) = self.events.event_queue.pop() {
                tracing::debug!(event = %event, "Handling event");

                if Some(event.tag) == self.events.peek_tag() {
                    // The next event is at the same time as the one we are processing
                    // This can happen if the event we are processing triggers a new event at the same time
                    // We need to process all events at the same time before moving on
                    //while let Some(next_event) = self.events.event_queue.pop() {
                    //    if next_event.tag == event.tag {
                    //        event.reactions.extend_above(next_event.reactions.view());
                    //    } else {
                    //        self.events.event_queue.push(next_event);
                    //        break;
                    //    }
                    //}
                    tracing::warn!("Next event is at the same time as the one we are processing");
                }

                self.process_tag(event.tag, event.reactions.view());

                // Return the ReactionSet to the free pool
                self.events.free_reaction_sets.push(event.reactions);

                current_tag = event.tag;

                if event.terminal {
                    // Break out of the event loop;
                    self.shutdown_tag = Some(current_tag);
                    break;
                }
            } else if let Some(async_event) = self.receive_event() {
                Self::handle_async_event(
                    async_event,
                    current_tag,
                    self.start_time,
                    &mut self.events,
                    &mut self.store,
                    &self.reaction_graph,
                );
            } else {
                tracing::debug!("No more events in queue. -> Terminate!");
                // Shutdown event will be processed at the next event loop iteration
                self.shutdown_tag = Some(current_tag);
                self.events.push_event(
                    current_tag,
                    self.reaction_graph.shutdown_reactions.iter().copied(),
                    true,
                );
            }
        } // loop

        // Release the current tag to downstream reactors
        self.release_tag(current_tag);

        self.shutdown();
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self), fields(target = ?target))]
    fn synchronize_wall_clock(&mut self, target: Timestamp, current_tag: Tag) -> bool {
        let now = Timestamp::now();

        if now < target {
            let advance = target - now;
            tracing::debug!(advance = ?advance, "Need to sleep");

            match self.event_rx.recv_timeout(advance) {
                Ok(event) => {
                    tracing::debug!(event = %event, "Sleep interrupted by");
                    Self::handle_async_event(
                        event,
                        current_tag,
                        self.start_time,
                        &mut self.events,
                        &mut self.store,
                        &self.reaction_graph,
                    );
                    return true;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let remaining: Option<Duration> =
                        target.checked_duration_since(Timestamp::now());
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

        false
    }

    /// Process the reactions at this tag in increasing order of level.
    ///
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[tracing::instrument(skip(self, reaction_view), fields(tag = %tag))]
    pub fn process_tag(&mut self, tag: Tag, reaction_view: KeySetView<ReactionKey>) {
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            tracing::trace!(level=?level, "Iter");

            // Safety: reaction_keys in the same level are guaranteed to be independent of each other.
            let iter_ctx = unsafe { self.store.iter_borrow_storage(reaction_keys) };

            #[cfg(feature = "parallel")]
            use rayon::prelude::ParallelIterator;

            #[cfg(feature = "parallel")]
            let iter_ctx = rayon::prelude::ParallelBridge::par_bridge(iter_ctx);

            let iter_ctx_res = iter_ctx.map(|trigger_ctx| trigger_ctx.trigger(tag));

            #[cfg(feature = "parallel")]
            let iter_ctx_res = iter_ctx_res.collect::<Vec<_>>();

            for trigger_res in iter_ctx_res {
                if let Some(shutdown_tag) = trigger_res.scheduled_shutdown {
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
                for &(action_key, tag) in trigger_res.scheduled_actions.iter() {
                    let downstream = self.reaction_graph.action_triggers[action_key]
                        .iter()
                        .copied();
                    self.events.push_event(tag, downstream, false);
                }
            }

            // Collect all the reactions that are triggered by the ports
            let downstream = self
                .store
                .iter_set_port_keys()
                .flat_map(|port_key| self.reaction_graph.port_triggers[port_key].iter());

            if let Some(mut next_levels) = next_levels {
                next_levels.extend_above(downstream.copied());
            }
        });

        self.store.reset_ports();
    }

    /// Consume the scheduler and return the `Env` instance.
    ///
    /// This method is useful for testing purposes, as it allows the caller to inspect reactor states after the
    /// scheduler has been run.
    pub fn into_env(self) -> Env {
        self.store.into_env()
    }
}
