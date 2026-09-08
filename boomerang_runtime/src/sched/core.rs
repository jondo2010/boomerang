//! Generic scheduler capabilities and algorithm shared by live and future compiled schedules.

use kanal::ReceiveErrorTimeout;

use super::{
    barrier::LogicalTimeBarrier,
    federate::{
        FederateControlAuthorization, FederateCoordinationError, FederateIdleWait,
        FederateSchedulerCoordination, FederateTagAcquisition, FederateTermination,
    },
    modal::EventManager,
    Config, Stats,
};
#[cfg(feature = "federated")]
use super::{FederatedBarrierOutcome, FederatedTimeBarrier};
use crate::{
    event::{AsyncEvent, AsyncEventTarget},
    keepalive,
    key_set::KeySetView,
    ActionKey, CommonContext, Duration, EnclaveKey, Level, ReactionSetLimits, ReactorData,
    RuntimeError, SendContext, Tag, TransitionKind,
};

/// Immutable normalized schedule addressed by one exact family of dense key types.
pub(crate) trait Schedule {
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
    fn action_capacity(&self) -> usize;
    fn has_modal_scopes(&self) -> bool {
        self.scopes()
            .any(|scope| self.mode_for_scope(scope).is_some())
    }
    fn startup_actions(&self) -> impl Iterator<Item = (Self::Action, Tag)> + '_;
    fn shutdown_actions(&self) -> impl Iterator<Item = Self::Action> + '_;
    fn shutdown_reactions(&self) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn action_triggers(
        &self,
        action: Self::Action,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn port_triggers(&self, port: Self::Port)
        -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn reactor_for_reaction(&self, reaction: Self::Reaction) -> Self::Reactor;
    fn scope_for_reaction(&self, reaction: Self::Reaction) -> Self::Scope;
    /// Whether the enabled-mode filter is absent or exactly matches the static reaction scope.
    ///
    /// A compiled schedule adapter must validate this equality before execution. Supporting a
    /// broader filter instead requires genuine enabled-mode filtering in the scheduler.
    fn reaction_filter_matches_scope(&self, reaction: Self::Reaction) -> bool;
    fn is_shutdown_reaction(&self, reaction: Self::Reaction) -> bool {
        self.shutdown_reactions()
            .any(|(_, candidate)| candidate == reaction)
    }
    fn scopes(&self) -> impl Iterator<Item = Self::Scope> + '_;
    fn scope_for_mode(&self, mode: Self::Mode) -> Self::Scope;
    fn scope_for_action(&self, action: Self::Action) -> Self::Scope;
    fn action_is_logical(&self, action: Self::Action) -> bool;
    /// Returns a positive recurrence period for a scheduler-owned timer action.
    fn action_period(&self, _action: Self::Action) -> Option<Duration> {
        None
    }
    fn parent_scope(&self, scope: Self::Scope) -> Option<Self::Scope>;
    fn reactor_for_scope(&self, scope: Self::Scope) -> Self::Reactor;
    fn mode_for_scope(&self, scope: Self::Scope) -> Option<Self::Mode>;
    fn descendant_scopes(&self, scope: Self::Scope) -> impl Iterator<Item = Self::Scope> + '_;
    fn logical_actions_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = Self::Action> + '_;
    fn timer_startups_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Self::Action, Tag)> + '_;
    fn reset_reactions_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Level, Self::Reaction)> + '_;
    fn startups_in_scope(
        &self,
        scope: Self::Scope,
    ) -> impl Iterator<Item = (Self::Action, (Level, Self::Reaction))> + '_;
    fn reactor_root_scopes(&self) -> impl Iterator<Item = (Self::Reactor, Self::Scope)> + '_;
    fn initial_mode_for_reactor(&self, reactor: Self::Reactor) -> Option<Self::Mode>;
}

/// One normalized reaction result retained in reusable scheduler scratch.
#[derive(Debug)]
pub(crate) struct ReactionOutcome<A, M> {
    /// Typed actions scheduled by the reaction.
    pub(crate) scheduled_actions: Vec<(A, Tag)>,
    /// Earliest shutdown requested by the reaction, if any.
    pub(crate) scheduled_shutdown: Option<Tag>,
    /// Modal transition requested by the reaction, if any.
    pub(crate) scheduled_mode: Option<ModeTransition<M>>,
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
pub(crate) struct ModeTransition<M> {
    /// Target mode.
    pub(crate) target: M,
    /// Reset or history transition semantics.
    pub(crate) transition: TransitionKind,
}

/// Mutable execution storage consumed by the scheduler independently of its schedule.
pub(crate) trait ExecutionStorage<S: Schedule> {
    /// Failure returned while invoking reactions.
    type Error;

