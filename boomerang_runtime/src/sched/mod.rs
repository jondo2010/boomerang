use std::pin::Pin;

use kanal::ReceiveErrorTimeout;

mod barrier;
mod modal;
mod queue;

use barrier::LogicalTimeBarrier;
pub use barrier::LogicalTimeBarrierError;
#[cfg(feature = "federated")]
use barrier::NoFederatedTimeBarrier;
#[cfg(feature = "federated")]
pub use barrier::{FederatedBarrierError, FederatedBarrierOutcome, FederatedTimeBarrier};
use modal::EventManager;

use crate::{
    build_reaction_contexts,
    env::{Enclave, EnclaveKey},
    event::AsyncEvent,
    keepalive,
    key_set::KeySetView,
    store::Store,
    CommonContext, Duration, Env, ModeTransitionRequest, ReactionGraph, ReactionKey,
    ReactionSetLimits, ReactorKey, RuntimeError, SendContext, Tag,
};

/// Failure while starting or running a set of local enclave schedulers.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteEnclavesError {
    #[error("failed to spawn scheduler thread for enclave {enclave}: {source}")]
    ThreadSpawn {
        enclave: EnclaveKey,
        #[source]
        source: std::io::Error,
    },

    #[error("scheduler for enclave {enclave} failed: {source}")]
    Scheduler {
        enclave: EnclaveKey,
        #[source]
        source: RuntimeError,
    },

    #[error("scheduler thread for enclave {enclave} panicked: {what}")]
    ThreadPanic { enclave: EnclaveKey, what: String },
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
    /// Federated logical-time coordination hook
    #[cfg(feature = "federated")]
    federated_time_barrier: Box<dyn FederatedTimeBarrier>,
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
        let events = EventManager::new(reaction_set_limits, &graph);

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
            #[cfg(feature = "federated")]
            federated_time_barrier: Box::new(NoFederatedTimeBarrier),
            stats: Stats::default(),
            reaction_buffer: Vec::with_capacity(reaction_capacity),
            transition_buffer: Vec::with_capacity(reaction_capacity),
            has_modes,
        }
    }

    /// Create a new Scheduler instance with a federated time barrier.
    ///
    /// This constructor is the opt-in path for federated time coordination.
    /// [`Scheduler::new`] and [`execute_enclaves`] keep the local-only behavior.
    #[cfg(feature = "federated")]
    pub fn new_with_federated_time_barrier(
        key: EnclaveKey,
        enclave: Enclave,
        config: Config,
        federated_time_barrier: impl FederatedTimeBarrier + 'static,
    ) -> Self {
        let mut scheduler = Self::new(key, enclave, config);
        scheduler.federated_time_barrier = Box::new(federated_time_barrier);
        scheduler
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

    #[cfg(feature = "federated")]
    fn acquire_federated_tag(
        &mut self,
        tag: Tag,
    ) -> Result<FederatedBarrierOutcome, FederatedBarrierError> {
        self.federated_time_barrier.acquire_tag(tag, &self.event_rx)
    }

    #[cfg(feature = "federated")]
    fn federated_logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError> {
        self.federated_time_barrier.logical_tag_complete(tag)
    }

    /// Process one scheduler step, returning coordination failures to the caller.
    #[tracing::instrument(skip(self), fields(tag = %self.current_tag))]
    pub fn try_next(&mut self) -> Result<bool, RuntimeError> {
        // Pump the event queue
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            self.handle_async_event(async_event);
        }

        if let Some(next_tag) = self.events.peek_tag() {
            tracing::trace!(next_tag = %next_tag, "Trying next tag");

            // Wait until all upstream barriers are released
            for (_upstream_enclave_key, barrier) in self.upstream_enclaves.iter_mut() {
                if let Some(async_event) =
                    barrier.acquire_tag(next_tag, self.key, &self.event_rx)?
                {
                    self.handle_async_event(async_event);
                    // Returned early due to async event
                    return Ok(true);
                }
            }

            #[cfg(feature = "federated")]
            {
                match self.acquire_federated_tag(next_tag)? {
                    FederatedBarrierOutcome::Granted => {}
                    FederatedBarrierOutcome::Interrupted(async_event) => {
                        self.handle_async_event(async_event);
                        // Returned early due to async event
                        return Ok(true);
                    }
                }
            }

            if !self.config.fast_forward {
                let target = next_tag.to_logical_time(self.start_time);
                if self.synchronize_wall_clock(target) {
                    // Woken up by async event
                    return Ok(true);
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
            #[cfg(feature = "federated")]
            self.federated_logical_tag_complete(self.current_tag)?;

            self.stats.increment_processed_tags();

            if event.terminal {
                // Break out of the event loop;
                self.shutdown_tag = Some(self.current_tag);
                return Ok(false);
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

        Ok(true)
    }

    /// Run until shutdown or return the first runtime coordination failure.
    #[tracing::instrument(skip(self), fields(key = %self.key))]
    pub fn try_event_loop(&mut self) -> Result<(), RuntimeError> {
        self.startup();

        loop {
            match self.try_next() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    self.shutdown_tx.shutdown();
                    self.events.shutdown();
                    return Err(error);
                }
            }
        }

        self.shutdown();
        Ok(())
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
/// A map of `Env` instances, one for each executed enclave.
///
/// # Errors
///
/// Returns a typed thread-spawn, scheduler-runtime, or thread-panic error. Runtime and panic
/// failures are reported after every successfully spawned scheduler thread has terminated.
pub fn execute_enclaves(
    enclaves: impl Iterator<Item = (EnclaveKey, Enclave)> + Send,
    config: Config,
) -> Result<tinymap::TinySecondaryMap<EnclaveKey, Env>, ExecuteEnclavesError> {
    let schedulers = enclaves.filter_map(move |(enclave_key, enclave)| {
        if enclave.env.reactions.is_empty() {
            // If there are no reactions, there is nothing to do
            tracing::info!("No reactions to execute for enclave {enclave_key:?}");
            None
        } else {
            tracing::info!("Starting scheduler for enclave {enclave_key:?}");
            Some(Scheduler::new(enclave_key, enclave, config.clone()))
        }
    });

    let mut handles = Vec::new();
    for mut sched in schedulers {
        let enclave = sched.key;
        let handle = std::thread::Builder::new()
            .name(sched.key.to_string())
            .spawn(move || {
                let result = sched.try_event_loop();
                (sched.key, sched.into_env(), result)
            })
            .map_err(|source| ExecuteEnclavesError::ThreadSpawn { enclave, source })?;
        handles.push((enclave, handle));
    }

    let mut envs = tinymap::TinySecondaryMap::new();
    let mut first_error = None;

    for (enclave, handle) in handles {
        match handle.join() {
            Ok((key, env, Ok(()))) => {
                envs.insert(key, env);
            }
            Ok((key, _env, Err(source))) => {
                first_error.get_or_insert(ExecuteEnclavesError::Scheduler {
                    enclave: key,
                    source,
                });
            }
            Err(payload) => {
                first_error.get_or_insert(ExecuteEnclavesError::ThreadPanic {
                    enclave,
                    what: panic_payload_message(payload),
                });
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(envs),
    }
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => "non-string panic payload".to_owned(),
        },
    }
}

#[cfg(test)]
mod local_tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{reaction_closure, ActionKey, Level, PortKey, Reaction, Reactor};

    #[test]
    fn scheduler_without_external_coordinator_processes_and_completes_tag() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut enclave = Enclave::default();
        let reactor = enclave.insert_reactor(Reactor::new("root", ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let reaction_log = Arc::clone(&log);
        let reaction = enclave.insert_reaction(
            Reaction::new(
                "record",
                reaction_closure!(ctx, _reactor, _refs => {
                    reaction_log.lock().unwrap().push(ctx.get_tag());
                }),
                None,
            ),
            reactor,
            std::iter::empty::<PortKey>(),
            std::iter::empty::<PortKey>(),
            std::iter::empty::<ActionKey>(),
            scope,
            None,
        );
        let mut scheduler = Scheduler::new(
            EnclaveKey::from(0),
            enclave,
            Config::default().with_fast_forward(true),
        );
        let tag = Tag::ZERO;

        scheduler.startup();
        scheduler
            .events
            .push_event(tag, std::iter::once((Level::from(0), reaction)), false);

        assert!(scheduler.try_next().unwrap());
        assert_eq!(scheduler.current_tag, tag);
        assert_eq!(*log.lock().unwrap(), vec![tag]);
    }
}

#[cfg(all(test, feature = "federated"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{
        env::{DownstreamRef, UpstreamRef},
        reaction_closure, ActionKey, Level, PortKey, Reaction, Reactor,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum HookCall {
        LocalAcquire(Tag),
        ExternalAcquire(Tag),
        Reaction(Tag),
        LocalComplete(Tag),
        ExternalComplete(Tag),
    }

    #[derive(Debug)]
    struct RecordingBarrier {
        log: Arc<Mutex<Vec<HookCall>>>,
        interrupt: Option<AsyncEvent>,
        acquire_error: Option<String>,
        completion_error: Option<String>,
    }

    impl RecordingBarrier {
        fn granting(log: Arc<Mutex<Vec<HookCall>>>) -> Self {
            Self {
                log,
                interrupt: None,
                acquire_error: None,
                completion_error: None,
            }
        }

        fn interrupting(log: Arc<Mutex<Vec<HookCall>>>, event: AsyncEvent) -> Self {
            Self {
                log,
                interrupt: Some(event),
                acquire_error: None,
                completion_error: None,
            }
        }

        fn failing_acquire(log: Arc<Mutex<Vec<HookCall>>>, message: &str) -> Self {
            Self {
                log,
                interrupt: None,
                acquire_error: Some(message.into()),
                completion_error: None,
            }
        }

        fn failing_completion(log: Arc<Mutex<Vec<HookCall>>>, message: &str) -> Self {
            Self {
                log,
                interrupt: None,
                acquire_error: None,
                completion_error: Some(message.into()),
            }
        }
    }

    impl FederatedTimeBarrier for RecordingBarrier {
        fn acquire_tag(
            &mut self,
            tag: Tag,
            _event_rx: &crate::Receiver<AsyncEvent>,
        ) -> Result<FederatedBarrierOutcome, FederatedBarrierError> {
            self.log
                .lock()
                .unwrap()
                .push(HookCall::ExternalAcquire(tag));
            if let Some(message) = self.acquire_error.take() {
                return Err(FederatedBarrierError::new(message));
            }
            Ok(match self.interrupt.take() {
                Some(event) => FederatedBarrierOutcome::Interrupted(event),
                None => FederatedBarrierOutcome::Granted,
            })
        }

        fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError> {
            self.log
                .lock()
                .unwrap()
                .push(HookCall::ExternalComplete(tag));
            if let Some(message) = self.completion_error.take() {
                return Err(FederatedBarrierError::new(message));
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct RecordingBarrierWithLocalRelease {
        log: Arc<Mutex<Vec<HookCall>>>,
        downstream_rx: crate::Receiver<AsyncEvent>,
    }

    impl FederatedTimeBarrier for RecordingBarrierWithLocalRelease {
        fn acquire_tag(
            &mut self,
            tag: Tag,
            _event_rx: &crate::Receiver<AsyncEvent>,
        ) -> Result<FederatedBarrierOutcome, FederatedBarrierError> {
            self.log
                .lock()
                .unwrap()
                .push(HookCall::ExternalAcquire(tag));
            Ok(FederatedBarrierOutcome::Granted)
        }

        fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError> {
            loop {
                let event = self
                    .downstream_rx
                    .recv()
                    .expect("local downstream release channel should remain open");
                if matches!(event, AsyncEvent::TagRelease { tag: released, .. } if released == tag)
                {
                    break;
                }
            }
            let mut log = self.log.lock().unwrap();
            log.push(HookCall::LocalComplete(tag));
            log.push(HookCall::ExternalComplete(tag));
            Ok(())
        }
    }

    fn scheduler_with_recording_reaction(
        log: Arc<Mutex<Vec<HookCall>>>,
        barrier: impl FederatedTimeBarrier + 'static,
    ) -> (Scheduler, ReactionKey) {
        let mut enclave = Enclave::default();
        let reactor = enclave.insert_reactor(Reactor::new("root", ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let reaction_log = Arc::clone(&log);
        let reaction = enclave.insert_reaction(
            Reaction::new(
                "record",
                reaction_closure!(ctx, _reactor, _refs => {
                    reaction_log
                        .lock()
                        .unwrap()
                        .push(HookCall::Reaction(ctx.get_tag()));
                }),
                None,
            ),
            reactor,
            std::iter::empty::<PortKey>(),
            std::iter::empty::<PortKey>(),
            std::iter::empty::<ActionKey>(),
            scope,
            None,
        );
        let scheduler = Scheduler::new_with_federated_time_barrier(
            EnclaveKey::from(0),
            enclave,
            Config::default().with_fast_forward(true),
            barrier,
        );
        (scheduler, reaction)
    }

    fn scheduler_with_local_dependencies(
        log: Arc<Mutex<Vec<HookCall>>>,
    ) -> (
        Scheduler,
        ReactionKey,
        crate::Receiver<AsyncEvent>,
        crate::Sender<AsyncEvent>,
        (crate::keepalive::Sender, crate::keepalive::Sender),
    ) {
        let mut enclave = Enclave::default();
        let event_tx = enclave.event_tx.clone();
        let reactor = enclave.insert_reactor(Reactor::new("root", ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let reaction_log = Arc::clone(&log);
        let reaction = enclave.insert_reaction(
            Reaction::new(
                "record",
                reaction_closure!(ctx, _reactor, _refs => {
                    reaction_log
                        .lock()
                        .unwrap()
                        .push(HookCall::Reaction(ctx.get_tag()));
                }),
                None,
            ),
            reactor,
            std::iter::empty::<PortKey>(),
            std::iter::empty::<PortKey>(),
            std::iter::empty::<ActionKey>(),
            scope,
            None,
        );

        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (upstream_shutdown_tx, upstream_shutdown_rx) = crate::keepalive::channel();
        enclave.upstream_enclaves.insert(
            upstream,
            UpstreamRef {
                send_ctx: SendContext {
                    enclave_key: upstream,
                    async_tx: upstream_tx,
                    shutdown_rx: upstream_shutdown_rx,
                },
                delay: None,
            },
        );

        let downstream = EnclaveKey::from(2);
        let (downstream_tx, downstream_rx) = kanal::unbounded();
        let (downstream_shutdown_tx, downstream_shutdown_rx) = crate::keepalive::channel();
        enclave.downstream_enclaves.insert(
            downstream,
            DownstreamRef {
                send_ctx: SendContext {
                    enclave_key: downstream,
                    async_tx: downstream_tx,
                    shutdown_rx: downstream_shutdown_rx,
                },
            },
        );

        let barrier = RecordingBarrierWithLocalRelease { log, downstream_rx };
        let scheduler = Scheduler::new_with_federated_time_barrier(
            EnclaveKey::from(0),
            enclave,
            Config::default().with_fast_forward(true),
            barrier,
        );
        (
            scheduler,
            reaction,
            upstream_rx,
            event_tx,
            (upstream_shutdown_tx, downstream_shutdown_tx),
        )
    }

    #[test]
    fn federated_time_barrier_wraps_processed_logical_tag() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let barrier = RecordingBarrier::granting(Arc::clone(&log));
        let (mut scheduler, reaction) =
            scheduler_with_recording_reaction(Arc::clone(&log), barrier);
        let tag = Tag::ZERO;

        scheduler.startup();
        scheduler
            .events
            .push_event(tag, std::iter::once((Level::from(0), reaction)), false);

        assert!(scheduler.try_next().unwrap());
        assert_eq!(scheduler.current_tag, tag);

        let calls = log.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                HookCall::ExternalAcquire(tag),
                HookCall::Reaction(tag),
                HookCall::ExternalComplete(tag)
            ]
        );
    }

    #[test]
    fn local_coordination_wraps_external_coordination() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (mut scheduler, reaction, upstream_rx, event_tx, _shutdown_guards) =
            scheduler_with_local_dependencies(Arc::clone(&log));
        let tag = Tag::ZERO;

        scheduler.startup();
        scheduler
            .events
            .push_event(tag, std::iter::once((Level::from(0), reaction)), false);

        let acquire_log = Arc::clone(&log);
        let release = std::thread::spawn(move || {
            let request = upstream_rx
                .recv()
                .expect("local upstream should receive a provisional request");
            assert!(matches!(
                request,
                AsyncEvent::TagReleaseProvisional {
                    enclave,
                    tag: requested,
                } if enclave == EnclaveKey::from(0) && requested == tag
            ));
            acquire_log
                .lock()
                .unwrap()
                .push(HookCall::LocalAcquire(tag));
            event_tx
                .send(AsyncEvent::release(EnclaveKey::from(1), tag))
                .unwrap();
        });

        assert!(scheduler.try_next().unwrap());
        release.join().unwrap();
        assert!(scheduler.try_next().unwrap());

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                HookCall::LocalAcquire(tag),
                HookCall::ExternalAcquire(tag),
                HookCall::Reaction(tag),
                HookCall::LocalComplete(tag),
                HookCall::ExternalComplete(tag),
            ]
        );
    }

    #[test]
    fn local_interruption_prevents_external_acquire() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (mut scheduler, _reaction, upstream_rx, event_tx, _shutdown_guards) =
            scheduler_with_local_dependencies(Arc::clone(&log));
        let tag = Tag::new(Duration::seconds(1), 0);

        scheduler.startup();
        scheduler
            .events
            .push_event(tag, std::iter::empty::<(Level, ReactionKey)>(), false);

        let acquire_log = Arc::clone(&log);
        let interrupt = std::thread::spawn(move || {
            let request = upstream_rx
                .recv()
                .expect("local upstream should receive a provisional request");
            assert!(matches!(
                request,
                AsyncEvent::TagReleaseProvisional { tag: requested, .. } if requested == tag
            ));
            acquire_log
                .lock()
                .unwrap()
                .push(HookCall::LocalAcquire(tag));
            event_tx
                .send(AsyncEvent::provisional(EnclaveKey::from(9), Tag::ZERO))
                .unwrap();
        });

        assert!(scheduler.try_next().unwrap());
        interrupt.join().unwrap();
        assert_eq!(*log.lock().unwrap(), vec![HookCall::LocalAcquire(tag)]);
    }

    #[test]
    fn federated_time_barrier_can_interrupt_wait_with_inbound_event() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let future_tag = Tag::new(Duration::seconds(1), 0);
        let inbound_tag = Tag::ZERO;
        let barrier = RecordingBarrier::interrupting(
            Arc::clone(&log),
            AsyncEvent::TagReleaseProvisional {
                enclave: EnclaveKey::from(1),
                tag: inbound_tag,
            },
        );
        let mut scheduler = Scheduler::new_with_federated_time_barrier(
            EnclaveKey::from(0),
            Enclave::default(),
            Config::default().with_fast_forward(true),
            barrier,
        );

        scheduler.startup();
        let before_wait = scheduler.current_tag;
        scheduler.events.push_event(
            future_tag,
            std::iter::empty::<(Level, ReactionKey)>(),
            false,
        );

        assert!(scheduler.try_next().unwrap());
        assert_eq!(scheduler.current_tag, before_wait);
        assert_eq!(scheduler.events.peek_tag(), Some(inbound_tag));

        let calls = log.lock().unwrap().clone();
        assert_eq!(calls, vec![HookCall::ExternalAcquire(future_tag)]);
    }

    #[test]
    fn federated_barrier_error_prevents_reaction_execution() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let barrier = RecordingBarrier::failing_acquire(Arc::clone(&log), "denied");
        let (mut scheduler, reaction) =
            scheduler_with_recording_reaction(Arc::clone(&log), barrier);
        let tag = Tag::ZERO;

        scheduler.startup();
        let before_wait = scheduler.current_tag;
        scheduler
            .events
            .push_event(tag, std::iter::once((Level::from(0), reaction)), false);

        assert!(matches!(
            scheduler.try_next(),
            Err(RuntimeError::FederatedBarrier(_))
        ));
        assert_eq!(scheduler.current_tag, before_wait);
        assert_eq!(scheduler.events.peek_tag(), Some(tag));
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, HookCall::Reaction(_))));
    }

    #[test]
    fn federated_completion_error_is_returned() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let barrier = RecordingBarrier::failing_completion(Arc::clone(&log), "ltc failed");
        let (mut scheduler, reaction) =
            scheduler_with_recording_reaction(Arc::clone(&log), barrier);
        let tag = Tag::ZERO;

        scheduler.startup();
        scheduler
            .events
            .push_event(tag, std::iter::once((Level::from(0), reaction)), false);

        assert!(matches!(
            scheduler.try_next(),
            Err(RuntimeError::FederatedBarrier(_))
        ));
        assert_eq!(scheduler.current_tag, tag);
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, HookCall::Reaction(reaction_tag) if *reaction_tag == tag)));
    }
}
