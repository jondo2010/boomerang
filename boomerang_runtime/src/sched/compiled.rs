//! Compiled-image scheduler adapters and owned execution composition.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc,
};

use super::{
    barrier::LogicalTimeBarrier,
    core::{
        ExecutionStorage, ReactionOutcome, Schedule, SchedulerActivity, SchedulerCore,
        SchedulerError,
    },
    modal::EventManager,
    Config, Stats,
};
#[cfg(feature = "federated")]
use super::{barrier::NoFederatedTimeBarrier, FederatedTimeBarrier};
use crate::{
    image::{
        ActionIndex, EnclaveImageView, LevelReactionImage, ModeIndex, PortIndex, ReactionIndex,
        ReactorIndex, ScopeIndex,
    },
    ActionKey, AsyncEvent, Duration, EnclaveKey, Level, OwnedStorage, OwnedStorageError,
    ReactionSetLimits, ReactorData, SendContext, Tag,
};

/// Crate-private local coordination links for one owned compiled scheduler.
pub(crate) struct OwnedSchedulerCoordination {
    /// Canonical identity of this scheduler-owned Enclave.
    key: EnclaveKey,
    /// Logical upstream Enclaves and minimum delays across parallel local routes.
    upstream: tinymap::TinySecondaryMap<EnclaveKey, (SendContext, Option<Duration>)>,
    /// Coalesced downstream Enclave contexts used for logical tag release.
    downstream: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
}

impl OwnedSchedulerCoordination {
    /// Creates an unlinked scheduler descriptor for one canonical Enclave.
    pub(crate) fn new(key: EnclaveKey) -> Self {
        Self {
            key,
            upstream: tinymap::TinySecondaryMap::new(),
            downstream: tinymap::TinySecondaryMap::new(),
        }
    }

    /// Adds one logical upstream, retaining the most restrictive delay for parallel routes.
    pub(crate) fn add_upstream(
        &mut self,
        key: EnclaveKey,
        context: SendContext,
        delay: Option<Duration>,
    ) {
        if let Some((_, existing_delay)) = self.upstream.get_mut(key) {
            *existing_delay = match (*existing_delay, delay) {
                (None, _) | (_, None) => None,
                (Some(existing), Some(candidate)) => Some(existing.min(candidate)),
            };
        } else {
            self.upstream.insert(key, (context, delay));
        }
    }

    /// Adds one logical downstream, coalescing parallel routes to the same Enclave.
    pub(crate) fn add_downstream(&mut self, key: EnclaveKey, context: SendContext) {
        if !self.downstream.contains_key(key) {
            self.downstream.insert(key, context);
        }
    }
}

/// Scheduler-to-coordinator reports for one owned Federate's idle epoch.
enum ActivityReport {
    /// One Enclave observed queued or processed work that invalidates the idle candidate.
    Active(EnclaveKey),
    /// One Enclave observed no queued work for the stated coordinator epoch.
    Idle { key: EnclaveKey, epoch: usize },
    /// One Enclave entered the parked barrier for the stated epoch.
    Parked {
        /// Canonical identity of the parked scheduler-owned Enclave.
        key: EnclaveKey,
        /// Idle epoch that this Enclave remains parked within.
        epoch: usize,
    },
    /// One parked Enclave confirmed its event queue remained empty for the stated epoch.
    Rechecked {
        /// Canonical identity of the rechecked scheduler-owned Enclave.
        key: EnclaveKey,
        /// Parked epoch whose final queue check was empty.
        epoch: usize,
    },
    /// One Enclave scheduler exited and no longer participates in coordination.
    Complete(EnclaveKey),
    /// An execution failure requires immediate Federate-wide abortion.
    Abort,
}

/// Coordinator-to-scheduler commands for a parked termination barrier or abortion.
#[derive(Clone, Copy)]
enum ActivityCommand {
    /// Requests a fresh queue check for the stated idle epoch.
    Probe(usize),
    /// Requires a scheduler to remain parked after confirming the stated idle epoch.
    Park(usize),
    /// Requests a final event-queue check while every scheduler remains parked.
    Recheck(usize),
    /// Cancels the parked candidate and resumes every scheduler in a fresh epoch.
    Resume(usize),
    /// Commits termination from a unanimously empty parked state.
    Commit(usize),
    /// Stops the scheduler immediately after an execution failure.
    Abort,
}

