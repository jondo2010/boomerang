//! Live and compiled scheduler composition over one key-generic execution core.
//!
//! Compiled execution injects backend-neutral Federate coordination, while the
//! live schedulers retain local Enclave coordination during migration.

use std::pin::Pin;

mod barrier;
mod compiled;
mod core;
pub(crate) mod federate;
mod modal;
mod queue;

// Kept at the scheduler-module boundary so sibling modules retain their narrow
// `super::` imports while the generic core lives in its own implementation module.
#[cfg(test)]
pub(crate) use compiled::run_owned_scheduler_with_origin;
pub(crate) use compiled::{
    run_owned_scheduler, run_owned_scheduler_with_coordination, OwnedSchedulerOutcome,
};
pub(crate) use core::{ExecutionStorage, ModeTransition, Schedule, SchedulerError};
use core::{ReactionOutcome, SchedulerCore};

use barrier::LogicalTimeBarrier;
pub use barrier::LogicalTimeBarrierError;
pub use federate::{
    CoordinationRevision, FederateAcquisition, FederateCompletion, FederateCoordinationBackend,
    FederateCoordinationError, FederatePublication,
};
use modal::EventManager;

use crate::{
    build_reaction_contexts,
    env::{Enclave, EnclaveKey},
    event::AsyncEvent,
    keepalive,
    key_set::KeySetView,
    store::Store,
    ActionKey, Duration, Env, Level, ModeKey, PortKey, ReactionGraph, ReactionKey,
    ReactionSetLimits, ReactorData, ReactorKey, RuntimeError, ScopeKey, SendContext, Tag,
    TriggerRes,
};

/// Failure while starting or running a set of local enclave schedulers.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteEnclavesError {
    /// One observation state cannot represent multiple scheduler-local sources.
    #[error(
        "one observation handle cannot be shared by {schedulers} schedulers; configure one scheduler-local source per handle"
    )]
    ObservationRequiresSingleScheduler {
        /// Number of non-empty schedulers that would share the configured handle.
        schedulers: usize,
    },

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
    /// Optional bounded observation state for this scheduler-local source.
    ///
    /// A handle may have concurrent samplers but must be attached to only one
    /// scheduler writer for its lifetime.
    pub observation: Option<crate::ObservationHandle>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fast_forward: false,
            keep_alive: false,
            physical_event_q_size: 1024,
            timeout: None,
            observation: None,
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

    /// Attaches this configuration to one scheduler-local observation source.
    ///
    /// This adds bounded atomic instrumentation but no transport, serialization,
    /// subscriber, or publication-queue work. Do not reuse the handle for another
    /// scheduler writer.
    pub fn with_observation(mut self, observation: crate::ObservationHandle) -> Self {
        self.observation = Some(observation);
        self
    }

    /// Returns the observation handle when scheduler-local sampling is enabled.
    pub const fn observation(&self) -> Option<&crate::ObservationHandle> {
        self.observation.as_ref()
    }
}

/// Scheduler-work counters accumulated by one execution or saturating aggregate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Stats {
    /// Scheduler tag-processing steps, including terminal tags.
    processed_tags: usize,
    /// Enabled reaction callbacks selected for invocation.
    processed_reactions: usize,
    /// Timing-dependent asynchronous scheduler events handled.
    processed_events: usize,
    /// Present-port observations during trigger propagation.
    set_ports: usize,
    /// Actions explicitly requested by reaction outcomes.
    scheduled_actions: usize,
}

impl Stats {
    /// Returns scheduler tag-processing steps, including terminal tags.
    pub const fn processed_tags(&self) -> usize {
        self.processed_tags
    }

    /// Returns enabled reaction callbacks selected for invocation.
    pub const fn processed_reactions(&self) -> usize {
        self.processed_reactions
    }

    /// Returns timing-dependent asynchronous scheduler events handled.
    ///
    /// This is scheduler telemetry, not a count of unique logical events.
    pub const fn processed_events(&self) -> usize {
        self.processed_events
    }

