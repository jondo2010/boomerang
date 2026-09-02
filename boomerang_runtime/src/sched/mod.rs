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
pub(crate) use compiled::{run_owned_scheduler, run_owned_scheduler_with_coordination};
pub(crate) use core::{ExecutionStorage, ModeTransition, Schedule, SchedulerError};
use core::{ReactionOutcome, SchedulerCore};

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
    ActionKey, Duration, Env, Level, ModeKey, PortKey, ReactionGraph, ReactionKey,
    ReactionSetLimits, ReactorData, ReactorKey, RuntimeError, ScopeKey, SendContext, Tag,
    TriggerRes,
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
    /// Federated logical-time coordination hook
    #[cfg(feature = "federated")]
    federated_time_barrier: Box<dyn FederatedTimeBarrier>,
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
        let reaction_set_limits = graph.reaction_limits();
        // Build contexts for each reaction
        let contexts = build_reaction_contexts(key, &graph, start_time, event_tx, shutdown_rx);

        let store = Store::new(env, contexts, &graph);
        let has_modal_scopes = graph.has_modal_scopes();
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
            outcomes: (0..reaction_capacity).map(|_| Default::default()).collect(),
            has_modal_scopes,
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

    /// Borrow the live scheduler fields as the two capability concerns and concrete coordination.
    fn core(&mut self) -> SchedulerCore<'_, ReactionGraph, Pin<Box<Store>>> {
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
            #[cfg(feature = "federated")]
            federated_time_barrier,
            stats,
            reaction_buffer,
            transition_buffer,
            outcomes,
            has_modal_scopes,
        } = self;

        SchedulerCore {
            key: *key,
            config,
            schedule: reaction_graph,
            storage: store,
            event_rx,
            quiescence: None,
            events,
            start_time,
            current_tag,
            last_nonterminal_tag: None,
            shutdown_tag,
            shutdown_tx,
            upstream_enclaves,
            downstream_enclaves,
            #[cfg(feature = "federated")]
            federated_time_barrier,
            stats,
            reaction_buffer,
            transition_buffer,
            outcomes,
            has_modal_scopes: *has_modal_scopes,
        }
    }

    /// Execute startup of the Scheduler.
    pub fn startup(&mut self) {
        self.core().startup();
    }

    /// Process one scheduler step, returning coordination failures to the caller.
    pub fn try_next(&mut self) -> Result<bool, RuntimeError> {
        live_scheduler_result(self.core().try_next())
    }

    /// Run until shutdown or return the first runtime coordination failure.
    pub fn try_event_loop(&mut self) -> Result<(), RuntimeError> {
        live_scheduler_result(self.core().try_event_loop())
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
        Err(SchedulerError::Execution(error)) => Err(error),
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

#[cfg(all(test, feature = "federated"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{image::PortIndex, reaction_closure, ActionKey, Level, PortKey, Reaction, Reactor};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum HookCall {
        Acquire(Tag),
        Reaction(Tag),
        Ltc(Tag),
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
            self.log.lock().unwrap().push(HookCall::Acquire(tag));
            if let Some(message) = self.acquire_error.take() {
                return Err(FederatedBarrierError::new(message));
            }
            Ok(match self.interrupt.take() {
                Some(event) => FederatedBarrierOutcome::Interrupted(event),
                None => FederatedBarrierOutcome::Granted,
            })
        }

        fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError> {
            self.log.lock().unwrap().push(HookCall::Ltc(tag));
            if let Some(message) = self.completion_error.take() {
                return Err(FederatedBarrierError::new(message));
            }
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

    fn scheduler_recording_start_origin(
        seen_origin: Arc<Mutex<Option<std::time::Instant>>>,
    ) -> (Scheduler, ReactionKey) {
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
                HookCall::Acquire(tag),
                HookCall::Reaction(tag),
                HookCall::Ltc(tag)
            ]
        );
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
        assert_eq!(calls, vec![HookCall::Acquire(future_tag)]);
    }

    #[test]
    fn live_scheduler_rejects_async_boundary_ports() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let future_tag = Tag::new(Duration::seconds(1), 0);
        let boundary = PortIndex::new(7);
        let barrier = RecordingBarrier::interrupting(
            Arc::clone(&log),
            AsyncEvent::Logical {
                tag: Tag::ZERO,
                target: crate::AsyncEventTarget::BoundaryPort(boundary),
                value: Box::new(42_u32),
            },
        );
        let mut scheduler = Scheduler::new_with_federated_time_barrier(
            EnclaveKey::from(0),
            Enclave::default(),
            Config::default().with_fast_forward(true),
            barrier,
        );

        scheduler.startup();
        scheduler.events.push_event(
            future_tag,
            std::iter::empty::<(Level, ReactionKey)>(),
            false,
        );

        assert!(matches!(
            scheduler.try_next(),
            Err(RuntimeError::AsyncBoundaryPortUnsupported(key)) if key == boundary
        ));
        assert_eq!(*log.lock().unwrap(), vec![HookCall::Acquire(future_tag)]);
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
