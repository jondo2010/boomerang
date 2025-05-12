use std::{collections::BinaryHeap, pin::Pin};

use kanal::ReceiveErrorTimeout;

use crate::{
    build_reaction_contexts,
    env::{Enclave, EnclaveKey},
    event::{AsyncEvent, ScheduledEvent},
    keepalive,
    key_set::KeySetView,
    store::Store,
    CommonContext, Duration, Env, Level, ReactionGraph, ReactionKey, ReactionSet,
    ReactionSetLimits, SendContext, Tag,
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
        if self.peek_tag() == Some(tag) {
            // If the tag is the same as the next event, merge the reactions
            let mut event = self.event_queue.peek_mut().unwrap();
            event.reactions.extend_above(reactions);
            event.terminal = event.terminal || terminal;
        } else {
            // Otherwise, push a new event
            let mut reaction_set = self.next_reaction_set();
            reaction_set.extend_above(reactions);
            let event = ScheduledEvent {
                tag,
                reactions: reaction_set,
                terminal,
            };
            self.event_queue.push(event);
        }
    }

    /// Pop the next event from the event queue.
    ///
    /// Any subsequent events with the same tag are merged into the returned event.
    fn pop_next_event(&mut self) -> Option<ScheduledEvent> {
        if let Some(mut event) = self.event_queue.pop() {
            // Merge events with the same tag
            while let Some(next_event) = self.event_queue.peek() {
                if next_event.tag == event.tag {
                    let next_event = self.event_queue.pop().unwrap();
                    event.reactions.merge(&next_event.reactions);
                    event.terminal = event.terminal || next_event.terminal;

                    // Return the ReactionSet to the free pool
                    self.free_reaction_sets.push(next_event.reactions);
                } else {
                    break;
                }
            }

            return Some(event);
        }

        None
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
                "---- The first future event has timestamp {} after start time.",
                event.tag.offset()
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
    provisional_tag: Tag,
    /// The send context for the upstream enclave
    upstream_ctx: SendContext,
    /// Optional delay for the upstream connection
    upstream_delay: Option<Duration>,
}

impl LogicalTimeBarrier {
    #[tracing::instrument(skip(self), fields(tag = %tag, released = %self.released_tag))]
    pub fn release_tag(&mut self, tag: Tag) {
        tracing::trace!("Release");

        if tag < self.released_tag {
            tracing::warn!(
                "Cannot release a tag ({tag}) earlier than the last released tag {}",
                self.released_tag
            );
        }
        self.released_tag = tag;
        // Reset the provisional tag
        self.provisional_tag = Tag::NEVER;
    }

    pub fn release_tag_provisional(&mut self, tag: Tag) {
        if tag <= self.provisional_tag {
            self.release_tag(tag);
        }
    }

    #[inline]
    /// Try to acquire the given tag without blocking.
    pub fn try_acquire_tag(&mut self, tag: Tag) -> bool {
        tag <= self.released_tag
    }