    /// Prepares the scheduler origin immediately before startup begins.
    fn prepare_startup_origin(&mut self, start_time: &mut std::time::Instant);
    /// Resolves an externally supplied runtime action identity to this storage's action key.
    fn action_from_runtime(&self, key: ActionKey) -> S::Action;
    /// Retain an action value until its scheduled tag is processed.
    fn push_action_value(&mut self, action: S::Action, tag: Tag, value: Box<dyn ReactorData>);
    /// Stages one inbound scheduler-boundary value until its logical tag is processed.
    fn stage_inbound_boundary_value(
        &mut self,
        port: crate::image::PortIndex,
        tag: Tag,
        value: Box<dyn ReactorData>,
    ) -> Result<S::Port, Self::Error>;
    /// Writes all retained scheduler-boundary values for one processing tag.
    fn commit_boundary_ports(&mut self, tag: Tag) -> Result<(), Self::Error>;
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
    /// Iterates over ports that are currently set.
    fn set_ports(&self) -> impl Iterator<Item = S::Port> + '_;
    /// Clear transient port presence after a tag.
    fn reset_ports(&mut self);
}

/// One scheduling algorithm borrowing separate immutable schedule and mutable storage concerns.
///
/// Coordination, clocks, wake reception, and shutdown remain concrete here; this
/// core is not a backend and performs no lowering.
pub(super) struct SchedulerCore<'a, 'coordination, S, E>
where
    S: Schedule,
    E: ExecutionStorage<S>,
{
    /// Enclave whose logical time this invocation advances.
    pub(super) key: EnclaveKey,
    /// Existing live scheduler configuration.
    pub(super) config: &'a Config,
    /// Immutable dependency and modal schedule tables.
    pub(super) schedule: &'a S,
    /// Mutable reaction, action, and port execution storage.
    pub(super) storage: &'a mut E,
    /// Existing asynchronous wake receiver.
    pub(super) event_rx: &'a crate::Receiver<AsyncEvent>,
    /// Optional backend-neutral coordination port installed only by compiled schedulers.
    pub(super) federate_coordination:
        Option<&'coordination mut (dyn FederateSchedulerCoordination + 'coordination)>,
    /// Graceful Federate terminal tag authorized to bypass unavailable local barriers.
    pub(super) federate_shutdown_tag: Option<Tag>,
    /// Root and modal event queues typed by the schedule keys.
    pub(super) events: &'a mut EventManager<S>,
    /// Physical origin used to translate logical tags.
    pub(super) start_time: &'a mut std::time::Instant,
    /// Most recently completed logical tag.
    pub(super) current_tag: &'a mut Tag,
    /// Last tag that processed non-terminal work, when an adapter needs to retain it.
    pub(super) last_nonterminal_tag: Option<&'a mut Option<Tag>>,
    /// Earliest scheduled shutdown tag, if any.
    pub(super) shutdown_tag: &'a mut Option<Tag>,
    /// Existing keepalive sender used to interrupt live reaction contexts.
    pub(super) shutdown_tx: &'a keepalive::Sender,
    /// Existing local upstream time barriers.
    pub(super) upstream_enclaves: &'a mut tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    /// Existing local downstream wake senders.
    pub(super) downstream_enclaves: &'a tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    /// Existing feature-gated federated time barrier installed only by live schedulers.
    #[cfg(feature = "federated")]
    pub(super) federated_time_barrier: Option<&'a mut dyn FederatedTimeBarrier>,
    /// Accumulated runtime statistics.
    pub(super) stats: &'a mut Stats,
    /// Reusable enabled-reaction scratch.
    pub(super) reaction_buffer: &'a mut Vec<S::Reaction>,
    /// Reusable modal-transition scratch.
    pub(super) transition_buffer: &'a mut Vec<(S::Reactor, ModeTransition<S::Mode>)>,
    /// Reusable normalized reaction outcomes.
    pub(super) outcomes: &'a mut Vec<ReactionOutcome<S::Action, S::Mode>>,
    /// Whether modal scope checks are required in the hot path.
    pub(super) has_modal_scopes: bool,
}

/// Failure from concrete time coordination or mutable execution storage.
#[derive(Debug)]
pub(crate) enum SchedulerError<E> {
    /// Existing local or federated logical-time coordination failed.
    Coordination(RuntimeError),
    /// The compiled Federate coordination port failed with a closed typed error.
    FederateCoordination(FederateCoordinationError),
    /// A reaction invocation in the execution storage failed.
    Execution(E),
    /// The compiled scheduler already emitted its one Federate failure report for this error.
    FederateFailureReported {
        /// Original typed scheduler failure retained without formatting or reparsing.
        source: Box<SchedulerError<E>>,
    },
}

