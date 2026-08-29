//! Generic scheduler capabilities and algorithm shared by live and future compiled schedules.

use kanal::ReceiveErrorTimeout;

use super::{barrier::LogicalTimeBarrier, modal::EventManager, Config, Stats};
#[cfg(feature = "federated")]
use super::{FederatedBarrierError, FederatedBarrierOutcome, FederatedTimeBarrier};
use crate::{
    event::AsyncEvent, keepalive, key_set::KeySetView, ActionKey, CommonContext, Duration,
    EnclaveKey, Level, ReactionSetLimits, ReactorData, RuntimeError, SendContext, Tag,
    TransitionKind,
};

/// Immutable scheduler tables addressed by one exact family of dense key types.
pub(crate) trait ScheduleAccess {
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
pub(crate) trait ExecutionStorage<S: ScheduleAccess> {
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

/// One scheduling algorithm borrowing separate immutable schedule and mutable storage concerns.
///
/// Coordination, clocks, wake reception, and shutdown remain concrete here; this
/// core is not a backend and performs no lowering.
pub(super) struct SchedulerCore<'a, S, E>
where
    S: ScheduleAccess,
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
    /// Root and modal event queues typed by the schedule keys.
    pub(super) events: &'a mut EventManager<S>,
    /// Physical origin used to translate logical tags.
    pub(super) start_time: &'a mut std::time::Instant,
    /// Most recently completed logical tag.
    pub(super) current_tag: &'a mut Tag,
    /// Earliest scheduled shutdown tag, if any.
    pub(super) shutdown_tag: &'a mut Option<Tag>,
    /// Existing keepalive sender used to interrupt live reaction contexts.
    pub(super) shutdown_tx: &'a keepalive::Sender,
    /// Existing local upstream time barriers.
    pub(super) upstream_enclaves: &'a mut tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    /// Existing local downstream wake senders.
    pub(super) downstream_enclaves: &'a tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    /// Existing feature-gated federated time barrier.
    #[cfg(feature = "federated")]
    pub(super) federated_time_barrier: &'a mut Box<dyn FederatedTimeBarrier>,
    /// Accumulated runtime statistics.
    pub(super) stats: &'a mut Stats,
    /// Reusable enabled-reaction scratch.
    pub(super) reaction_buffer: &'a mut Vec<S::Reaction>,
    /// Reusable modal-transition scratch.
    pub(super) transition_buffer: &'a mut Vec<(S::Reactor, ModeTransition<S::Mode>)>,
    /// Reusable normalized reaction outcomes.
    pub(super) outcomes: &'a mut Vec<ReactionOutcome<S::Action, S::Mode>>,
    /// Reusable keys for ports set by the current reaction level.
    pub(super) port_buffer: &'a mut Vec<S::Port>,
    /// Whether modal scope checks are required in the hot path.
    pub(super) has_modes: bool,
}

/// Failure from concrete time coordination or mutable execution storage.
#[derive(Debug)]
pub(super) enum SchedulerCoreError<E> {
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
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, ), fields(event = %event))]
    fn handle_async_event(&mut self, event: AsyncEvent) {
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
                    tracing::warn!(target: "boomerang_runtime::sched", tag = %tag, "Ignoring empty event in the past");
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
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self))]
    pub(super) fn startup(&mut self) {
        let tag = Tag::ZERO;

        // Initialize the event queue with the startup actions
        for index in 0..self.schedule.startup_action_count() {
            let (action_key, tag) = self.schedule.startup_action(index);
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

        *self.start_time = std::time::Instant::now();
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
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self), fields(tag = %self.current_tag))]
    pub(super) fn try_next(&mut self) -> Result<bool, SchedulerCoreError<E::Error>> {
        // Pump the event queue
        while let Ok(Some(async_event)) = self.event_rx.try_recv() {
            self.handle_async_event(async_event);
        }

        if let Some(next_tag) = self.events.peek_tag() {
            tracing::trace!(target: "boomerang_runtime::sched", next_tag = %next_tag, "Trying next tag");

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

            tracing::debug!(target: "boomerang_runtime::sched", event = ?event, "Processing");

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
            tracing::debug!(target: "boomerang_runtime::sched", "No more events in queue, pushing a shutdown event.");
            // Shutdown event will be processed at the next event loop iteration
            let shutdown = (*self.current_tag).delay(Duration::ZERO);
            *self.shutdown_tag = Some(shutdown);
            self.schedule_shutdown_at(shutdown);
        }

        Ok(true)
    }

    /// Run until shutdown or return the first runtime coordination failure.
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self), fields(key = %self.key))]
    pub(super) fn try_event_loop(&mut self) -> Result<(), SchedulerCoreError<E::Error>> {
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
    #[tracing::instrument(target = "boomerang_runtime::sched", skip(self, target))]
    fn synchronize_wall_clock(&mut self, target: std::time::Instant) -> bool {
        let now = std::time::Instant::now();

        match now.cmp(&target) {
            std::cmp::Ordering::Less => {
                let advance = target - now;
                tracing::trace!(target: "boomerang_runtime::sched", advance = ?advance, "Need to sleep");

                match self.event_rx.recv_timeout(advance) {
                    Ok(event) => {
                        tracing::debug!(target: "boomerang_runtime::sched", event = %event, "Sleep interrupted by");
                        self.handle_async_event(event);
                        return true;
                    }
                    Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
                        let remaining = target.checked_duration_since(std::time::Instant::now());
                        if let Some(remaining) = remaining {
                            tracing::debug!(target: "boomerang_runtime::sched", remaining = ?remaining,
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
                tracing::warn!(target: "boomerang_runtime::sched", delay = ?delay, "running late");
            }

            std::cmp::Ordering::Equal => {}
        }

        false
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
    ) -> Result<(), E::Error> {
        self.transition_buffer.clear();
        let mut execution_error = None;
        reaction_view.for_each_level(|level, reaction_keys, next_levels| {
            if execution_error.is_some() {
                return;
            }
            tracing::trace!(target: "boomerang_runtime::sched", level=?level, "Iter");

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