/// Current phase of one owned Federate's channel-coordinated quiescence barrier.
#[derive(Clone, Copy, Eq, PartialEq)]
enum CoordinationPhase {
    /// Collecting ordinary idle observations before requesting an epoch confirmation.
    Observing,
    /// Awaiting fresh idle confirmations after a probe.
    Probing,
    /// Awaiting confirmation that every scheduler has entered the parked state.
    Parking,
    /// Awaiting final empty-queue confirmations while every scheduler remains parked.
    Rechecking,
    /// Awaiting fresh-epoch idle reports after every parked scheduler was resumed.
    Resuming,
    /// Termination was committed from the unanimously empty parked state.
    Committed,
}

/// Abort handle for one owned Federate's shared scheduler coordinator.
pub(crate) struct OwnedFederateCoordinator {
    /// Shared scheduler-report sender used to request immediate abortion.
    report_tx: mpsc::Sender<ActivityReport>,
}

impl OwnedFederateCoordinator {
    /// Interrupts idle scheduler clients; event-channel closure remains abort-only.
    pub(crate) fn abort(&self) {
        let _ = self.report_tx.send(ActivityReport::Abort);
    }
}

/// Blocking channel coordinator that confirms an uninterrupted all-idle epoch.
pub(crate) struct OwnedFederateCoordinatorRunner {
    /// Serialized activity reports from every scheduler-owned Enclave.
    report_rx: mpsc::Receiver<ActivityReport>,
    /// Per-Enclave command senders keyed by canonical scheduler identity.
    commands: BTreeMap<EnclaveKey, mpsc::Sender<ActivityCommand>>,
    /// Enclaves that have not completed or aborted execution.
    active: BTreeSet<EnclaveKey>,
    #[cfg(test)]
    /// One-shot test hook that admits work immediately before the final parked queue recheck.
    commit_window_hook: Option<Box<dyn FnOnce() + Send>>,
}

impl OwnedFederateCoordinatorRunner {
    /// Runs until all schedulers complete or abort is requested.
    pub(crate) fn run(self) {
        let Self {
            report_rx,
            commands,
            mut active,
            #[cfg(test)]
            mut commit_window_hook,
        } = self;
        let send = |command, active: &BTreeSet<EnclaveKey>| {
            for (key, tx) in &commands {
                if active.contains(key) {
                    let _ = tx.send(command);
                }
            }
        };
        let mut busy = BTreeSet::new();
        let mut seen_idle = BTreeSet::new();
        let mut confirmed = BTreeSet::new();
        let mut epoch = 0usize;
        let mut phase = CoordinationPhase::Observing;
        while !active.is_empty() {
            let report = match report_rx.recv() {
                Ok(report) => report,
                Err(_) => {
                    send(ActivityCommand::Abort, &active);
                    break;
                }
            };
            match report {
                ActivityReport::Abort => {
                    send(ActivityCommand::Abort, &active);
                    break;
                }
                ActivityReport::Complete(key) => {
                    active.remove(&key);
                    busy.remove(&key);
                    seen_idle.remove(&key);
                    confirmed.remove(&key);
                }
                ActivityReport::Active(key) => {
                    busy.insert(key);
                    seen_idle.remove(&key);
                    confirmed.clear();
                    if matches!(
                        phase,
                        CoordinationPhase::Parking | CoordinationPhase::Rechecking
                    ) {
                        epoch = epoch.wrapping_add(1);
                        seen_idle.clear();
                        send(ActivityCommand::Resume(epoch), &active);
                        phase = CoordinationPhase::Resuming;
                    } else if phase != CoordinationPhase::Committed {
                        phase = CoordinationPhase::Observing;
                    }
                }
                ActivityReport::Idle {
                    key,
                    epoch: observed,
                } => {
                    busy.remove(&key);
                    if phase == CoordinationPhase::Observing {
                        seen_idle.insert(key);
                    } else if matches!(
                        phase,
                        CoordinationPhase::Probing | CoordinationPhase::Resuming
                    ) && observed == epoch
                        && active.contains(&key)
                    {
                        confirmed.insert(key);
                    }
                }
                ActivityReport::Parked {
                    key,
                    epoch: observed,
                } if phase == CoordinationPhase::Parking
                    && observed == epoch
                    && active.contains(&key) =>
                {
                    confirmed.insert(key);
                }
                ActivityReport::Rechecked {
                    key,
                    epoch: observed,
                } if phase == CoordinationPhase::Rechecking
                    && observed == epoch
                    && active.contains(&key) =>
                {
                    confirmed.insert(key);
                }
                ActivityReport::Parked { .. } | ActivityReport::Rechecked { .. } => {}
            }
            if active.is_empty() {
                continue;
            }
            match phase {
                CoordinationPhase::Observing if busy.is_empty() && seen_idle == active => {
                    epoch = epoch.wrapping_add(1);
                    confirmed.clear();
                    send(ActivityCommand::Probe(epoch), &active);
                    phase = CoordinationPhase::Probing;
                }
                CoordinationPhase::Probing if confirmed == active => {
                    confirmed.clear();
                    send(ActivityCommand::Park(epoch), &active);
                    phase = CoordinationPhase::Parking;
                }
                CoordinationPhase::Parking if confirmed == active => {
                    #[cfg(test)]
                    if let Some(hook) = commit_window_hook.take() {
                        hook();
                    }
                    confirmed.clear();
                    send(ActivityCommand::Recheck(epoch), &active);
                    phase = CoordinationPhase::Rechecking;
                }
                CoordinationPhase::Rechecking if confirmed == active => {
                    send(ActivityCommand::Commit(epoch), &active);
                    confirmed.clear();
                    phase = CoordinationPhase::Committed;
                }
                CoordinationPhase::Resuming if confirmed == active => {
                    confirmed.clear();
                    send(ActivityCommand::Park(epoch), &active);
                    phase = CoordinationPhase::Parking;
                }
                _ => {}
            }
        }
    }
}