/// Raw result from the scheduler event channel before an interruption is handled.
enum WallClockReceive {
    /// The requested physical deadline elapsed normally.
    DeadlineReached,
    /// A scheduler event interrupted the deadline and remains to be handled.
    Interrupted(AsyncEvent),
    /// Federate coordination terminated the scheduler through its existing event channel.
    FederateTerminated(FederateTermination),
}

/// Performs one uninterrupted scheduler-event receive until a physical deadline.
fn receive_until_wall_clock_deadline(
    target: std::time::Instant,
    event_rx: &crate::Receiver<AsyncEvent>,
    entered_receive: impl FnOnce(),
    terminal_after_close: impl FnOnce()
        -> Result<Option<FederateTermination>, FederateCoordinationError>,
) -> Result<WallClockReceive, FederateCoordinationError> {
    let advance = target.saturating_duration_since(std::time::Instant::now());
    entered_receive();
    match event_rx.recv_timeout(advance) {
        Ok(event) => Ok(WallClockReceive::Interrupted(event)),
        Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
            if let Some(termination) = terminal_after_close()? {
                return Ok(WallClockReceive::FederateTerminated(termination));
            }
            if let Some(remaining) = target.checked_duration_since(std::time::Instant::now()) {
                tracing::debug!(target: "boomerang_runtime::sched", remaining = ?remaining,
                    "Sleep interrupted disconnect, sleeping for remaining",
                );
                std::thread::sleep(remaining);
            }
            Ok(WallClockReceive::DeadlineReached)
        }
        Err(ReceiveErrorTimeout::Timeout) => Ok(WallClockReceive::DeadlineReached),
    }
}

