use std::{convert::Infallible, pin::Pin};

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
    ActionKey, CommonContext, Duration, Env, Level, ModeKey, PortKey, ReactionGraph, ReactionKey,
    ReactionSetLimits, ReactorData, ReactorKey, RuntimeError, ScopeKey, SendContext, Tag,
    TransitionKind, TriggerRes,
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

/// Immutable scheduler tables addressed by one exact family of dense key types.
trait ScheduleAccess {
    /// Action identity used by the schedule.
    type Action: tinymap::Key + Copy + std::fmt::Debug;
    /// Port identity used by the schedule.
    type Port: tinymap::Key + Copy + std::fmt::Debug;
    /// Reaction identity used by the schedule.
    type Reaction: tinymap::Key + Copy + std::fmt::Debug;
    /// Reactor identity used by the schedule.
    type Reactor: tinymap::Key + Copy + std::fmt::Debug;
    /// Mode identity used by the schedule.
    type Mode: tinymap::Key + Copy + std::fmt::Debug;
    /// Scope identity used by the schedule.
    type Scope: tinymap::Key + Copy + std::fmt::Debug;

    fn reaction_limits(&self) -> ReactionSetLimits;
    fn has_modes(&self) -> bool;
    fn action_from_runtime(&self, key: ActionKey) -> Self::Action;
    fn startup_action_count(&self) -> usize;
    fn startup_action(&self, index: usize) -> (Self::Action, Tag);
    fn shutdown_action_count(&self) -> usize;
    fn shutdown_action(&self, index: usize) -> Self::Action;
    fn shutdown_reactions(&self) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn action_triggers(
        &self,
        action: Self::Action,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn port_triggers(&self, port: Self::Port)
        -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn reaction_reactor(&self, reaction: Self::Reaction) -> Self::Reactor;
    fn reaction_scope(&self, reaction: Self::Reaction) -> Self::Scope;
    /// Whether the enabled-mode filter is absent or exactly matches the static reaction scope.
    ///
    /// A compiled schedule adapter must validate this equality before execution. Supporting a
    /// broader filter instead requires genuine enabled-mode filtering in the scheduler.
    fn reaction_mode_filter_matches_scope(&self, reaction: Self::Reaction) -> bool;
    fn is_shutdown_reaction(&self, reaction: Self::Reaction) -> bool;
    fn scopes(&self) -> impl Iterator<Item = Self::Scope> + '_;
    fn reactor_initial_modes(
        &self,
    ) -> impl Iterator<Item = (Self::Reactor, Option<Self::Mode>)> + '_;
    fn mode_scope(&self, mode: Self::Mode) -> Self::Scope;
    fn action_scope(&self, action: Self::Action) -> Self::Scope;
    fn action_is_logical(&self, action: Self::Action) -> bool;
    fn scope_parent(&self, scope: Self::Scope) -> Option<Self::Scope>;
    fn scope_reactor(&self, scope: Self::Scope) -> Self::Reactor;
    fn scope_mode(&self, scope: Self::Scope) -> Option<Self::Mode>;
    fn scope_descendants(&self, scope: Self::Scope) -> impl Iterator<Item = Self::Scope> + '_;
    fn scope_logical_action_count(&self, scope: Self::Scope) -> usize;
    fn scope_logical_action(&self, scope: Self::Scope, index: usize) -> Self::Action;
    fn scope_timer_startup_count(&self, scope: Self::Scope) -> usize;
    fn scope_timer_startup(&self, scope: Self::Scope, index: usize) -> (Self::Action, Tag);
    fn scope_reset_reactions(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn scope_startup_count(&self, scope: Self::Scope) -> usize;
    fn scope_startup(
        &self,
        scope: Self::Scope,
        index: usize,
    ) -> (Self::Action, (Level, Self::Reaction));
    fn reactor_root_scopes(&self) -> impl Iterator<Item = (Self::Reactor, Self::Scope)> + '_;
    fn reactor_initial_mode(&self, reactor: Self::Reactor) -> Option<Self::Mode>;
}

/// One normalized reaction result retained in reusable scheduler scratch.
#[derive(Debug)]
struct ReactionOutcome<A, M> {
    /// Typed actions scheduled by the reaction.
    scheduled_actions: Vec<(A, Tag)>,
    /// Earliest shutdown requested by the reaction, if any.
    scheduled_shutdown: Option<Tag>,
    /// Modal transition requested by the reaction, if any.
    scheduled_mode: Option<ModeTransition<M>>,
}

impl<A, M> Default for ReactionOutcome<A, M> {
    fn default() -> Self {
        Self {
            scheduled_actions: Vec::new(),
            scheduled_shutdown: None,
            scheduled_mode: None,
        }
    }
}

/// A mode transition normalized to the schedule's exact mode key.
#[derive(Debug, Clone)]
struct ModeTransition<M> {
    /// Target mode.
    target: M,
    /// Reset or history transition semantics.
    transition: TransitionKind,
}

/// Mutable execution storage consumed by the scheduler independently of its schedule.
trait ExecutionStorage<S: ScheduleAccess> {
    /// Failure returned while invoking reactions.
    type Error;

    /// Retain an action value until its scheduled tag is processed.
    fn push_action_value(&mut self, action: S::Action, tag: Tag, value: Box<dyn ReactorData>);
    /// Remove all pending values for an action during a modal reset.
    fn clear_action_values(&mut self, action: S::Action);
    /// Move a retained action value when a modal scope resumes.
    fn reschedule_action_value(&mut self, action: S::Action, from: Tag, to: Tag);
    /// Execute one independent reaction level into reusable scheduler outcomes.
    fn execute_reactions(
        &mut self,
        reactions: &[S::Reaction],
        tag: Tag,
        outcomes: &mut [ReactionOutcome<S::Action, S::Mode>],
    ) -> Result<(), Self::Error>;
    /// Copy currently set port keys into reusable scheduler scratch.
    fn collect_set_ports(&self, ports: &mut Vec<S::Port>);
    /// Clear transient port presence after a tag.
    fn reset_ports(&mut self);
}

impl ExecutionStorage<ReactionGraph> for Pin<Box<Store>> {
    type Error = Infallible;

    fn push_action_value(&mut self, action: ActionKey, tag: Tag, value: Box<dyn ReactorData>) {
        Store::push_action_value(self, action, tag, value);
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

    fn collect_set_ports(&self, ports: &mut Vec<PortKey>) {
        ports.clear();
        ports.extend(Store::iter_set_port_keys(self));
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

impl ScheduleAccess for ReactionGraph {
    type Action = ActionKey;
    type Port = PortKey;
    type Reaction = ReactionKey;
    type Reactor = ReactorKey;
    type Mode = ModeKey;
    type Scope = ScopeKey;

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

    fn has_modes(&self) -> bool {
        !self.modes.is_empty()
    }

    fn action_from_runtime(&self, key: ActionKey) -> Self::Action {
        key
    }

    fn startup_action_count(&self) -> usize {
        self.startup_actions.len()
    }

    fn startup_action(&self, index: usize) -> (Self::Action, Tag) {
        self.startup_actions[index]
    }

    fn shutdown_action_count(&self) -> usize {
        self.modal_schedule_index.all_shutdown_actions_unique.len()
    }

    fn shutdown_action(&self, index: usize) -> Self::Action {
        self.modal_schedule_index.all_shutdown_actions_unique[index]
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

    fn reaction_reactor(&self, reaction: Self::Reaction) -> Self::Reactor {
        self.reaction_reactors[reaction]
    }

    fn reaction_scope(&self, reaction: Self::Reaction) -> Self::Scope {
        self.reaction_scopes[reaction]
    }

    fn reaction_mode_filter_matches_scope(&self, reaction: Self::Reaction) -> bool {
        let scope = self.reaction_scopes[reaction];
        self.reaction_modes[reaction].as_ref().is_none_or(|filter| {
            self.scopes[scope].mode.is_some_and(|mode| {
                let modes = filter.modes();
                modes.len() == 1 && modes[0] == mode
            })
        })
    }

    fn is_shutdown_reaction(&self, reaction: Self::Reaction) -> bool {
        ReactionGraph::is_shutdown_reaction(self, reaction)
    }

    fn scopes(&self) -> impl Iterator<Item = Self::Scope> + '_ {
        self.scopes.keys()
    }

    fn reactor_initial_modes(
        &self,
    ) -> impl Iterator<Item = (Self::Reactor, Option<Self::Mode>)> + '_ {
        self.reactor_initial_modes
            .iter()
            .map(|(reactor, mode)| (reactor, *mode))
    }

    fn mode_scope(&self, mode: Self::Mode) -> Self::Scope {
        self.mode_scopes[mode]
    }

    fn action_scope(&self, action: Self::Action) -> Self::Scope {
        self.action_scopes[action]
    }

    fn action_is_logical(&self, action: Self::Action) -> bool {
        self.action_is_logical[action]
    }

    fn scope_parent(&self, scope: Self::Scope) -> Option<Self::Scope> {
        self.scopes[scope].parent
    }

    fn scope_reactor(&self, scope: Self::Scope) -> Self::Reactor {
        self.scopes[scope].reactor
    }

    fn scope_mode(&self, scope: Self::Scope) -> Option<Self::Mode> {
        self.scopes[scope].mode
    }

    fn scope_descendants(&self, scope: Self::Scope) -> impl Iterator<Item = Self::Scope> + '_ {
        self.modal_schedule_index
            .scope_descendants(scope)
            .iter()
            .copied()
    }

    fn scope_logical_action_count(&self, scope: Self::Scope) -> usize {
        self.modal_schedule_index.scope_logical_actions(scope).len()
    }

    fn scope_logical_action(&self, scope: Self::Scope, index: usize) -> Self::Action {
        self.modal_schedule_index.scope_logical_actions(scope)[index]
    }

    fn scope_timer_startup_count(&self, scope: Self::Scope) -> usize {
        self.modal_schedule_index.scope_timer_startups(scope).len()
    }

    fn scope_timer_startup(&self, scope: Self::Scope, index: usize) -> (Self::Action, Tag) {
        self.modal_schedule_index.scope_timer_startups(scope)[index]
    }

    fn scope_reset_reactions(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ {
        self.modal_schedule_index
            .scope_reset_reactions(scope)
            .iter()
            .copied()
    }

    fn scope_startup_count(&self, scope: Self::Scope) -> usize {
        self.modal_schedule_index
            .scope_startup_reactions(scope)
            .len()
    }

    fn scope_startup(
        &self,
        scope: Self::Scope,
        index: usize,
    ) -> (Self::Action, (Level, Self::Reaction)) {
        let reaction = self.modal_schedule_index.scope_startup_reactions(scope)[index];
        (reaction.action, reaction.reaction)
    }

    fn reactor_root_scopes(&self) -> impl Iterator<Item = (Self::Reactor, Self::Scope)> + '_ {
        self.reactor_root_scopes
            .iter()
            .map(|(reactor, scope)| (reactor, *scope))
    }

    fn reactor_initial_mode(&self, reactor: Self::Reactor) -> Option<Self::Mode> {
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
    /// Reusable set-port keys populated after each reaction level.
    port_buffer: Vec<PortKey>,
    /// Whether this graph contains any modes and needs modal scope checks in the hot path.
    has_modes: bool,
}

/// One scheduling algorithm borrowing separate immutable schedule and mutable storage concerns.
///
/// Coordination, clocks, wake reception, and shutdown remain concrete here; this
/// core is not a backend and performs no lowering.
struct SchedulerCore<'a, S, E>
where
    S: ScheduleAccess,
    E: ExecutionStorage<S>,
{
    /// Enclave whose logical time this invocation advances.
    key: EnclaveKey,
    /// Existing live scheduler configuration.
    config: &'a Config,
    /// Immutable dependency and modal schedule tables.
    schedule: &'a S,
    /// Mutable reaction, action, and port execution storage.
    storage: &'a mut E,
    /// Existing asynchronous wake receiver.
    event_rx: &'a crate::Receiver<AsyncEvent>,
    /// Root and modal event queues typed by the schedule keys.
    events: &'a mut EventManager<S>,
    /// Physical origin used to translate logical tags.
    start_time: &'a mut std::time::Instant,
    /// Most recently completed logical tag.
    current_tag: &'a mut Tag,
    /// Earliest scheduled shutdown tag, if any.
    shutdown_tag: &'a mut Option<Tag>,
    /// Existing keepalive sender used to interrupt live reaction contexts.
    shutdown_tx: &'a keepalive::Sender,
    /// Existing local upstream time barriers.
    upstream_enclaves: &'a mut tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    /// Existing local downstream wake senders.
    downstream_enclaves: &'a tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    /// Existing feature-gated federated time barrier.
    #[cfg(feature = "federated")]
    federated_time_barrier: &'a mut Box<dyn FederatedTimeBarrier>,
    /// Accumulated runtime statistics.
    stats: &'a mut Stats,
    /// Reusable enabled-reaction scratch.
    reaction_buffer: &'a mut Vec<S::Reaction>,
    /// Reusable modal-transition scratch.
    transition_buffer: &'a mut Vec<(S::Reactor, ModeTransition<S::Mode>)>,
    /// Reusable normalized reaction outcomes.
    outcomes: &'a mut Vec<ReactionOutcome<S::Action, S::Mode>>,
    /// Reusable keys for ports set by the current reaction level.
    port_buffer: &'a mut Vec<S::Port>,
    /// Whether modal scope checks are required in the hot path.
    has_modes: bool,
}

/// Failure from concrete time coordination or mutable execution storage.
#[derive(Debug)]
enum SchedulerCoreError<E> {
    /// Existing local or federated logical-time coordination failed.
    Runtime(RuntimeError),
    /// A reaction invocation in the execution storage failed.
    Execution(E),
}

impl<S, E> SchedulerCore<'_, S, E>
where
    S: ScheduleAccess,
    E: ExecutionStorage<S>,
{
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
                if tag <= *self.current_tag {
                    if tag < *self.current_tag {
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
                if tag <= *self.current_tag {
                    tracing::warn!(tag = %tag, "Ignoring empty event in the past");
                    return;
                }
                let key = self.schedule.action_from_runtime(key);
                self.storage.push_action_value(key, tag, value);
                self.events.push_action_event(
                    key,
                    tag,
                    self.schedule.action_triggers(key),
                    false,
                    self.schedule,
                );
            }
            AsyncEvent::Physical { time, key, value } => {
                let tag = Tag::from_physical_time(*self.start_time, time);
                let key = self.schedule.action_from_runtime(key);
                self.storage.push_action_value(key, tag, value);
                self.events.push_action_event(
                    key,
                    tag,
                    self.schedule.action_triggers(key),
                    false,
                    self.schedule,
                );
            }
            AsyncEvent::Shutdown { delay } => {
                let tag = self.current_tag.delay(delay);
                self.schedule_shutdown_at(tag);
            }
        }
    }

    fn schedule_shutdown_at(&mut self, tag: Tag) {
        for index in 0..self.schedule.shutdown_action_count() {
            let action = self.schedule.shutdown_action(index);
            self.storage.push_action_value(action, tag, Box::new(()));
        }

        self.events
            .push_event(tag, self.schedule.shutdown_reactions(), true);
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    fn startup(&mut self) {
        let tag = Tag::ZERO;

        // Initialize the event queue with the startup actions
        for index in 0..self.schedule.startup_action_count() {
            let (action_key, tag) = self.schedule.startup_action(index);
            self.storage
                .push_action_value(action_key, tag, Box::new(()));
            let downstream = self.schedule.action_triggers(action_key).inspect(|(lvl, reaction_key)| {
                    tracing::trace!(level = %lvl, reaction = ?reaction_key, tag = %tag, "Startup reaction");
                });
            self.events
                .push_action_event(action_key, tag, downstream, false, self.schedule);
        }

        // Schedule a shutdown event if a timeout is set
        if let Some(timeout) = self.config.timeout {
            let tag = tag.delay(timeout);
            tracing::info!(tag = %tag, "Timeout set, scheduling shutdown");
            self.schedule_shutdown_at(tag);
        }

        tracing::info!(tag = %tag, "Starting the execution.");

        *self.current_tag = tag.decrement();

        // Release the current tag to downstream reactors
        self.release_tag_downstream(*self.current_tag);

        *self.start_time = std::time::Instant::now();
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(skip(self))]
    fn shutdown(&mut self) {
        tracing::info!("Shutting down.");

        self.events.shutdown();

        let logical_elapsed = (*self.shutdown_tag).unwrap().offset();
        tracing::info!("---- Elapsed logical time: {logical_elapsed}",);
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = std::time::Instant::now() - *self.start_time;
        tracing::info!("---- Elapsed physical time: {physical_elapsed:?}");

        tracing::info!(stats = ?self.stats, "Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event_async(&mut self) -> Option<AsyncEvent> {
        if let Some(shutdown) = *self.shutdown_tag {
            let abs = shutdown.to_logical_time(*self.start_time);
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
        self.federated_time_barrier.acquire_tag(tag, self.event_rx)
    }

    #[cfg(feature = "federated")]
    fn federated_logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError> {
        self.federated_time_barrier.logical_tag_complete(tag)
    }

    /// Process one scheduler step, returning coordination failures to the caller.
    #[tracing::instrument(skip(self), fields(tag = %self.current_tag))]
    fn try_next(&mut self) -> Result<bool, SchedulerCoreError<E::Error>> {
        // Pump the event queue
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            self.handle_async_event(async_event);
        }

        if let Some(next_tag) = self.events.peek_tag() {
            tracing::trace!(next_tag = %next_tag, "Trying next tag");

            // Wait until all upstream barriers are released
            for (_upstream_enclave_key, barrier) in self.upstream_enclaves.iter_mut() {
                if let Some(async_event) = barrier
                    .acquire_tag(next_tag, self.key, self.event_rx)
                    .map_err(RuntimeError::from)
                    .map_err(SchedulerCoreError::Runtime)?
                {
                    self.handle_async_event(async_event);
                    // Returned early due to async event
                    return Ok(true);
                }
            }

            #[cfg(feature = "federated")]
            {
                match self
                    .acquire_federated_tag(next_tag)
                    .map_err(RuntimeError::from)
                    .map_err(SchedulerCoreError::Runtime)?
                {
                    FederatedBarrierOutcome::Granted => {}
                    FederatedBarrierOutcome::Interrupted(async_event) => {
                        self.handle_async_event(async_event);
                        // Returned early due to async event
                        return Ok(true);
                    }
                }
            }

            if !self.config.fast_forward {
                let target = next_tag.to_logical_time(*self.start_time);
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

            self.process_tag(event.tag, event.reactions.view(), event.terminal)
                .map_err(SchedulerCoreError::Execution)?;

            *self.current_tag = event.tag;

            // Return the ReactionSet to the free pool
            self.events.return_reaction_set(event.reactions);

            // Release the current tag to downstream reactors
            self.release_tag_downstream(*self.current_tag);
            #[cfg(feature = "federated")]
            self.federated_logical_tag_complete(*self.current_tag)
                .map_err(RuntimeError::from)
                .map_err(SchedulerCoreError::Runtime)?;

            self.stats.increment_processed_tags();

            if event.terminal {
                // Break out of the event loop;
                *self.shutdown_tag = Some(*self.current_tag);
                return Ok(false);
            }
        } else if let Some(async_event) = self.receive_event_async() {
            self.handle_async_event(async_event);
        } else {
            tracing::debug!("No more events in queue, pushing a shutdown event.");
            // Shutdown event will be processed at the next event loop iteration
            let shutdown = (*self.current_tag).delay(Duration::ZERO);
            *self.shutdown_tag = Some(shutdown);
            self.schedule_shutdown_at(shutdown);
        }

        Ok(true)
    }

    /// Run until shutdown or return the first runtime coordination failure.
    #[tracing::instrument(skip(self), fields(key = %self.key))]
    fn try_event_loop(&mut self) -> Result<(), SchedulerCoreError<E::Error>> {
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
    fn process_tag(
        &mut self,
        tag: Tag,
        reaction_view: KeySetView<S::Reaction>,
        terminal: bool,
    ) -> Result<(), E::Error> {
        self.transition_buffer.clear();
        let mut execution_error = None;
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            if execution_error.is_some() {
                return;
            }
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

            let outcome_count = self.reaction_buffer.len();
            if let Err(error) = self.storage.execute_reactions(
                self.reaction_buffer,
                tag,
                &mut self.outcomes[..outcome_count],
            ) {
                execution_error = Some(error);
                return;
            }

            let mut pending_shutdown_tag = None;
            for (idx, outcome) in self.outcomes[..outcome_count].iter().enumerate() {
                let reaction_key = self.reaction_buffer[idx];
                let reactor_key = self.schedule.reaction_reactor(reaction_key);
                if let Some(request) = &outcome.scheduled_mode {
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

                if let Some(shutdown_tag) = outcome.scheduled_shutdown {
                    // if the new shutdown tag is earlier than the current shutdown tag, update the shutdown tag and
                    // schedule a shutdown event
                    if (*self.shutdown_tag)
                        .map(|t| shutdown_tag < t)
                        .unwrap_or(true)
                    {
                        *self.shutdown_tag = Some(shutdown_tag);
                        pending_shutdown_tag = Some(shutdown_tag);
                    }
                }

                // Submit events to the event queue for all scheduled actions
                self.stats
                    .increment_scheduled_actions(outcome.scheduled_actions.len());
                for &(action_key, tag) in &outcome.scheduled_actions {
                    self.events.push_action_event(
                        action_key,
                        tag,
                        self.schedule.action_triggers(action_key),
                        false,
                        self.schedule,
                    );
                }
            }

            if let Some(shutdown_tag) = pending_shutdown_tag {
                self.schedule_shutdown_at(shutdown_tag);
            }

            // Collect all the reactions that are triggered by the ports
            if let Some(mut next_levels) = next_levels {
                let events = &self.events;
                let has_modes = self.has_modes;
                self.storage.collect_set_ports(self.port_buffer);

                for &port_key in self.port_buffer.iter() {
                    self.stats.increment_set_ports();
                    let downstream = self.schedule.port_triggers(port_key);
                    if has_modes {
                        next_levels.extend_above(downstream.filter(|&(_, reaction_key)| {
                            let scope_key = self.schedule.reaction_scope(reaction_key);
                            events.scope_active(scope_key)
                        }));
                    } else {
                        next_levels.extend_above(downstream);
                    }
                }
            }
        });

        if let Some(error) = execution_error {
            return Err(error);
        }

        if self.transition_buffer.is_empty() {
            self.storage.reset_ports();
            return Ok(());
        }

        for idx in 0..self.transition_buffer.len() {
            let (reactor_key, request) = self.transition_buffer[idx].clone();
            self.events
                .apply_transition(reactor_key, &request, self.schedule, self.storage, tag);
        }
        self.transition_buffer.clear();

        self.storage.reset_ports();
        Ok(())
    }

    fn reaction_is_enabled_at_current_tag(
        &self,
        reaction_key: S::Reaction,
        terminal: bool,
    ) -> bool {
        debug_assert!(self.has_modes);

        let scope_key = self.schedule.reaction_scope(reaction_key);
        let shutdown_lifecycle = terminal && self.schedule.is_shutdown_reaction(reaction_key);
        if shutdown_lifecycle {
            return self.events.scope_ever_active(scope_key);
        }

        if !self.events.scope_active(scope_key) {
            return false;
        }

        debug_assert!(
            self.schedule
                .reaction_mode_filter_matches_scope(reaction_key),
            "reaction mode filters are expected to be equivalent to the static reaction scope"
        );

        true
    }
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
        let port_capacity = env.ports.len();
        let reaction_set_limits = graph.reaction_limits();
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
            outcomes: (0..reaction_capacity).map(|_| Default::default()).collect(),
            port_buffer: Vec::with_capacity(port_capacity),
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
            port_buffer,
            has_modes,
        } = self;

        SchedulerCore {
            key: *key,
            config,
            schedule: reaction_graph,
            storage: store,
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
            port_buffer,
            has_modes: *has_modes,
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
        match self.core().process_tag(tag, reaction_view, terminal) {
            Ok(()) => {}
            Err(error) => match error {},
        }
    }

    /// Consume the scheduler and return the `Env` instance.
    ///
    /// This method is useful for testing purposes, as it allows the caller to inspect reactor states after the
    /// scheduler has been run.
    pub fn into_env(self) -> Env {
        self.store.into_env()
    }
}

/// Removes the impossible live-storage error while preserving coordination failures.
fn live_scheduler_result<T>(
    result: Result<T, SchedulerCoreError<Infallible>>,
) -> Result<T, RuntimeError> {
    match result {
        Ok(value) => Ok(value),
        Err(SchedulerCoreError::Runtime(error)) => Err(error),
        Err(SchedulerCoreError::Execution(error)) => match error {},
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
    use crate::{reaction_closure, ActionKey, Level, PortKey, Reaction, Reactor};

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