/// Per-Enclave endpoint for one owned Federate's shared activity protocol.
pub(crate) struct OwnedSchedulerActivity {
    /// Canonical identity of this scheduler-owned Enclave.
    key: EnclaveKey,
    /// Most recent idle-probe epoch received from the coordinator.
    epoch: usize,
    /// Whether work has been reported since this scheduler last became idle.
    active: bool,
    /// Termination epoch while this scheduler is held inside the parked barrier.
    parked_epoch: Option<usize>,
    /// Shared channel used to report scheduler activity and completion.
    report_tx: mpsc::Sender<ActivityReport>,
    /// Dedicated coordinator-command receiver for this scheduler.
    command_rx: mpsc::Receiver<ActivityCommand>,
    /// Scheduler event queue inspected before confirming idleness or termination.
    event_rx: crate::Receiver<AsyncEvent>,
}

impl OwnedSchedulerActivity {
    /// Reports this scheduler's idle observation for its current epoch.
    fn report_idle(&self) {
        let _ = self.report_tx.send(ActivityReport::Idle {
            key: self.key,
            epoch: self.epoch,
        });
    }

    /// Reports activity once until this scheduler next begins an idle wait.
    fn report_active(&mut self) {
        if !self.active {
            let _ = self.report_tx.send(ActivityReport::Active(self.key));
            self.active = true;
        }
    }

    /// Removes one queued scheduler event without leaving a parked barrier independently.
    fn take_queued_event(&self) -> Option<AsyncEvent> {
        self.event_rx.try_recv().ok().flatten()
    }
}

impl SchedulerActivity for OwnedSchedulerActivity {
    fn active(&mut self) {
        self.parked_epoch = None;
        self.report_active();
    }