    /// Acquire the given tag, blocking until it is released, or an [`AsyncEvent`] is received.
    ///
    /// If an async event is received, it is returned to the caller. A return value of `None` indicates that the tag has been released.
    #[inline]
    #[tracing::instrument(skip(self, tag, this_enclave, event_rx), fields(tag = %tag))]
    pub fn acquire_tag(
        &mut self,
        tag: Tag,
        this_enclave: EnclaveKey,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Option<AsyncEvent> {
        // Since this is a delayed connection, we can go back in time and need to
        // acquire the latest upstream tag that can create an event at the given
        // tag.
        let upstream_tag = if let Some(delay) = self.upstream_delay {
            tag.pre(delay)
        } else {
            tag
        };

        tracing::trace!(upstream_tag = %upstream_tag, "Try acquire");
        if self.try_acquire_tag(upstream_tag) {
            return None;
        }

        tracing::trace!(%upstream_tag, "Releasing provisional tag");
        self.provisional_tag = upstream_tag;
        if !self
            .upstream_ctx
            .release_provisional(this_enclave, upstream_tag)
        {
            // The upstream has terminated try to return a queued event here. If the upstream terminated, we probably
            // have an event queued from it. This prevents pre-mature termination of this enclave.
            tracing::warn!("Upstream has terminated");
            return event_rx.try_recv().expect("Upstream terminated");
        }

        // Block until the tag is released
        tracing::trace!("Blocking");
        event_rx.recv().ok()
    }
}

#[derive(Debug, Default)]
pub struct Stats {
    /// Number of `tag`s processed
    processed_tags: usize,
    /// Number of reactions processed
    processed_reactions: usize,
    /// Number of scheduled async events
    processed_events: usize,
    /// Number of ports set
    set_ports: usize,
    /// Number of scheduled, sync actions
    scheduled_actions: usize,
}

impl Stats {
    pub fn increment_processed_tags(&mut self) {
        self.processed_tags += 1;
    }
    pub fn increment_processed_reactions(&mut self, count: usize) {
        self.processed_reactions += count;
    }
    pub fn increment_processed_events(&mut self) {
        self.processed_events += 1;
    }
    pub fn increment_set_ports(&mut self) {
        self.set_ports += 1;
    }
    pub fn increment_scheduled_actions(&mut self, count: usize) {
        self.scheduled_actions += count;
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stats")
            .field("Processed tags", &self.processed_tags)
            .field("Processed reactions", &self.processed_reactions)
            .field("Processed events", &self.processed_events)
            .field("Set ports", &self.set_ports)
            .field("Scheduled actions", &self.scheduled_actions)
            .finish()
    }
}

#[derive(Debug)]
pub struct Scheduler {
    /// The enclave key
    key: EnclaveKey,
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
    start_time: std::time::Instant,
    /// Current tag
    current_tag: Tag,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
    /// Shutdown channel
    shutdown_tx: keepalive::Sender,
    /// Logical time barriers for each upstream enclave
    upstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    /// The senders for downstream enclaves
    downstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    /// Runtime statistics
    stats: Stats,
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
    pub fn new(key: EnclaveKey, enclave: Enclave, config: Config) -> Self {
        let Enclave {
            env,
            graph,
            event_tx,
            event_rx,
            downstream_enclaves,
            upstream_enclaves,
            shutdown_tx,
            shutdown_rx,
        } = enclave;

        let start_time = std::time::Instant::now();

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
        let contexts = build_reaction_contexts(key, &graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);

        let upstream_enclaves = upstream_enclaves
            .into_iter()
            .map(|(enclave_key, upstream_ref)| {
                (
                    enclave_key,
                    LogicalTimeBarrier {
                        released_tag: Tag::NEVER,
                        provisional_tag: Tag::NEVER,
                        upstream_ctx: upstream_ref.send_ctx,
                        upstream_delay: upstream_ref.delay,
                    },
                )
            })
            .collect();

        let downstream_enclaves = downstream_enclaves
            .into_iter()
            .map(|(enclave_key, downstream_ref)| (enclave_key, downstream_ref.send_ctx))
            .collect();

        Self {
            key,
            config,
            store,
            reaction_graph: graph,
            event_rx,
            events,
            start_time,
            current_tag: Tag::NEVER,
            shutdown_tag: None,
            shutdown_tx,
            upstream_enclaves,
            downstream_enclaves,
            stats: Stats::default(),
        }
    }