    /// Returns present-port observations during trigger propagation.
    pub const fn set_ports(&self) -> usize {
        self.set_ports
    }

    /// Returns actions explicitly requested by reaction outcomes.
    pub const fn scheduled_actions(&self) -> usize {
        self.scheduled_actions
    }

    /// Adds every scheduler counter from `other`, saturating at [`usize::MAX`].
    pub fn saturating_add_assign(&mut self, other: &Self) {
        self.processed_tags = self.processed_tags.saturating_add(other.processed_tags);
        self.processed_reactions = self
            .processed_reactions
            .saturating_add(other.processed_reactions);
        self.processed_events = self.processed_events.saturating_add(other.processed_events);
        self.set_ports = self.set_ports.saturating_add(other.set_ports);
        self.scheduled_actions = self
            .scheduled_actions
            .saturating_add(other.scheduled_actions);
    }

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

impl ExecutionStorage<ReactionGraph> for Pin<Box<Store>> {
    type Error = RuntimeError;

    fn prepare_startup_origin(&mut self, start_time: &mut std::time::Instant) {
        let origin = std::time::Instant::now();
        *start_time = origin;
        Store::initialize_reaction_context_origins(self, origin);
    }

    fn action_from_runtime(&self, key: ActionKey) -> ActionKey {
        key
    }

    fn push_action_value(&mut self, action: ActionKey, tag: Tag, value: Box<dyn ReactorData>) {
        Store::push_action_value(self, action, tag, value);
    }

    fn stage_inbound_boundary_value(
        &mut self,
        port: crate::image::PortIndex,
        _tag: Tag,
        _value: Box<dyn ReactorData>,
    ) -> Result<PortKey, Self::Error> {
        Err(RuntimeError::AsyncBoundaryPortUnsupported(port))
    }

    fn commit_boundary_ports(&mut self, _tag: Tag) -> Result<(), Self::Error> {
        Ok(())
    }

    fn clear_action_values(&mut self, action: ActionKey) {
        Store::clear_action_values(self, action);
    }

    fn reschedule_action_value(&mut self, action: ActionKey, from: Tag, to: Tag) {
        Store::reschedule_action_value(self, action, from, to);
    }

    fn execute_reactions(
        &mut self,
        reactions: &[ReactionKey],
        tag: Tag,
        outcomes: &mut [ReactionOutcome<ActionKey, ModeKey>],
    ) -> Result<(), Self::Error> {
        // SAFETY: reactions in one dependency level have disjoint mutable runtime state.
        let contexts = unsafe { Store::iter_borrow_storage(self, reactions.iter().copied()) };

        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::{ParallelBridge, ParallelIterator};
            let results = contexts
                .enumerate()
                .par_bridge()
                .map(|(index, context)| (index, context.trigger(tag)))
                .collect::<Vec<_>>();
            for (index, result) in results {
                copy_live_outcome(&mut outcomes[index], result);
            }
        }

        #[cfg(not(feature = "parallel"))]
        for (outcome, context) in outcomes.iter_mut().zip(contexts) {
            copy_live_outcome(outcome, context.trigger(tag));
        }

        Ok(())
    }

    fn set_ports(&self) -> impl Iterator<Item = PortKey> + '_ {
        Store::iter_set_port_keys(self)
    }

    fn reset_ports(&mut self) {
        Store::reset_ports(self);
    }
}

/// Copies one live trigger result into reusable key-generic scheduler scratch.
fn copy_live_outcome(outcome: &mut ReactionOutcome<ActionKey, ModeKey>, result: &TriggerRes) {
    outcome.scheduled_actions.clear();
    outcome
        .scheduled_actions
        .extend(result.scheduled_actions.iter().copied());
    outcome.scheduled_shutdown = result.scheduled_shutdown;
    outcome.scheduled_mode = result
        .scheduled_mode
        .as_ref()
        .map(|request| ModeTransition {
            target: request.target,
            transition: request.transition,
        });
}