impl<S, E> SchedulerCore<'_, '_, S, E>
where
    S: Schedule,
    E: ExecutionStorage<S>,
{
    /// Handle an asynchronous event from the event queue
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, ), fields(event = %event))]
    fn handle_async_event(&mut self, event: AsyncEvent) -> Result<(), E::Error> {
        self.stats.increment_processed_events();
        tracing::trace!(target: "boomerang_runtime::sched", "Handling");
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
                        tracing::warn!(target: "boomerang_runtime::sched", tag = %tag, "Ignoring empty event in the past");
                    }
                    return Ok(());
                }
                // TagReleaseProvisional events are coming from downstream enclaves.
                // If this enclave is also an upstream (cycle), then also release it provisionally.
                if let Some(barrier) = self.upstream_enclaves.get_mut(enclave) {
                    barrier.release_tag_provisional(tag);
                }
                self.events.push_control_event(tag);
            }
            AsyncEvent::Logical { tag, target, value } => {
                if tag <= *self.current_tag {
                    tracing::warn!(target: "boomerang_runtime::sched", tag = %tag, "Ignoring empty event in the past");
                    return Ok(());
                }
                self.admit_value(tag, target, value)?;
            }
            AsyncEvent::Physical {
                time,
                target,
                value,
            } => {
                let tag = Tag::from_physical_time(*self.start_time, time);
                self.admit_value(tag, target, value)?;
            }
            AsyncEvent::Shutdown { delay } => {
                let tag = self.current_tag.delay(delay);
                self.schedule_shutdown_at(tag);
            }
        }
        Ok(())
    }

    /// Stores one action or validated boundary-port payload and schedules its trigger set.
    fn admit_value(
        &mut self,
        tag: Tag,
        target: AsyncEventTarget,
        value: Box<dyn ReactorData>,
    ) -> Result<(), E::Error> {
        match target {
            AsyncEventTarget::Action(key) => {
                let action = self.storage.action_from_runtime(key);
                self.storage.push_action_value(action, tag, value);
                self.events.push_action_event(
                    action,
                    tag,
                    self.schedule.action_triggers(action),
                    false,
                    self.schedule,
                );
            }
            AsyncEventTarget::BoundaryPort(key) => {
                let port = self.storage.stage_inbound_boundary_value(key, tag, value)?;
                self.events
                    .push_event(tag, self.schedule.port_triggers(port), false);
            }
        }
        Ok(())
    }

    fn schedule_shutdown_at(&mut self, tag: Tag) {
        *self.shutdown_tag = Some((*self.shutdown_tag).map_or(tag, |pending| pending.min(tag)));
        for action in self.schedule.shutdown_actions() {
            self.storage.push_action_value(action, tag, Box::new(()));
        }

        self.events
            .push_event(tag, self.schedule.shutdown_reactions(), true);
    }

    /// Returns the first representable shutdown tag strictly after the current scheduler tag.
    fn next_shutdown_tag(&self) -> Tag {
        if *self.current_tag < Tag::ZERO {
            Tag::ZERO
        } else if self.current_tag.microstep() == usize::MAX {
            self.current_tag
                .checked_delay(Duration::nanoseconds(1))
                .unwrap_or(Tag::FOREVER)
        } else {
            self.current_tag.delay(Duration::ZERO)
        }
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self))]
    pub(super) fn startup(&mut self) {
        self.storage.prepare_startup_origin(self.start_time);
        let tag = Tag::ZERO;

        // Initialize the event queue with the startup actions
        for (action_key, tag) in self.schedule.startup_actions() {
            self.storage
                .push_action_value(action_key, tag, Box::new(()));
            let downstream = self.schedule.action_triggers(action_key).inspect(|(lvl, reaction_key)| {
                    tracing::trace!(target: "boomerang_runtime::sched", level = %lvl, reaction = ?reaction_key, tag = %tag, "Startup reaction");
                });
            self.events
                .push_action_event(action_key, tag, downstream, false, self.schedule);
        }

        // Schedule a shutdown event if a timeout is set
        if let Some(timeout) = self.config.timeout {
            let tag = tag.delay(timeout);
            tracing::info!(target: "boomerang_runtime::sched", tag = %tag, "Timeout set, scheduling shutdown");
            self.schedule_shutdown_at(tag);
        }

        tracing::info!(target: "boomerang_runtime::sched", tag = %tag, "Starting the execution.");

        *self.current_tag = tag.decrement();

        // Release the current tag to downstream reactors
        self.release_tag_downstream(*self.current_tag);
    }

    /// Final shutdown of the Scheduler. The last tag has already been processed.
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self))]
    fn shutdown(&mut self) {
        tracing::info!(target: "boomerang_runtime::sched", "Shutting down.");

        self.events.shutdown();

        let logical_elapsed = (*self.shutdown_tag).unwrap().offset();
        tracing::info!(target: "boomerang_runtime::sched", "---- Elapsed logical time: {logical_elapsed}",);
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = std::time::Instant::now() - *self.start_time;
        tracing::info!(target: "boomerang_runtime::sched", "---- Elapsed physical time: {physical_elapsed:?}");

        tracing::info!(target: "boomerang_runtime::sched", stats = ?self.stats, "Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self))]
    fn receive_event_async(&mut self) -> Option<AsyncEvent> {
        if let Some(shutdown) = *self.shutdown_tag {
            let abs = shutdown.to_logical_time(*self.start_time);
            if let Some(timeout) = abs.checked_duration_since(std::time::Instant::now()) {
                tracing::debug!(target: "boomerang_runtime::sched", timeout = ?timeout, "Waiting for async event.");
                self.event_rx.recv_timeout(timeout).ok()
            } else {
                tracing::debug!(target: "boomerang_runtime::sched", "Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.config.keep_alive {
            tracing::debug!(target: "boomerang_runtime::sched", "Waiting indefinitely for async event.");
            self.event_rx.recv().ok()
        } else {
            None
        }
    }

    /// Release the current tag to downstream reactors
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, current_tag), fields(tag = %current_tag))]
    fn release_tag_downstream(&self, current_tag: Tag) {
        for (key, ctx) in self.downstream_enclaves.iter() {
            let event = AsyncEvent::release(self.key, current_tag);
            tracing::trace!(target: "boomerang_runtime::sched", downstream = %key, event = %event, "Releasing downstream");
            if !ctx.schedule_external(event) && self.shutdown_tag.is_none() {
                tracing::warn!(target: "boomerang_runtime::sched",
                    "Failed to send tag downstream, downstream has unexpectedly terminated."
                );
            }
        }
    }

    /// Reports one scheduler failure and marks its ownership for the supervising layer.
    fn report_federate_failure(
        &mut self,
        error: SchedulerError<E::Error>,
    ) -> SchedulerError<E::Error> {
        if let Some(coordination) = self.federate_coordination.take() {
            coordination.fail();
            SchedulerError::FederateFailureReported {
                source: Box::new(error),
            }
        } else {
            error
        }
    }

    /// Terminates this scheduler after coordination ends without granting further work.
    fn stop_for_federate_termination(&mut self, tag: Tag) {
        self.federate_coordination.take();
        self.federate_shutdown_tag = Some(tag);
        self.shutdown_tx.shutdown();
        self.schedule_shutdown_at(tag);
    }

    /// Exits immediately after a Federate abort without running shutdown lifecycle work.
    fn abort_for_federate_termination(&mut self) {
        self.federate_coordination.take();
        self.shutdown_tx.shutdown();
        self.events.shutdown();
        if self.shutdown_tag.is_none() {
            *self.shutdown_tag = Some(self.next_shutdown_tag());
        }
    }

    /// Applies a terminal command that closed the scheduler event channel.
    fn handle_closed_event_channel(&mut self) -> Result<Option<bool>, SchedulerError<E::Error>> {
        let termination = self
            .federate_coordination
            .as_deref_mut()
            .map_or(Ok(None), |coordination| {
                coordination.terminal_after_event_channel_closed()
            })
            .map_err(SchedulerError::FederateCoordination)?;
        Ok(match termination {
            Some(FederateTermination::Graceful { tag }) => {
                let tag = tag.unwrap_or_else(|| self.next_shutdown_tag());
                self.stop_for_federate_termination(tag);
                Some(true)
            }
            Some(FederateTermination::Abort) => {
                self.abort_for_federate_termination();
                Some(false)
            }
            None => None,
        })
    }

    /// Drains pending scheduler events without blocking.
    fn pump_pending_async_events(&mut self) -> Result<(), SchedulerError<E::Error>> {
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            if matches!(
                &async_event,
                AsyncEvent::Logical { .. }
                    | AsyncEvent::Physical { .. }
                    | AsyncEvent::Shutdown { .. }
            ) {
                if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                    coordination.active();
                }
            }
            self.handle_async_event(async_event)
                .map_err(SchedulerError::Execution)?;
        }
        Ok(())
    }

    /// Coordinates permission to process the next compiled scheduler tag.
    fn coordinate_next_tag(
        &mut self,
        next_tag: Tag,
        control_only: bool,
        logical_horizon: Option<Tag>,
    ) -> Result<Option<bool>, SchedulerError<E::Error>> {
        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
            if logical_horizon == Some(next_tag) && !self.events.has_nonterminal_work() {
                match coordination
                    .wait()
                    .map_err(SchedulerError::FederateCoordination)
                {
                    Ok(FederateIdleWait::Interrupted(async_event)) => {
                        self.handle_async_event(async_event)
                            .map_err(SchedulerError::Execution)?;
                        return Ok(Some(true));
                    }
                    Ok(FederateIdleWait::LogicalHorizon(tag)) => {
                        tracing::trace!(tag = %tag, "Federate logical horizon ended coordination");
                        self.stop_for_federate_termination(tag);
                        return Ok(Some(true));
                    }
                    Ok(FederateIdleWait::Stopped) => {
                        self.stop_for_federate_termination(self.next_shutdown_tag());
                        return Ok(Some(true));
                    }
                    Ok(FederateIdleWait::Aborted) => {
                        self.abort_for_federate_termination();
                        return Ok(Some(false));
                    }
                    Err(error) => {
                        return Err(self.report_federate_failure(error));
                    }
                }
            }
            if !control_only {
                coordination.active();
            }
        }
        tracing::trace!(target: "boomerang_runtime::sched", next_tag = %next_tag, "Trying next tag");

        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
            if control_only {
                match coordination
                    .authorize_control(next_tag)
                    .map_err(SchedulerError::FederateCoordination)
                {
                    Ok(FederateControlAuthorization::Authorized) => {}
                    Ok(FederateControlAuthorization::Interrupted(async_event)) => {
                        self.handle_async_event(async_event)
                            .map_err(SchedulerError::Execution)?;
                        return Ok(Some(true));
                    }
                    Ok(FederateControlAuthorization::LogicalHorizon(tag)) => {
                        self.stop_for_federate_termination(tag);
                        return Ok(Some(true));
                    }
                    Ok(FederateControlAuthorization::Stopped) => {
                        self.stop_for_federate_termination(self.next_shutdown_tag());
                        return Ok(Some(true));
                    }
                    Ok(FederateControlAuthorization::Aborted) => {
                        self.abort_for_federate_termination();
                        return Ok(Some(false));
                    }
                    Err(error) => return Err(self.report_federate_failure(error)),
                }
            } else {
                match coordination
                    .acquire_tag(next_tag)
                    .map_err(SchedulerError::FederateCoordination)
                {
                    Ok(FederateTagAcquisition::Granted) => {}
                    Ok(FederateTagAcquisition::Interrupted(async_event)) => {
                        self.handle_async_event(async_event)
                            .map_err(SchedulerError::Execution)?;
                        return Ok(Some(true));
                    }
                    Ok(FederateTagAcquisition::LogicalHorizon(tag)) => {
                        tracing::trace!(tag = %tag, "Federate logical horizon ended acquisition");
                        self.stop_for_federate_termination(tag);
                        return Ok(Some(true));
                    }
                    Ok(FederateTagAcquisition::Stopped) => {
                        self.stop_for_federate_termination(self.next_shutdown_tag());
                        return Ok(Some(true));
                    }
                    Ok(FederateTagAcquisition::Aborted) => {
                        self.abort_for_federate_termination();
                        return Ok(Some(false));
                    }
                    Err(error) => return Err(self.report_federate_failure(error)),
                }
            }
        }
        Ok(None)
    }

    /// Waits until every upstream enclave releases the next tag.
    fn wait_for_upstream_release(
        &mut self,
        next_tag: Tag,
    ) -> Result<Option<bool>, SchedulerError<E::Error>> {
        if self.federate_shutdown_tag != Some(next_tag) {
            for (_upstream_enclave_key, barrier) in self.upstream_enclaves.iter_mut() {
                let async_event = match barrier.acquire_tag(next_tag, self.key, self.event_rx) {
                    Ok(async_event) => async_event,
                    Err(error) => {
                        if let Some(keep_running) = self.handle_closed_event_channel()? {
                            return Ok(Some(keep_running));
                        }
                        return Err(SchedulerError::Coordination(error.into()));
                    }
                };
                if let Some(async_event) = async_event {
                    if matches!(
                        &async_event,
                        AsyncEvent::Logical { .. }
                            | AsyncEvent::Physical { .. }
                            | AsyncEvent::Shutdown { .. }
                    ) {
                        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                            coordination.active();
                        }
                    }
                    self.handle_async_event(async_event)
                        .map_err(SchedulerError::Execution)?;
                    // Returned early due to async event.
                    return Ok(Some(true));
                }
            }
        }
        Ok(None)
    }

    /// Acquires the next tag from the legacy federated barrier.
    #[cfg(feature = "federated")]
    fn acquire_legacy_federated_tag(
        &mut self,
        next_tag: Tag,
    ) -> Result<Option<bool>, SchedulerError<E::Error>> {
        if self.federate_coordination.is_none() {
            if let Some(barrier) = self.federated_time_barrier.as_deref_mut() {
                match barrier
                    .acquire_tag(next_tag, self.event_rx)
                    .map_err(|error| SchedulerError::Coordination(error.into()))?
                {
                    FederatedBarrierOutcome::Granted => {}
                    FederatedBarrierOutcome::Interrupted(async_event) => {
                        self.handle_async_event(async_event)
                            .map_err(SchedulerError::Execution)?;
                        return Ok(Some(true));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Synchronizes the next tag with the scheduler wall clock.
    fn synchronize_next_tag(
        &mut self,
        next_tag: Tag,
    ) -> Result<Option<bool>, SchedulerError<E::Error>> {
        if !self.config.fast_forward {
            let target = next_tag.to_logical_time(*self.start_time);
            match self.synchronize_wall_clock(target)? {
                WallClockReceive::DeadlineReached => {}
                WallClockReceive::Interrupted(event) => {
                    tracing::debug!(target: "boomerang_runtime::sched", event = %event, "Sleep interrupted by");
                    if matches!(
                        &event,
                        AsyncEvent::Logical { .. }
                            | AsyncEvent::Physical { .. }
                            | AsyncEvent::Shutdown { .. }
                    ) {
                        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                            coordination.active();
                        }
                    }
                    self.handle_async_event(event)
                        .map_err(SchedulerError::Execution)?;
                    return Ok(Some(true));
                }
                WallClockReceive::FederateTerminated(FederateTermination::Graceful { tag }) => {
                    let tag = tag.unwrap_or_else(|| self.next_shutdown_tag());
                    self.stop_for_federate_termination(tag);
                    return Ok(Some(true));
                }
                WallClockReceive::FederateTerminated(FederateTermination::Abort) => {
                    self.abort_for_federate_termination();
                    return Ok(Some(false));
                }
            }
        }
        Ok(None)
    }

    /// Processes and completes the next queued scheduler event.
    fn process_next_event(
        &mut self,
        logical_horizon: Option<Tag>,
    ) -> Result<bool, SchedulerError<E::Error>> {
        let mut event = self.events.pop_next_event().unwrap();

        tracing::debug!(target: "boomerang_runtime::sched", event = ?event, "Processing");

        if event.terminal {
            if logical_horizon == Some(event.tag) {
                if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                    coordination.logical_horizon_reached(event.tag);
                }
                self.federate_coordination.take();
            }
            // Signal to any waiting threads that the scheduler is shutting down.
            self.shutdown_tx.shutdown();
        }

        self.storage
            .commit_boundary_ports(event.tag)
            .map_err(SchedulerError::Execution)?;

        self.process_tag(
            event.tag,
            event.reactions.view(),
            event.terminal,
            &event.action_values,
        )?;

        *self.current_tag = event.tag;
        if event.has_nonterminal_work {
            if let Some(last_nonterminal_tag) = self.last_nonterminal_tag.as_deref_mut() {
                *last_nonterminal_tag = Some(event.tag);
            }
        }

        // Return the reaction key set to the free pool.
        self.events.return_reaction_set(event.reactions);
        self.events.return_action_values(event.action_values);

        // Release the current tag to downstream reactors
        self.release_tag_downstream(*self.current_tag);
        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
            if let Err(error) = coordination
                .logical_tag_complete(*self.current_tag)
                .map_err(SchedulerError::FederateCoordination)
            {
                return Err(self.report_federate_failure(error));
            }
        } else {
            #[cfg(feature = "federated")]
            if let Some(barrier) = self.federated_time_barrier.as_deref_mut() {
                barrier
                    .logical_tag_complete(*self.current_tag)
                    .map_err(|error| SchedulerError::Coordination(error.into()))?;
            }
        }

        self.stats.increment_processed_tags();

        if event.terminal {
            if logical_horizon != Some(event.tag) {
                if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                    coordination.participant_stopped();
                }
            }
            // Break out of the event loop;
            *self.shutdown_tag = Some(*self.current_tag);
            return Ok(false);
        }
        Ok(true)
    }

    /// Waits for asynchronous work when the scheduler queue is empty.
    fn wait_for_next_event(&mut self) -> Result<bool, SchedulerError<E::Error>> {
        if let Some(coordination) = self.federate_coordination.as_deref_mut() {
            match coordination
                .wait()
                .map_err(SchedulerError::FederateCoordination)
            {
                Ok(FederateIdleWait::Interrupted(async_event)) => {
                    self.handle_async_event(async_event)
                        .map_err(SchedulerError::Execution)?;
                }
                Ok(FederateIdleWait::LogicalHorizon(tag)) => {
                    self.stop_for_federate_termination(tag);
                    return Ok(true);
                }
                Ok(FederateIdleWait::Stopped) => {
                    self.stop_for_federate_termination(self.next_shutdown_tag());
                    return Ok(true);
                }
                Ok(FederateIdleWait::Aborted) => {
                    self.abort_for_federate_termination();
                    return Ok(false);
                }
                Err(error) => return Err(self.report_federate_failure(error)),
            }
        } else if let Some(async_event) = self.receive_event_async() {
            self.handle_async_event(async_event)
                .map_err(SchedulerError::Execution)?;
        } else {
            tracing::debug!(target: "boomerang_runtime::sched", "No more events in queue, pushing a shutdown event.");
            // Shutdown event will be processed at the next event loop iteration
            let shutdown = self.next_shutdown_tag();
            *self.shutdown_tag = Some(shutdown);
            self.schedule_shutdown_at(shutdown);
        }

        Ok(true)
    }

    /// Process one scheduler step, returning coordination failures to the caller.
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self), fields(tag = %self.current_tag))]
    pub(super) fn try_next(&mut self) -> Result<bool, SchedulerError<E::Error>> {
        self.pump_pending_async_events()?;

        if self.event_rx.is_closed() {
            if let Some(keep_running) = self.handle_closed_event_channel()? {
                return Ok(keep_running);
            }
            if self.federate_coordination.is_none() && self.federate_shutdown_tag.is_none() {
                self.schedule_shutdown_at(self.next_shutdown_tag());
            }
        }

        if let Some(next_tag) = self.events.peek_tag() {
            let control_only =
                self.federate_coordination.is_some() && self.events.peek_is_control_only();
            let logical_horizon = self.config.timeout.map(|timeout| Tag::ZERO.delay(timeout));
            if let Some(keep_running) =
                self.coordinate_next_tag(next_tag, control_only, logical_horizon)?
            {
                return Ok(keep_running);
            }
            if let Some(keep_running) = self.wait_for_upstream_release(next_tag)? {
                return Ok(keep_running);
            }

            #[cfg(feature = "federated")]
            if let Some(keep_running) = self.acquire_legacy_federated_tag(next_tag)? {
                return Ok(keep_running);
            }

            if let Some(keep_running) = self.synchronize_next_tag(next_tag)? {
                return Ok(keep_running);
            }

            self.process_next_event(logical_horizon)
        } else {
            self.wait_for_next_event()
        }
    }

    /// Run until shutdown or return the first runtime coordination failure.
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self), fields(key = %self.key))]
    pub(super) fn try_event_loop(&mut self) -> Result<(), SchedulerError<E::Error>> {
        self.startup();

        loop {
            match self.try_next() {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    let error = match error {
                        error @ SchedulerError::FederateFailureReported { .. } => error,
                        error => self.report_federate_failure(error),
                    };
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
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, target))]
    fn synchronize_wall_clock(
        &mut self,
        target: std::time::Instant,
    ) -> Result<WallClockReceive, SchedulerError<E::Error>> {
        let now = std::time::Instant::now();

        match now.cmp(&target) {
            std::cmp::Ordering::Less => {
                let advance = target - now;
                tracing::trace!(target: "boomerang_runtime::sched", advance = ?advance, "Need to sleep");

                return receive_until_wall_clock_deadline(
                    target,
                    self.event_rx,
                    || {},
                    || {
                        self.federate_coordination.as_deref_mut().map_or(
                            Ok(None),
                            FederateSchedulerCoordination::terminal_after_event_channel_closed,
                        )
                    },
                )
                .map_err(SchedulerError::FederateCoordination);
            }

            std::cmp::Ordering::Greater => {
                let delay = now - target;
                tracing::warn!(target: "boomerang_runtime::sched", delay = ?delay, "running late");
            }

            std::cmp::Ordering::Equal => {}
        }

        Ok(WallClockReceive::DeadlineReached)
    }

    /// Process the reactions at this tag in increasing order of level.
    ///
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, reaction_view), fields(tag = %tag))]
    pub(super) fn process_tag(
        &mut self,
        tag: Tag,
        reaction_view: KeySetView<S::Reaction>,
        terminal: bool,
        periodic_actions: &[S::Action],
    ) -> Result<(), SchedulerError<E::Error>> {
        self.transition_buffer.clear();
        let mut execution_error = None;
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            if execution_error.is_some() {
                return;
            }
            tracing::trace!(target: "boomerang_runtime::sched", level=?level, "Iter");

            self.reaction_buffer.clear();
            if self.has_modal_scopes {
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
                let reactor_key = self.schedule.reactor_for_reaction(reaction_key);
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
                let has_modal_scopes = self.has_modal_scopes;

                for port_key in self.storage.set_ports() {
                    self.stats.increment_set_ports();
                    let downstream = self.schedule.port_triggers(port_key);
                    if has_modal_scopes {
                        next_levels.extend_above(downstream.filter(|&(_, reaction_key)| {
                            let scope_key = self.schedule.scope_for_reaction(reaction_key);
                            events.scope_active(scope_key)
                        }));
                    } else {
                        next_levels.extend_above(downstream);
                    }
                }
            }
        });

        if let Some(error) = execution_error {
            return Err(SchedulerError::Execution(error));
        }

        for &action in periodic_actions {
            let Some(period) = self.schedule.action_period(action) else {
                continue;
            };
            let successor = tag.checked_delay(period);
            if (*self.shutdown_tag)
                .is_some_and(|shutdown| successor.is_none_or(|successor| successor >= shutdown))
            {
                continue;
            }
            let successor = successor.ok_or_else(|| {
                SchedulerError::Coordination(RuntimeError::LogicalTimeOverflow { tag, period })
            })?;
            self.storage
                .push_action_value(action, successor, Box::new(()));
            self.events.push_action_event(
                action,
                successor,
                self.schedule.action_triggers(action),
                false,
                self.schedule,
            );
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
        debug_assert!(self.has_modal_scopes);

        let scope_key = self.schedule.scope_for_reaction(reaction_key);
        let shutdown_lifecycle = terminal && self.schedule.is_shutdown_reaction(reaction_key);
        if shutdown_lifecycle {
            return self.events.scope_ever_active(scope_key);
        }

        if !self.events.scope_active(scope_key) {
            return false;
        }

        debug_assert!(
            self.schedule.reaction_filter_matches_scope(reaction_key),
            "reaction mode filters are expected to be equivalent to the static reaction scope"
        );

        true
    }
}