    fn wait(&mut self) -> Option<AsyncEvent> {
        self.active = false;
        self.parked_epoch = None;
        self.report_idle();
        let mut parked_event = None;
        loop {
            if self.parked_epoch.is_some() {
                if parked_event.is_none() {
                    if let Some(event) = self.take_queued_event() {
                        self.report_active();
                        parked_event = Some(event);
                    }
                }
            } else if let Some(event) = self.take_queued_event() {
                self.active();
                return Some(event);
            }
            match self
                .command_rx
                .recv_timeout(std::time::Duration::from_millis(1))
            {
                Ok(ActivityCommand::Probe(epoch)) => {
                    self.parked_epoch = None;
                    self.epoch = epoch;
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.report_idle();
                }
                Ok(ActivityCommand::Park(epoch)) => {
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.parked_epoch = Some(epoch);
                    let _ = self.report_tx.send(ActivityReport::Parked {
                        key: self.key,
                        epoch,
                    });
                }
                Ok(ActivityCommand::Recheck(epoch)) if self.parked_epoch == Some(epoch) => {
                    if parked_event.is_none() {
                        if let Some(event) = self.take_queued_event() {
                            self.report_active();
                            parked_event = Some(event);
                        }
                    }
                    if parked_event.is_none() {
                        let _ = self.report_tx.send(ActivityReport::Rechecked {
                            key: self.key,
                            epoch,
                        });
                    }
                }
                Ok(ActivityCommand::Resume(epoch)) => {
                    self.parked_epoch = None;
                    self.epoch = epoch;
                    if let Some(event) = parked_event.take() {
                        return Some(event);
                    }
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.active = false;
                    self.report_idle();
                }
                Ok(ActivityCommand::Commit(epoch)) if self.parked_epoch == Some(epoch) => {
                    return None;
                }
                Ok(ActivityCommand::Recheck(_)) | Ok(ActivityCommand::Commit(_)) => {}
                Ok(ActivityCommand::Abort) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl Drop for OwnedSchedulerActivity {
    fn drop(&mut self) {
        let _ = self.report_tx.send(ActivityReport::Complete(self.key));
    }
}

/// Creates one coordinator and one activity endpoint per scheduler-owned Enclave.
pub(crate) fn owned_federate_coordination(
    keys: impl IntoIterator<Item = (EnclaveKey, crate::Receiver<AsyncEvent>)>,
) -> (
    OwnedFederateCoordinator,
    OwnedFederateCoordinatorRunner,
    BTreeMap<EnclaveKey, OwnedSchedulerActivity>,
) {
    let (report_tx, report_rx) = mpsc::channel();
    let mut commands = BTreeMap::new();
    let mut clients = BTreeMap::new();
    for (key, event_rx) in keys {
        let (command_tx, command_rx) = mpsc::channel();
        commands.insert(key, command_tx);
        clients.insert(
            key,
            OwnedSchedulerActivity {
                key,
                epoch: 0,
                active: false,
                parked_epoch: None,
                report_tx: report_tx.clone(),
                command_rx,
                event_rx,
            },
        );
    }
    let active = commands.keys().copied().collect();
    (
        OwnedFederateCoordinator {
            report_tx: report_tx.clone(),
        },
        OwnedFederateCoordinatorRunner {
            report_rx,
            commands,
            active,
            #[cfg(test)]
            commit_window_hook: None,
        },
        clients,
    )
}

/// Defines explicit compiled-image schedule methods without repeating adapter boilerplate.
macro_rules! image_schedule_accessors {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),*) -> $return_type:ty |$image:ident| $body:expr;)*) => {
        $(fn $name(&self $(, $argument: $argument_type)*) -> $return_type { let $image = self; $body })*
    };
}