impl Schedule for ReactionGraph {
    type Action = ActionKey;
    type Port = PortKey;
    type Reaction = ReactionKey;
    type Reactor = ReactorKey;
    type Mode = ModeKey;
    type Scope = ScopeKey;

    fn action_capacity(&self) -> usize {
        self.action_scopes.len()
    }

    fn reaction_limits(&self) -> ReactionSetLimits {
        let max_level = self
            .action_triggers
            .values()
            .chain(self.port_triggers.values())
            .flat_map(|reactions| reactions.iter().map(|(level, _)| level))
            .max()
            .copied()
            .unwrap_or_default();
        ReactionSetLimits {
            max_level,
            num_keys: self.reaction_reactors.len(),
        }
    }

    fn startup_actions(&self) -> impl Iterator<Item = (Self::Action, Tag)> + '_ {
        self.startup_actions.iter().copied()
    }

    fn shutdown_actions(&self) -> impl Iterator<Item = Self::Action> + '_ {
        self.modal_schedule_index
            .all_shutdown_actions_unique
            .iter()
            .copied()
    }

    fn shutdown_reactions(&self) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ {
        self.modal_schedule_index
            .all_shutdown_reactions
            .iter()
            .map(|reaction| reaction.reaction)
    }

    fn action_triggers(
        &self,
        action: Self::Action,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ {
        self.action_triggers[action].iter().copied()
    }

    fn port_triggers(
        &self,
        port: Self::Port,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ {
        self.port_triggers[port].iter().copied()
    }

    fn reactor_for_reaction(&self, reaction: Self::Reaction) -> Self::Reactor {
        self.reaction_reactors[reaction]
    }

    fn scope_for_reaction(&self, reaction: Self::Reaction) -> Self::Scope {
        self.reaction_scopes[reaction]
    }

    fn reaction_filter_matches_scope(&self, reaction: Self::Reaction) -> bool {
        let scope = self.reaction_scopes[reaction];
        self.reaction_modes[reaction].as_ref().is_none_or(|filter| {
            self.scopes[scope].mode.is_some_and(|mode| {
                let modes = filter.modes();
                modes.len() == 1 && modes[0] == mode
            })
        })
    }

    fn scopes(&self) -> impl Iterator<Item = Self::Scope> + '_ {
        self.scopes.keys()
    }

    fn scope_for_mode(&self, mode: Self::Mode) -> Self::Scope {
        self.mode_scopes[mode]
    }

    fn scope_for_action(&self, action: Self::Action) -> Self::Scope {
        self.action_scopes[action]
    }

    fn action_is_logical(&self, action: Self::Action) -> bool {
        self.action_is_logical[action]
    }

    fn parent_scope(&self, scope: Self::Scope) -> Option<Self::Scope> {
        self.scopes[scope].parent
    }

    fn reactor_for_scope(&self, scope: Self::Scope) -> Self::Reactor {
        self.scopes[scope].reactor
    }

    fn mode_for_scope(&self, scope: Self::Scope) -> Option<Self::Mode> {
        self.scopes[scope].mode
    }

    fn descendant_scopes(&self, scope: Self::Scope) -> impl Iterator<Item = Self::Scope> + '_ {
        self.modal_schedule_index
            .scope_descendants(scope)
            .iter()
            .copied()
    }

    fn logical_actions_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = Self::Action> + '_ {
        self.modal_schedule_index
            .scope_logical_actions(scope)
            .iter()
            .copied()
    }

    fn timer_startups_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Self::Action, Tag)> + '_ {
        self.modal_schedule_index
            .scope_timer_startups(scope)
            .iter()
            .copied()
    }

    fn reset_reactions_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ {
        self.modal_schedule_index
            .scope_reset_reactions(scope)
            .iter()
            .copied()
    }

    fn startups_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Self::Action, (Level, Self::Reaction))> + '_ {
        self.modal_schedule_index
            .scope_startup_reactions(scope)
            .iter()
            .map(|reaction| (reaction.action, reaction.reaction))
    }

    fn reactor_root_scopes(&self) -> impl Iterator<Item = (Self::Reactor, Self::Scope)> + '_ {
        self.reactor_root_scopes
            .iter()
            .map(|(reactor, scope)| (reactor, *scope))
    }

    fn initial_mode_for_reactor(&self, reactor: Self::Reactor) -> Option<Self::Mode> {
        self.reactor_initial_modes[reactor]
    }
}