    /// Handle an asynchronous event from the event queue
    #[tracing::instrument(skip(self, ), fields(event = %event))]
    fn handle_async_event(&mut self, event: AsyncEvent) {
        self.stats.increment_processed_events();
        tracing::trace!("Handling");
        let reactions = event.downstream_reactions(&self.reaction_graph);
        match event {
            AsyncEvent::TagRelease { enclave, tag } => {
                self.upstream_enclaves
                    .get_mut(enclave)
                    .expect("Unknown upstream enclave")
                    .release_tag(tag);
            }
            AsyncEvent::TagReleaseProvisional { enclave, tag } => {
                if tag <= self.current_tag {
                    if tag < self.current_tag {
                        tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    }
                    return;
                }
                // TagReleaseProvisional events are coming from downstream enclaves. If this enclave is also an upstream
                // (cycle), then also release it provisionally.
                if let Some(barrier) = self.upstream_enclaves.get_mut(enclave) {
                    barrier.release_tag_provisional(tag);
                }
                self.events.push_event(tag, reactions, false);
            }
            AsyncEvent::Logical { tag, key, value } => {
                if tag <= self.current_tag {
                    tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    return;
                }
                self.events.push_event(tag, reactions, false);
                self.store.push_action_value(key, tag, value);
            }
            AsyncEvent::Physical { time, key, value } => {
                let tag = Tag::from_physical_time(self.start_time, time);
                self.events.push_event(tag, reactions, false);
                self.store.push_action_value(key, tag, value);
            }
            AsyncEvent::Shutdown { delay } => {
                self.events
                    .push_event(self.current_tag.delay(delay), reactions, true);
                //self.shutdown_tag = Some(tag);
            }
        }
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    pub fn startup(&mut self) {
        let tag = Tag::ZERO;

        // Set up the startup reactions
        for (delay, level_reactions) in &self.reaction_graph.startup_reactions {
            let t = Tag::new(*delay, 0);
            self.events
                .push_event(t, level_reactions.iter().inspect(|(lvl, reaction_key)| {
                    tracing::trace!(level = %lvl, reaction = %reaction_key, tag = %t, "Startup reaction");
                }).copied(), false);
        }

        // Schedule a shutdown event if a timeout is set
        if let Some(timeout) = self.config.timeout {
            let t = tag.delay(timeout);
            tracing::info!(tag = %t, "Scheduling shutdown");
            self.events.push_event(
                t,
                self.reaction_graph.shutdown_reactions.iter().copied(),
                true,
            );
        }

        tracing::info!(tag = %tag, "Starting the execution.");

        self.current_tag = tag.decrement();

        // Release the current tag to downstream reactors
        self.release_tag_downstream(self.current_tag);

        self.start_time = std::time::Instant::now();
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self) {
        tracing::info!("Shutting down.");

        self.events.shutdown();

        let logical_elapsed = self.shutdown_tag.unwrap().offset();
        tracing::info!("---- Elapsed logical time: {logical_elapsed}",);
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = std::time::Instant::now() - self.start_time;
        tracing::info!("---- Elapsed physical time: {physical_elapsed:?}");

        tracing::info!(stats = %self.stats, "Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event_async(&mut self) -> Option<AsyncEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(std::time::Instant::now()) {
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
    #[tracing::instrument(skip(self, current_tag), fields(tag = %current_tag))]
    fn release_tag_downstream(&self, current_tag: Tag) {
        for (key, ctx) in self.downstream_enclaves.iter() {
            let event = AsyncEvent::release(self.key, current_tag);
            tracing::trace!(downstream = %key, event = %event, "Releasing downstream");
            if !ctx.schedule_external(event) && self.shutdown_tag.is_none() {
                tracing::warn!(
                    "Failed to send tag downstream, downstream has unexpectedly terminated."
                );
            }
        }
    }

    #[tracing::instrument(skip(self), fields(tag = %self.current_tag))]
    pub fn next(&mut self) -> bool {
        // Pump the event queue
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            self.handle_async_event(async_event);
        }

        if let Some(next_tag) = self.events.peek_tag() {
            tracing::trace!(next_tag = %next_tag, "Trying next tag");

            // Wait until all upstream barriers are released
            for (_upstream_enclave_key, barrier) in self.upstream_enclaves.iter_mut() {
                if let Some(async_event) = barrier.acquire_tag(next_tag, self.key, &self.event_rx) {
                    self.handle_async_event(async_event);
                    // Returned early due to async event
                    return true;
                }
            }

            if !self.config.fast_forward {
                let target = next_tag.to_logical_time(self.start_time);
                if self.synchronize_wall_clock(target) {
                    // Woken up by async event
                    return true;
                }
            }

            //if let Some(mut event) = self.events.pop_next_event() {
            let mut event = self.events.pop_next_event().unwrap();

            tracing::debug!(event = %event, "Processing");

            if event.terminal {
                // Signal to any waiting threads that the scheduler is shutting down.
                self.shutdown_tx.shutdown();
            }

            self.process_tag(event.tag, event.reactions.view());

            // Return the ReactionSet to the free pool
            self.events.free_reaction_sets.push(event.reactions);

            self.current_tag = event.tag;

            // Release the current tag to downstream reactors
            self.release_tag_downstream(self.current_tag);

            self.stats.increment_processed_tags();

            if event.terminal {
                // Break out of the event loop;
                self.shutdown_tag = Some(self.current_tag);
                return false;
            }
        } else if let Some(async_event) = self.receive_event_async() {
            self.handle_async_event(async_event);
        } else {
            tracing::debug!("No more events in queue. -> Terminate!");
            // Shutdown event will be processed at the next event loop iteration
            self.shutdown_tag = Some(self.current_tag);
            self.events.push_event(
                self.current_tag,
                self.reaction_graph.shutdown_reactions.iter().copied(),
                true,
            );
        }

        true
    }

    #[tracing::instrument(skip(self))]
    pub fn event_loop(&mut self) {
        self.startup();

        while self.next() {}

        self.shutdown();
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self, target))]
    fn synchronize_wall_clock(&mut self, target: std::time::Instant) -> bool {
        let now = std::time::Instant::now();

        match now.cmp(&target) {
            std::cmp::Ordering::Less => {
                let advance = target - now;
                tracing::trace!(advance = ?advance, "Need to sleep");

                match self.event_rx.recv_timeout(advance) {
                    Ok(event) => {
                        tracing::debug!(event = %event, "Sleep interrupted by");
                        self.handle_async_event(event);
                        return true;
                    }
                    Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
                        let remaining = target.checked_duration_since(std::time::Instant::now());
                        if let Some(remaining) = remaining {
                            tracing::debug!(remaining = ?remaining,
                                "Sleep interrupted disconnect, sleeping for remaining",
                            );
                            std::thread::sleep(remaining);
                        }
                    }
                    Err(ReceiveErrorTimeout::Timeout) => {}
                }
            }

            std::cmp::Ordering::Greater => {
                let delay = now - target;
                tracing::warn!(delay = ?delay, "running late");
            }

            std::cmp::Ordering::Equal => {}
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

            self.stats
                .increment_processed_reactions(reaction_keys.len());

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
                self.stats
                    .increment_scheduled_actions(trigger_res.scheduled_actions.len());
                for &(action_key, tag) in trigger_res.scheduled_actions.iter() {
                    let downstream = self.reaction_graph.action_triggers[action_key]
                        .iter()
                        .copied();
                    self.events.push_event(tag, downstream, false);
                }
            }