/// Adapts immutable compiled-image tables to the shared scheduler's exact key domains.
impl Schedule for EnclaveImageView<'_> {
    type Action = ActionIndex;
    type Port = PortIndex;
    type Reaction = ReactionIndex;
    type Reactor = ReactorIndex;
    type Mode = ModeIndex;
    type Scope = ScopeIndex;

    fn action_capacity(&self) -> usize {
        self.actions().len()
    }

    fn reaction_limits(&self) -> ReactionSetLimits {
        let max_level = self
            .reactions()
            .values()
            .map(|reaction| Level::from(reaction.dependency_level() as usize))
            .max()
            .unwrap_or_default();
        ReactionSetLimits {
            max_level,
            num_keys: self.reactions().len(),
        }
    }

    image_schedule_accessors! {
        fn startup_actions() -> impl Iterator<Item = (Self::Action, Tag)> + '_ |image| image.startup_actions().iter().chain(image.timer_startup_actions()).map(|startup| (startup.action(), compiled_tag(startup.logical_delay_nanos())));
        fn shutdown_actions() -> impl Iterator<Item = Self::Action> + '_ |image| image.shutdown_actions().iter().copied();
        fn reactor_for_reaction(reaction: Self::Reaction) -> Self::Reactor |image| image.reactions()[reaction].reactor();
        fn scope_for_reaction(reaction: Self::Reaction) -> Self::Scope |image| image.reactions()[reaction].scope();
        fn scopes() -> impl Iterator<Item = Self::Scope> + '_ |image| image.scopes().keys();
        fn scope_for_mode(mode: Self::Mode) -> Self::Scope |image| image.modes()[mode].scope();
        fn scope_for_action(action: Self::Action) -> Self::Scope |image| image.actions()[action].scope();
        fn parent_scope(scope: Self::Scope) -> Option<Self::Scope> |image| image.scopes()[scope].parent();
        fn reactor_for_scope(scope: Self::Scope) -> Self::Reactor |image| image.scopes()[scope].reactor();
        fn mode_for_scope(scope: Self::Scope) -> Option<Self::Mode> |image| image.scopes()[scope].mode();
        fn logical_actions_in_scope(scope: Self::Scope) -> impl Iterator<Item = Self::Action> + '_ |image| image.scope_logical_actions(scope).iter().copied();
        fn timer_startups_in_scope(scope: Self::Scope) -> impl Iterator<Item = (Self::Action, Tag)> + '_ |image| image.scope_timer_startups(scope).iter().map(|startup| (startup.action(), compiled_tag(startup.logical_delay_nanos())));
        fn initial_mode_for_reactor(reactor: Self::Reactor) -> Option<Self::Mode> |image| image.reactors()[reactor].initial_mode();
        fn shutdown_reactions() -> impl Iterator<Item = (Level, Self::Reaction)> + '_ |image| compiled_reactions(image.shutdown_reactions().iter().map(|lifecycle| lifecycle.reaction()));
        fn action_triggers(action: Self::Action) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ |image| compiled_reactions(image.action_triggers(action).iter().copied());
        fn port_triggers(port: Self::Port) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ |image| compiled_reactions(image.port_triggers(port).iter().copied());
        fn reaction_filter_matches_scope(reaction: Self::Reaction) -> bool |image| { let modes = image.reaction_modes(reaction); modes.is_empty() || (modes.len() == 1 && image.scopes()[image.reactions()[reaction].scope()].mode() == Some(modes[0])) };
        fn action_is_logical(action: Self::Action) -> bool |image| !matches!(image.actions()[action].timing(), crate::image::ActionTiming::Standard { domain: crate::image::TimingDomain::Physical, .. });
        fn action_period(action: Self::Action) -> Option<Duration> |image| match image.actions()[action].timing() { crate::image::ActionTiming::Timer { period_nanos: Some(period) } => Some(Duration::nanoseconds(i64::try_from(period).expect("validated compiled timer period"))), _ => None };
        fn descendant_scopes(scope: Self::Scope) -> impl Iterator<Item = Self::Scope> + '_ |image| image.scope_descendants(scope).iter().copied();
        fn reset_reactions_in_scope(scope: Self::Scope) -> impl Iterator<Item = (Level, Self::Reaction)> + '_ |image| compiled_reactions(image.scope_reset_reactions(scope).iter().copied());
        fn startups_in_scope(scope: Self::Scope) -> impl Iterator<Item = (Self::Action, (Level, Self::Reaction))> + '_ |image| image.scope_startup_reactions(scope).iter().map(|startup| { let reaction = startup.reaction(); (startup.action(), (Level::from(reaction.level() as usize), reaction.reaction())) });
        fn reactor_root_scopes() -> impl Iterator<Item = (Self::Reactor, Self::Scope)> + '_ |image| image.reactors().keys().map(move |reactor| (reactor, image.reactors()[reactor].root_scope()));
    }
}

/// Adapts direct owned storage operations to the shared compiled-image scheduler.
impl ExecutionStorage<EnclaveImageView<'_>> for OwnedStorage<'_> {
    type Error = OwnedStorageError;

    fn prepare_startup_origin(&mut self, start_time: &mut std::time::Instant) {
        self.initialize_reaction_context_origins(*start_time);
    }

    fn action_from_runtime(&self, key: ActionKey) -> ActionIndex {
        self.scheduler_action(key)
    }

    fn push_action_value(&mut self, action: ActionIndex, tag: Tag, value: Box<dyn ReactorData>) {
        self.scheduler_push_action(action, tag, value);
    }

    fn stage_inbound_boundary_value(
        &mut self,
        key: crate::PortKey,
        tag: Tag,
        value: Box<dyn ReactorData>,
    ) -> Result<PortIndex, Self::Error> {
        OwnedStorage::stage_inbound_boundary_value(self, key, tag, value)
    }