/// Public live-authoring wrapper around the key-generic scheduler core.
///
/// This preserves the existing `Enclave` authoring path while its internal core
/// is prepared for compiled execution. It is not a backend and does not lower
/// live graphs into a compiled representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveSchedulerState {
    /// Startup has not run.
    NotStarted,
    /// Startup completed and steps may still be processed.
    Running,
    /// Normal or failed finalization completed.
    Terminated,
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
    events: EventManager<ReactionGraph>,
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
    transition_buffer: Vec<(ReactorKey, ModeTransition<ModeKey>)>,
    /// Reusable normalized reaction results, one slot per possible reaction.
    outcomes: Vec<ReactionOutcome<ActionKey, ModeKey>>,
    /// Whether this graph contains modal scopes that need hot-path activity checks.
    has_modal_scopes: bool,
    /// Lifecycle of the public live scheduler API.
    state: LiveSchedulerState,
}

impl Scheduler {
    /// Creates a live scheduler for `enclave` using `config`.
    ///
    /// The scheduler is initially not started. Call [`Self::startup`] before
    /// driving it with [`Self::try_next`], or use [`Self::try_event_loop`] to
    /// perform both operations. If observation is configured, its handle must
    /// not be attached to another scheduler writer.
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
        let reaction_set_limits = graph.reaction_limits();
        // Build contexts for each reaction
        let contexts = build_reaction_contexts(key, &graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);
        let has_modal_scopes = graph.has_modal_scopes();
        let events = EventManager::new(reaction_set_limits, &graph, config.observation().is_some());

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
            outcomes: (0..reaction_capacity).map(|_| Default::default()).collect(),
            has_modal_scopes,
            state: LiveSchedulerState::NotStarted,
        }
    }

    /// Borrow the live scheduler fields as the two capability concerns and concrete coordination.
    fn core(&mut self) -> SchedulerCore<'_, '_, ReactionGraph, Pin<Box<Store>>> {
        self.events
            .set_event_queue_observation_enabled(self.config.observation().is_some());
        let Self {
            key,
            config,
            store,
            reaction_graph,
            event_rx,
            events,
            start_time,
            current_tag,
            shutdown_tag,
            shutdown_tx,
            upstream_enclaves,
            downstream_enclaves,
            stats,
            reaction_buffer,
            transition_buffer,
            outcomes,
            has_modal_scopes,
            state: _,
        } = self;

        SchedulerCore {
            key: *key,
            config,
            observation: config.observation(),
            schedule: reaction_graph,
            storage: store,
            event_rx,
            federate_coordination: None,
            federate_shutdown_tag: None,
            events,
            start_time,
            current_tag,
            last_nonterminal_tag: None,
            shutdown_tag,
            shutdown_tx,
            upstream_enclaves,
            downstream_enclaves,
            stats,
            reaction_buffer,
            transition_buffer,
            outcomes,
            has_modal_scopes: *has_modal_scopes,
        }
    }

    /// Executes scheduler startup once.
    ///
    /// Calls after startup or termination are no-ops; a terminated scheduler
    /// cannot be restarted.
    pub fn startup(&mut self) {
        if self.state != LiveSchedulerState::NotStarted {
            return;
        }
        self.core().startup();
        self.state = LiveSchedulerState::Running;
    }

    /// Processes one scheduler step after [`Self::startup`].
    ///
    /// Returns `Ok(true)` while another step may be processed. `Ok(false)` means
    /// normal shutdown has been finalized. An error finalizes failed shutdown
    /// before returning it. After either terminal result, later calls return
    /// `Ok(false)` without restarting or finalizing the scheduler again.
    pub fn try_next(&mut self) -> Result<bool, RuntimeError> {
        if self.state == LiveSchedulerState::Terminated {
            return Ok(false);
        }

        let result = {
            let mut core = self.core();
            match core.try_next() {
                Ok(true) => Ok(true),
                Ok(false) => {
                    core.shutdown();
                    Ok(false)
                }
                Err(error) => {
                    core.shutdown_failed();
                    Err(error)
                }
            }
        };
        if !matches!(result, Ok(true)) {
            self.state = LiveSchedulerState::Terminated;
        }
        live_scheduler_result(result)
    }

    /// Starts the scheduler once and processes steps until shutdown.
    ///
    /// Normal and failed finalization use the same [`Self::try_next`] path as
    /// manual stepping. Calling this method after termination returns `Ok(())`.
    pub fn try_event_loop(&mut self) -> Result<(), RuntimeError> {
        self.startup();
        while self.try_next()? {
            // The live wrapper uses the same step and finalization path as
            // callers that drive the scheduler manually.
        }
        Ok(())
    }

    /// Process the reactions at this tag in increasing order of level.
    ///
    /// Reactions at a level N may trigger further reactions at levels M>N.
    pub fn process_tag(
        &mut self,
        tag: Tag,
        reaction_view: KeySetView<ReactionKey>,
        terminal: bool,
    ) {
        self.core()
            .process_tag(tag, reaction_view, terminal, &[])
            .expect("live reaction invocation is infallible");
    }

    /// Consume the scheduler and return the `Env` instance.
    ///
    /// This method is useful for testing purposes, as it allows the caller to inspect reactor states after the
    /// scheduler has been run.
    pub fn into_env(self) -> Env {
        self.store.into_env()
    }
}

