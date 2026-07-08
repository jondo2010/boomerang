use std::pin::Pin;

use kanal::ReceiveErrorTimeout;

mod barrier;
mod modal;
mod queue;

use barrier::LogicalTimeBarrier;
use modal::EventManager;

use crate::{
    build_reaction_contexts,
    env::{Enclave, EnclaveKey},
    event::AsyncEvent,
    keepalive,
    key_set::KeySetView,
    store::Store,
    CommonContext, Duration, Env, ModeTransitionRequest, ReactionGraph, ReactionKey,
    ReactionSetLimits, ReactorKey, SendContext, Tag,
};

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
    /// Event queues for root-scope and mode-local events.
    events: EventManager,
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
    /// Reusable buffer for reaction keys to avoid allocations in hot loops
    reaction_buffer: Vec<ReactionKey>,
    /// Reusable buffer for mode transitions to avoid allocations in hot loops
    transition_buffer: Vec<(ReactorKey, ModeTransitionRequest)>,
    /// Whether this graph contains any modes and needs modal scope checks in the hot path.
    has_modes: bool,
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
        let reaction_capacity = env.reactions.len();

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
        // Build contexts for each reaction
        let contexts = build_reaction_contexts(key, &graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);
        let has_modes = !graph.modes.is_empty();
        let events = EventManager::new(reaction_set_limits, &graph, &store);

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
            reaction_buffer: Vec::with_capacity(reaction_capacity),
            transition_buffer: Vec::with_capacity(reaction_capacity),
            has_modes,
        }
    }

    /// Handle an asynchronous event from the event queue
    #[tracing::instrument(skip(self, ), fields(event = %event))]
    fn handle_async_event(&mut self, event: AsyncEvent) {
        self.stats.increment_processed_events();
        tracing::trace!("Handling");
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
                // TagReleaseProvisional events are coming from downstream enclaves.
                // If this enclave is also an upstream (cycle), then also release it provisionally.
                if let Some(barrier) = self.upstream_enclaves.get_mut(enclave) {
                    barrier.release_tag_provisional(tag);
                }
                self.events.push_event(tag, std::iter::empty(), false);
            }
            AsyncEvent::Logical { tag, key, value } => {
                if tag <= self.current_tag {
                    tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    return;
                }
                let downstream = self.reaction_graph.action_triggers[key].iter().copied();
                self.store.push_action_value(key, tag, value);
                self.events
                    .push_action_event(key, tag, downstream, false, &self.reaction_graph);
            }
            AsyncEvent::Physical { time, key, value } => {
                let tag = Tag::from_physical_time(self.start_time, time);
                let downstream = self.reaction_graph.action_triggers[key].iter().copied();
                self.store.push_action_value(key, tag, value);
                self.events
                    .push_action_event(key, tag, downstream, false, &self.reaction_graph);
            }
            AsyncEvent::Shutdown { delay } => {
                let tag = self.current_tag.delay(delay);
                self.schedule_shutdown_at(tag);
            }
        }
    }

    fn schedule_shutdown_at(&mut self, tag: Tag) {
        let shutdown_reactions = &self
            .reaction_graph
            .modal_schedule_index
            .all_shutdown_reactions;

        for &action_key in &self
            .reaction_graph
            .modal_schedule_index
            .all_shutdown_actions_unique
        {
            self.store.push_action_value(action_key, tag, Box::new(()));
        }

        self.events.push_event(
            tag,
            shutdown_reactions.iter().map(|reaction| reaction.reaction),
            true,
        );
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    pub fn startup(&mut self) {
        let tag = Tag::ZERO;

        // Initialize the event queue with the startup actions
        for &(action_key, tag) in &self.reaction_graph.startup_actions {
            self.store.push_action_value(action_key, tag, Box::new(()));
            let downstream = self.reaction_graph.action_triggers[action_key]
                .iter()
                .inspect(|(lvl, reaction_key)| {
                    tracing::trace!(level = %lvl, reaction = %reaction_key, tag = %tag, "Startup reaction");
                })
                .copied();
            self.events
                .push_action_event(action_key, tag, downstream, false, &self.reaction_graph);
        }

        // Schedule a shutdown event if a timeout is set
        if let Some(timeout) = self.config.timeout {
            let tag = tag.delay(timeout);
            tracing::info!(tag = %tag, "Timeout set, scheduling shutdown");
            self.schedule_shutdown_at(tag);
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

        tracing::info!(stats = ?self.stats, "Scheduler has been shut down.");
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
    #[allow(clippy::should_implement_trait)]
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

            let mut event = self.events.pop_next_event().unwrap();

            tracing::debug!(event = ?event, "Processing");

            if event.terminal {
                // Signal to any waiting threads that the scheduler is shutting down.
                self.shutdown_tx.shutdown();
            }

            self.process_tag(event.tag, event.reactions.view(), event.terminal);

            self.current_tag = event.tag;

            // Return the ReactionSet to the free pool
            self.events.return_reaction_set(event.reactions);

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
            tracing::debug!("No more events in queue, pushing a shutdown event.");
            // Shutdown event will be processed at the next event loop iteration
            let shutdown = self.current_tag.delay(Duration::ZERO);
            self.shutdown_tag = Some(shutdown);
            self.schedule_shutdown_at(shutdown);
        }

        true
    }

    #[tracing::instrument(skip(self), fields(key = %self.key))]
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
    pub fn process_tag(
        &mut self,
        tag: Tag,
        reaction_view: KeySetView<ReactionKey>,
        terminal: bool,
    ) {
        self.transition_buffer.clear();
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            tracing::trace!(level=?level, "Iter");

            self.reaction_buffer.clear();
            if self.has_modes {
                for reaction_key in reaction_keys {
                    if self.reaction_is_enabled_at_current_tag(reaction_key, terminal) {
                        self.reaction_buffer.push(reaction_key);
                    }
                }
            } else {
                self.reaction_buffer.extend(reaction_keys);
            }

            self.stats
                .increment_processed_reactions(self.reaction_buffer.len());

            // Safety: reaction_keys in the same level are guaranteed to be independent of each other.
            let iter_ctx = unsafe {
                self.store
                    .iter_borrow_storage(self.reaction_buffer.iter().copied())
            }
            .enumerate();

            #[cfg(feature = "parallel")]
            use rayon::prelude::ParallelIterator;

            #[cfg(feature = "parallel")]
            let iter_ctx = rayon::prelude::ParallelBridge::par_bridge(iter_ctx);

            let iter_ctx_res = iter_ctx.map(|(idx, trigger_ctx)| (idx, trigger_ctx.trigger(tag)));

            #[cfg(feature = "parallel")]
            let iter_ctx_res = iter_ctx_res.collect::<Vec<_>>();

            let mut pending_shutdown_tag = None;
            for (idx, trigger_res) in iter_ctx_res {
                let reaction_key = self.reaction_buffer[idx];
                let reactor_key = self.reaction_graph.reaction_reactors[reaction_key];
                if let Some(request) = &trigger_res.scheduled_mode {
                    if let Some((_, existing)) = self
                        .transition_buffer
                        .iter_mut()
                        .find(|(existing_reactor, _)| *existing_reactor == reactor_key)
                    {
                        *existing = request.clone();
                    } else {
                        self.transition_buffer.push((reactor_key, request.clone()));
                    }
                }

                if let Some(shutdown_tag) = trigger_res.scheduled_shutdown {
                    // if the new shutdown tag is earlier than the current shutdown tag, update the shutdown tag and
                    // schedule a shutdown event
                    if self.shutdown_tag.map(|t| shutdown_tag < t).unwrap_or(true) {
                        self.shutdown_tag = Some(shutdown_tag);
                        pending_shutdown_tag = Some(shutdown_tag);
                    }
                }

                // Submit events to the event queue for all scheduled actions
                self.stats
                    .increment_scheduled_actions(trigger_res.scheduled_actions.len());
                for &(action_key, tag) in trigger_res.scheduled_actions.iter() {
                    let downstream = self.reaction_graph.action_triggers[action_key]
                        .iter()
                        .copied();
                    self.events.push_action_event(
                        action_key,
                        tag,
                        downstream,
                        false,
                        &self.reaction_graph,
                    );
                }
            }

            if let Some(shutdown_tag) = pending_shutdown_tag {
                self.schedule_shutdown_at(shutdown_tag);
            }

            // Collect all the reactions that are triggered by the ports
            if let Some(mut next_levels) = next_levels {
                let reaction_graph = &self.reaction_graph;
                let events = &self.events;
                let has_modes = self.has_modes;

                for port_key in self.store.iter_set_port_keys() {
                    self.stats.increment_set_ports();
                    let downstream = reaction_graph.port_triggers[port_key].iter().copied();
                    if has_modes {
                        next_levels.extend_above(downstream.filter(|&(_, reaction_key)| {
                            let scope_key = reaction_graph.reaction_scopes[reaction_key];
                            events.scope_active(scope_key)
                        }));
                    } else {
                        next_levels.extend_above(downstream);
                    }
                }
            }
        });

        if self.transition_buffer.is_empty() {
            self.store.reset_ports();
            return;
        }

        for idx in 0..self.transition_buffer.len() {
            let (reactor_key, request) = self.transition_buffer[idx].clone();
            self.events.apply_transition(
                reactor_key,
                &request,
                &mut self.store,
                &self.reaction_graph,
                tag,
            );
        }
        self.transition_buffer.clear();

        self.store.reset_ports();
    }

    fn reaction_is_enabled_at_current_tag(
        &self,
        reaction_key: ReactionKey,
        terminal: bool,
    ) -> bool {
        debug_assert!(self.has_modes);

        let scope_key = self.reaction_graph.reaction_scopes[reaction_key];
        let shutdown_lifecycle = terminal && self.reaction_graph.is_shutdown_reaction(reaction_key);
        if shutdown_lifecycle {
            return self.events.scope_ever_active(scope_key);
        }

        if !self.events.scope_active(scope_key) {
            return false;
        }

        debug_assert!(
            self.reaction_graph.reaction_modes[reaction_key]
                .as_ref()
                .is_none_or(|filter| {
                    self.reaction_graph.scopes[scope_key]
                        .mode
                        .is_some_and(|mode| {
                            let modes = filter.modes();
                            modes.len() == 1 && modes[0] == mode
                        })
                }),
            "reaction mode filters are expected to be equivalent to the static reaction scope"
        );

        true
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