    fn commit_boundary_ports(&mut self, tag: Tag) -> Result<(), Self::Error> {
        self.scheduler_commit_boundary_ports(tag)
    }

    fn clear_action_values(&mut self, action: ActionIndex) {
        self.scheduler_clear_action(action);
    }

    fn reschedule_action_value(&mut self, action: ActionIndex, from: Tag, to: Tag) {
        self.scheduler_reschedule_action(action, from, to);
    }

    fn execute_reactions(
        &mut self,
        reactions: &[ReactionIndex],
        tag: Tag,
        outcomes: &mut [ReactionOutcome<ActionIndex, ModeIndex>],
    ) -> Result<(), Self::Error> {
        for (&reaction, outcome) in reactions.iter().zip(outcomes) {
            self.invoke_reaction(reaction, tag)?;
            let result = self.reaction_trigger_res(reaction);
            outcome.scheduled_actions.clear();
            outcome.scheduled_actions.extend(
                result
                    .scheduled_actions
                    .iter()
                    .map(|&(action, tag)| (self.scheduler_action(action), tag)),
            );
            outcome.scheduled_shutdown = result.scheduled_shutdown;
            outcome.scheduled_mode =
                result
                    .scheduled_compiled_mode
                    .map(|request| super::core::ModeTransition {
                        target: request.target,
                        transition: request.transition,
                    });
        }
        Ok(())
    }

    fn set_ports(&self) -> impl Iterator<Item = PortIndex> + '_ {
        self.scheduler_set_ports()
    }

    fn reset_ports(&mut self) {
        OwnedStorage::reset_ports(self);
    }
}

/// Converts a validated compiled logical delay to the runtime tag representation.
fn compiled_tag(delay_nanos: u64) -> Tag {
    Tag::new(
        Duration::nanoseconds(
            i64::try_from(delay_nanos).expect("validated compiled delay exceeds runtime range"),
        ),
        0,
    )
}

/// Converts compiled level-reaction rows to the runtime's typed scheduler entries.
fn compiled_reactions(
    reactions: impl Iterator<Item = LevelReactionImage>,
) -> impl Iterator<Item = (Level, ReactionIndex)> {
    reactions.map(|reaction| (Level::from(reaction.level() as usize), reaction.reaction()))
}

/// Runs validated owned storage through the shared core with local-only coordination.
/// Owns the queue, scratch, clock, wake channel, and no-op federated hook for public execution.
pub(crate) fn run_owned_scheduler(
    storage: &mut OwnedStorage<'_>,
    config: &Config,
) -> Result<Tag, SchedulerError<OwnedStorageError>> {
    run_owned_scheduler_with_origin(storage, config, std::time::Instant::now())
}

/// Runs validated owned storage with a caller-supplied monotonic origin shared by the scheduler
/// clock and every compiled reaction context.
pub(crate) fn run_owned_scheduler_with_origin(
    storage: &mut OwnedStorage<'_>,
    config: &Config,
    origin: std::time::Instant,
) -> Result<Tag, SchedulerError<OwnedStorageError>> {
    run_owned_scheduler_with_coordination(
        storage,
        config,
        origin,
        OwnedSchedulerCoordination::new(EnclaveKey::default()),
        None,
    )
}

