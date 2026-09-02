//! Compiled-image scheduler adapters and owned execution composition.

use super::federate::{EnclaveDependencies, QuiescenceControl, QuiescenceParticipant};
use super::{
    barrier::LogicalTimeBarrier,
    core::{ExecutionStorage, ReactionOutcome, Schedule, SchedulerCore, SchedulerError},
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
    ActionKey, Duration, EnclaveKey, Level, OwnedStorage, OwnedStorageError, ReactionSetLimits,
    ReactorData, Tag,
};

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
        port: PortIndex,
        tag: Tag,
        value: Box<dyn ReactorData>,
    ) -> Result<PortIndex, Self::Error> {
        OwnedStorage::stage_inbound_boundary_value(self, port, tag, value)
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
        EnclaveDependencies::new(EnclaveKey::default()),
        None,
    )
}

/// Runs validated owned storage with one Federate origin and explicit local route coordination.
pub(crate) fn run_owned_scheduler_with_coordination(
    storage: &mut OwnedStorage<'_>,
    config: &Config,
    origin: std::time::Instant,
    dependencies: EnclaveDependencies,
    participant: Option<&mut QuiescenceParticipant>,
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
    let EnclaveDependencies {
        key,
        upstream,
        downstream: downstream_enclaves,
    } = dependencies;
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
        quiescence: participant.map(|participant| participant as &mut dyn QuiescenceControl),
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