#[cfg(test)]
mod wall_clock_tests {
    //! Exact receive-entry coverage for coordinated wall-clock termination.

    use super::{receive_until_wall_clock_deadline, WallClockReceive};
    use crate::{
        image::EnclaveIndex,
        sched::federate::{
            FederateCoordinationParts, FederateSchedulerCoordination, FederateTermination,
            LifecyclePolicy, LocalFederateCoordinationBackend,
        },
        AsyncEvent,
    };
    use std::{sync::mpsc, time::Duration as StdDuration};

    /// Verifies coordinator abort closes an entered timed receive only after queuing termination.
    #[test]
    fn coordinated_abort_interrupts_entered_wall_clock_receive() {
        let enclave = EnclaveIndex::new(0);
        let (event_tx, event_rx) = kanal::unbounded::<AsyncEvent>();
        let scheduler_event_rx = event_rx.clone();
        let FederateCoordinationParts {
            abort_handle,
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::KeepAlive,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let target = std::time::Instant::now() + StdDuration::from_secs(1);

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            let waiter = scope.spawn(move || {
                receive_until_wall_clock_deadline(
                    target,
                    &scheduler_event_rx,
                    || entered_tx.send(()).unwrap(),
                    || participant.terminal_after_event_channel_closed(),
                )
            });
            entered_rx
                .recv_timeout(StdDuration::from_millis(100))
                .expect("scheduler must enter the production timed-receive boundary");
            abort_handle.abort();

            assert!(matches!(
                waiter.join().unwrap().unwrap(),
                WallClockReceive::FederateTerminated(FederateTermination::Abort)
            ));
            coordinator.join().unwrap().unwrap();
        });
    }
}