            // Collect all the reactions that are triggered by the ports
            let downstream = self.store.iter_set_port_keys().flat_map(|port_key| {
                self.stats.increment_set_ports();
                self.reaction_graph.port_triggers[port_key].iter()
            });

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

/// Execute the given enclaves with the provided configuration.
///
/// This function will create a new `Scheduler` thread for each enclave and run its event loop.
///
/// # Arguments
///
/// * `enclaves` - An iterator over the enclaves to be executed.
/// * `config` - The configuration to be used for the schedulers.
///
/// # Returns
///
/// A vector of `Env` instances, one for each executed enclave.
///
/// # Panics
///
/// Panics if there is an error during the execution of any enclave.
pub fn execute_enclaves(
    #[allow(unused_mut)] mut enclaves: impl Iterator<Item = (EnclaveKey, Enclave)> + Send,
    config: Config,
) -> tinymap::TinySecondaryMap<EnclaveKey, Env> {
    let handles: Vec<_> = enclaves
        .filter_map(move |(enclave_key, enclave)| {
            if enclave.env.reactions.is_empty() {
                // If there are no reactions, there is nothing to do
                tracing::info!("No reactions to execute for enclave {enclave_key:?}");
                None
            } else {
                tracing::info!("Starting scheduler for enclave {enclave_key:?}");
                Some(Scheduler::new(enclave_key, enclave, config.clone()))
            }
        })
        .map(|mut sched| {
            std::thread::Builder::new()
                .name(sched.key.to_string())
                .spawn(move || {
                    sched.event_loop();
                    (sched.key, sched.into_env())
                })
                .unwrap()
        })
        .collect();

    handles
        .into_iter()
        .map(|handle| handle.join().expect("Thread panicked"))
        .collect()
}