/// Runs validated owned storage with one Federate origin and explicit local route coordination.
pub(crate) fn run_owned_scheduler_with_coordination(
    storage: &mut OwnedStorage<'_>,
    config: &Config,
    origin: std::time::Instant,
    coordination: OwnedSchedulerCoordination,
    activity: Option<&mut OwnedSchedulerActivity>,
) -> Result<Tag, SchedulerError<OwnedStorageError>> {
    let schedule = storage.scheduler_image();
    let reaction_limits = schedule.reaction_limits();
    let reaction_capacity = reaction_limits.num_keys;
    let mut events = EventManager::new(reaction_limits, &schedule);
    let event_rx = storage.scheduler_event_rx();
    let shutdown_tx = storage.take_scheduler_shutdown_tx();
    let mut start_time = origin;
    let mut current_tag = Tag::NEVER;
    let mut last_nonterminal_tag = None;
    let mut shutdown_tag = None;
    let OwnedSchedulerCoordination {
        key,
        upstream,
        downstream: downstream_enclaves,
    } = coordination;
    let mut upstream_enclaves = upstream
        .into_iter()
        .map(|(key, (context, delay))| {
            (
                key,
                LogicalTimeBarrier {
                    released_tag: Tag::NEVER,
                    provisional_tag: Tag::NEVER,
                    upstream_ctx: context,
                    upstream_delay: delay,
                },
            )
        })
        .collect();
    #[cfg(feature = "federated")]
    let mut federated_time_barrier: Box<dyn FederatedTimeBarrier> =
        Box::new(NoFederatedTimeBarrier);
    let mut stats = Stats::default();
    let mut reaction_buffer = Vec::with_capacity(reaction_capacity);
    let mut transition_buffer = Vec::with_capacity(reaction_capacity);
    let mut outcomes = (0..reaction_capacity).map(|_| Default::default()).collect();

    SchedulerCore {
        key,
        config,
        schedule: &schedule,
        storage,
        event_rx: &event_rx,
        activity: activity.map(|activity| activity as &mut dyn SchedulerActivity),
        events: &mut events,
        start_time: &mut start_time,
        current_tag: &mut current_tag,
        last_nonterminal_tag: Some(&mut last_nonterminal_tag),
        shutdown_tag: &mut shutdown_tag,
        shutdown_tx: &shutdown_tx,
        upstream_enclaves: &mut upstream_enclaves,
        downstream_enclaves: &downstream_enclaves,
        #[cfg(feature = "federated")]
        federated_time_barrier: &mut federated_time_barrier,
        stats: &mut stats,
        reaction_buffer: &mut reaction_buffer,
        transition_buffer: &mut transition_buffer,
        outcomes: &mut outcomes,
        has_modal_scopes: schedule.has_modal_scopes(),
    }
    .try_event_loop()?;
    Ok(last_nonterminal_tag.unwrap_or(Tag::NEVER))
}

#[cfg(test)]
mod tests {
    use super::owned_federate_coordination;
    use crate::{sched::core::SchedulerActivity, AsyncEvent, Duration, EnclaveKey};
    use std::{sync::mpsc, time::Duration as StdDuration};

    #[test]
    fn parked_barrier_resumes_all_schedulers_when_work_arrives_before_commit() {
        let eventful = EnclaveKey::from(3);
        let peer = EnclaveKey::from(7);
        let (eventful_tx, eventful_rx) = kanal::unbounded();
        let (_peer_tx, peer_rx) = kanal::unbounded();
        let (coordinator, mut runner, mut activities) =
            owned_federate_coordination([(eventful, eventful_rx), (peer, peer_rx)]);
        runner.commit_window_hook = Some(Box::new(move || {
            eventful_tx
                .send(AsyncEvent::Shutdown {
                    delay: Duration::ZERO,
                })
                .unwrap();
        }));
        let mut eventful_activity = activities.remove(&eventful).unwrap();
        let mut peer_activity = activities.remove(&peer).unwrap();
        let (result_tx, result_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let coordinator_handle = scope.spawn(move || runner.run());
            let eventful_result_tx = result_tx.clone();
            let eventful_handle = scope.spawn(move || {
                eventful_result_tx
                    .send((eventful, eventful_activity.wait()))
                    .unwrap();
                eventful_result_tx
                    .send((eventful, eventful_activity.wait()))
                    .unwrap();
            });
            let peer_result_tx = result_tx.clone();
            let peer_handle = scope.spawn(move || {
                peer_result_tx.send((peer, peer_activity.wait())).unwrap();
            });
            drop(result_tx);

            let mut observations = Vec::new();
            for _ in 0..3 {
                match result_rx.recv_timeout(StdDuration::from_secs(1)) {
                    Ok(observation) => observations.push(observation),
                    Err(_) => {
                        coordinator.abort();
                        break;
                    }
                }
            }
            eventful_handle.join().unwrap();
            peer_handle.join().unwrap();
            coordinator_handle.join().unwrap();

            assert_eq!(
                observations.len(),
                3,
                "both parked schedulers must reach a later common commit"
            );
            assert!(observations.iter().any(|(key, event)| {
                *key == eventful
                    && matches!(
                        event,
                        Some(AsyncEvent::Shutdown { delay }) if *delay == Duration::ZERO
                    )
            }));
            assert!(observations
                .iter()
                .any(|(key, event)| *key == eventful && event.is_none()));
            assert!(observations
                .iter()
                .any(|(key, event)| *key == peer && event.is_none()));
        });
    }
}