/// Flattens coordination and live-storage failures into the public runtime error type.
fn live_scheduler_result<T>(
    result: Result<T, SchedulerError<RuntimeError>>,
) -> Result<T, RuntimeError> {
    match result {
        Ok(value) => Ok(value),
        Err(SchedulerError::Coordination(error)) => Err(error),
        Err(SchedulerError::FederateCoordination(_)) => {
            unreachable!("live schedulers do not install compiled Federate coordination")
        }
        Err(SchedulerError::Execution(error)) => Err(error),
        Err(SchedulerError::FederateFailureReported { source }) => {
            live_scheduler_result(Err(*source))
        }
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
/// Returns [`ExecuteEnclavesError::ObservationRequiresSingleScheduler`] when
/// observation is enabled for more than one non-empty scheduler, because one
/// handle cannot represent multiple scheduler-local sources. Otherwise returns
/// a typed thread-spawn, scheduler-runtime, or thread-panic error. Runtime and
/// panic failures are reported after every successfully spawned scheduler thread
/// has terminated.
pub fn execute_enclaves(
    enclaves: impl Iterator<Item = (EnclaveKey, Enclave)> + Send,
    config: Config,
) -> Result<tinymap::TinySecondaryMap<EnclaveKey, Env>, ExecuteEnclavesError> {
    let observation_enabled = config.observation().is_some();
    let schedulers = enclaves.filter_map(move |(enclave_key, enclave)| {
        if enclave.env.reactions.is_empty() {
            // If there are no reactions, there is nothing to do
            tracing::debug!(target: "boomerang::runtime",
                event = "runtime.scheduler.skipped", enclave = enclave_key.as_u32(),
                reason = "no_reactions",
            );
            None
        } else {
            Some(Scheduler::new(enclave_key, enclave, config.clone()))
        }
    });

    if observation_enabled {
        let schedulers = schedulers.collect::<Vec<_>>();
        // A Slice 1 snapshot is scheduler-local and intentionally carries no source
        // identity. Hosted Slice 2 configuration will collect separately identified
        // sources; sharing one state here would silently overwrite their gauges.
        if schedulers.len() > 1 {
            return Err(ExecuteEnclavesError::ObservationRequiresSingleScheduler {
                schedulers: schedulers.len(),
            });
        }
        execute_scheduler_threads(schedulers.into_iter())
    } else {
        // Preserve the original lazy path when monitoring is disabled.
        execute_scheduler_threads(schedulers)
    }
}

fn execute_scheduler_threads(
    schedulers: impl Iterator<Item = Scheduler>,
) -> Result<tinymap::TinySecondaryMap<EnclaveKey, Env>, ExecuteEnclavesError> {
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
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{image::PortIndex, reaction_closure, ActionKey, Level, PortKey, Reaction, Reactor};

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn enclave_recording_start_origin(
        seen_origin: Arc<Mutex<Option<std::time::Instant>>>,
    ) -> (Enclave, ReactionKey) {
        let mut enclave = Enclave::default();
        let reactor = enclave.insert_reactor(Reactor::new("root", ()).boxed(), None);
        let scope = enclave.root_scope(reactor);
        let reaction = enclave.insert_reaction(
            Reaction::new(
                "record-origin",
                reaction_closure!(ctx, _reactor, _refs => {
                    *seen_origin.lock().unwrap() = Some(ctx.get_start_time());
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
        (enclave, reaction)
    }

    fn scheduler_recording_start_origin(
        seen_origin: Arc<Mutex<Option<std::time::Instant>>>,
    ) -> (Scheduler, ReactionKey) {
        let (enclave, reaction) = enclave_recording_start_origin(seen_origin);
        (
            Scheduler::new(
                EnclaveKey::from(0),
                enclave,
                Config::default().with_fast_forward(true),
            ),
            reaction,
        )
    }

    #[test]
    fn live_scheduler_origin_is_captured_at_startup_and_shared_with_contexts() {
        let seen_origin = Arc::new(Mutex::new(None));
        let (mut scheduler, reaction) = scheduler_recording_start_origin(Arc::clone(&seen_origin));
        std::thread::sleep(std::time::Duration::from_millis(2));
        let startup_floor = std::time::Instant::now();

        scheduler.startup();
        scheduler.events.push_event(
            Tag::ZERO,
            std::iter::once((Level::from(0), reaction)),
            false,
        );
        assert!(scheduler.try_next().unwrap());

        assert!(scheduler.start_time >= startup_floor);
        assert_eq!(*seen_origin.lock().unwrap(), Some(scheduler.start_time));
    }

    #[test]
    fn live_scheduler_observation_attributes_callback_elapsed_time_to_reactions() {
        let seen_origin = Arc::new(Mutex::new(None));
        let (mut scheduler, reaction) = scheduler_recording_start_origin(seen_origin);
        let observation =
            std::sync::Arc::new(crate::ObservationState::new(std::time::Instant::now()));
        scheduler.config.observation = Some(observation.clone());

        scheduler.startup();
        scheduler.events.push_event(
            Tag::ZERO,
            std::iter::once((Level::from(0), reaction)),
            false,
        );
        assert!(scheduler.try_next().unwrap());

        let snapshot = observation.snapshot(std::time::Instant::now()).unwrap();
        assert!(snapshot.reaction_elapsed_ns > 0);
        assert_eq!(snapshot.processed_reactions, 1);
        assert_eq!(snapshot.processed_tags, 1);
        assert_eq!(snapshot.event_queue_occupancy, 0);
        assert_eq!(snapshot.event_queue_peak_occupancy, 1);
    }

    #[test]
    fn live_scheduler_without_observation_does_not_track_event_queue_peaks() {
        let (mut scheduler, reaction) =
            scheduler_recording_start_origin(Arc::new(Mutex::new(None)));

        scheduler.events.push_event(
            Tag::ZERO,
            std::iter::once((Level::from(0), reaction)),
            false,
        );

        assert_eq!(scheduler.events.event_queue_observation(), None);
    }

    #[test]
    fn direct_try_next_finalizes_observation_after_normal_shutdown() {
        let (mut scheduler, _) = scheduler_recording_start_origin(Arc::new(Mutex::new(None)));
        let observation =
            std::sync::Arc::new(crate::ObservationState::new(std::time::Instant::now()));
        scheduler.config.observation = Some(observation.clone());

        scheduler.startup();
        while scheduler.try_next().unwrap() {}

        let sampled_at = std::time::Instant::now();
        let snapshot = observation.snapshot(sampled_at).unwrap();
        assert_eq!(snapshot.lifecycle, crate::SchedulerLifecycle::Stopped);
        assert_eq!(snapshot.current_phase, crate::SchedulerPhase::Idle);
        assert!(!scheduler.try_next().unwrap());
        scheduler.startup();
        assert_eq!(observation.snapshot(sampled_at).unwrap(), snapshot);
    }

    #[test]
    fn execute_enclaves_rejects_one_observation_handle_for_multiple_schedulers() {
        let enclaves = (0..2).map(|key| {
            let (enclave, _) = enclave_recording_start_origin(Arc::new(Mutex::new(None)));
            (EnclaveKey::from(key), enclave)
        });
        let observation =
            std::sync::Arc::new(crate::ObservationState::new(std::time::Instant::now()));

        let error = execute_enclaves(
            enclaves,
            Config::default()
                .with_fast_forward(true)
                .with_observation(observation),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ExecuteEnclavesError::ObservationRequiresSingleScheduler { schedulers: 2 }
        ));
    }

    #[test]
    fn live_scheduler_rejects_async_boundary_ports() {
        let enclave = Enclave::default();
        let event_tx = enclave.event_tx.clone();
        let boundary = PortIndex::new(7);
        let observation =
            std::sync::Arc::new(crate::ObservationState::new(std::time::Instant::now()));
        let mut scheduler = Scheduler::new(
            EnclaveKey::from(0),
            enclave,
            Config::default()
                .with_fast_forward(true)
                .with_observation(observation.clone()),
        );
        scheduler.startup();
        event_tx
            .send(AsyncEvent::Logical {
                tag: Tag::ZERO,
                target: crate::AsyncEventTarget::BoundaryPort(boundary),
                value: Box::new(42_u32),
            })
            .unwrap();
        assert!(matches!(
            scheduler.try_next(),
            Err(RuntimeError::AsyncBoundaryPortUnsupported(key)) if key == boundary
        ));
        let sampled_at = std::time::Instant::now();
        let snapshot = observation.snapshot(sampled_at).unwrap();
        assert_eq!(snapshot.lifecycle, crate::SchedulerLifecycle::Failed);
        assert_eq!(snapshot.current_phase, crate::SchedulerPhase::Idle);
        assert!(!scheduler.try_next().unwrap());
        scheduler.startup();
        assert_eq!(observation.snapshot(sampled_at).unwrap(), snapshot);
    }

    #[test]
    fn manual_startup_followed_by_event_loop_starts_scheduler_once() {
        let captured = Captured::default();
        let make_writer = {
            let captured = captured.clone();
            move || captured.clone()
        };
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_level(false)
            .with_writer(make_writer)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let (mut scheduler, _) = scheduler_recording_start_origin(Arc::new(Mutex::new(None)));
            scheduler.startup();
            scheduler.try_event_loop().unwrap();
        });

        let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            output
                .matches("event=\"runtime.scheduler.started\"")
                .count(),
            1,
            "{output}"
        );
    }

    #[test]
    fn runtime_trace_uses_stable_scheduler_vocabulary_without_object_dumps() {
        let captured = Captured::default();
        let make_writer = {
            let captured = captured.clone();
            move || captured.clone()
        };
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_level(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(make_writer)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let (mut scheduler, _) = scheduler_recording_start_origin(Arc::new(Mutex::new(None)));
            scheduler.try_event_loop().unwrap();

            let enclave = Enclave::default();
            let event_tx = enclave.event_tx.clone();
            let mut scheduler = Scheduler::new(
                EnclaveKey::from(1),
                enclave,
                Config::default().with_fast_forward(true),
            );
            event_tx
                .send(AsyncEvent::shutdown(Duration::nanoseconds(1)))
                .unwrap();
            scheduler.try_event_loop().unwrap();
        });

        let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains(
                "boomerang::runtime: event=\"runtime.scheduler.started\" enclave=0 tag_kind=\"finite\" tag_offset_ns=0 tag_microstep=0"
            ),
            "{output}"
        );
        assert!(
            output.contains(
                "boomerang::runtime: event=\"runtime.scheduler.waiting\" enclave=0 reason=\"empty_queue\""
            ),
            "{output}"
        );
        assert!(
            output.contains(
                "boomerang::runtime: event=\"runtime.event.admitted\" enclave=1 kind=\"shutdown\" tag_kind=\"finite\" tag_offset_ns=0 tag_microstep=0"
            ),
            "{output}"
        );
        assert!(
            output.contains("event=\"runtime.scheduler.tag_processed\" enclave=0 tag_kind=\"finite\" tag_offset_ns=0 tag_microstep=0 terminal=true network_input=false"),
            "{output}"
        );
        assert!(
            output.contains("event=\"runtime.scheduler.stopped\" enclave=0 tag_kind=\"finite\" tag_offset_ns=0 tag_microstep=0 processed_tags=1 processed_reactions=0 processed_events=0"),
            "{output}"
        );
        for legacy in [
            concat!("boomerang_runtime", "::sched"),
            "try_event_loop",
            "event=Shutdown",
            "Stats {",
            "reactor=",
            "reaction=",
        ] {
            assert!(!output.contains(legacy), "found {legacy:?} in {output}");
        }
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[cfg(feature = "serde")]
    #[test]
    fn stats_serde_round_trips_numeric_counters() {
        let stats = Stats {
            processed_tags: 1,
            processed_reactions: 2,
            processed_events: 3,
            set_ports: 4,
            scheduled_actions: 5,
        };

        let expected = serde_json::json!({
            "processed_tags": 1,
            "processed_reactions": 2,
            "processed_events": 3,
            "set_ports": 4,
            "scheduled_actions": 5,
        });
        assert_eq!(serde_json::to_value(stats).unwrap(), expected);
        assert_eq!(serde_json::from_value::<Stats>(expected).unwrap(), stats);
    }

    #[test]
    fn stats_aggregation_saturates_every_counter() {
        let mut aggregate = Stats {
            processed_tags: usize::MAX,
            processed_reactions: usize::MAX,
            processed_events: usize::MAX,
            set_ports: usize::MAX,
            scheduled_actions: usize::MAX,
        };
        aggregate.saturating_add_assign(&Stats {
            processed_tags: 1,
            processed_reactions: 1,
            processed_events: 1,
            set_ports: 1,
            scheduled_actions: 1,
        });
        assert_eq!(aggregate.processed_tags(), usize::MAX);
        assert_eq!(aggregate.processed_reactions(), usize::MAX);
        assert_eq!(aggregate.processed_events(), usize::MAX);
        assert_eq!(aggregate.set_ports(), usize::MAX);
        assert_eq!(aggregate.scheduled_actions(), usize::MAX);
    }
}
