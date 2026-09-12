//! Compiled-image scheduler adapters and owned execution composition.

use super::federate::{EnclaveDependencies, FederateSchedulerCoordination};
use super::{
    barrier::LogicalTimeBarrier,
    core::{ExecutionStorage, ReactionOutcome, Schedule, SchedulerCore, SchedulerError},
    modal::EventManager,
    Config, Stats,
};
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
) -> Result<OwnedSchedulerOutcome, SchedulerError<OwnedStorageError>> {
    run_owned_scheduler_with_origin(storage, config, std::time::Instant::now())
}

/// Runs validated owned storage with a caller-supplied monotonic origin shared by the scheduler
/// clock and every compiled reaction context.
pub(crate) fn run_owned_scheduler_with_origin(
    storage: &mut OwnedStorage<'_>,
    config: &Config,
    origin: std::time::Instant,
) -> Result<OwnedSchedulerOutcome, SchedulerError<OwnedStorageError>> {
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
    federate_coordination: Option<&mut dyn FederateSchedulerCoordination>,
) -> Result<OwnedSchedulerOutcome, SchedulerError<OwnedStorageError>> {
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
        federate_coordination,
        federate_shutdown_tag: None,
        events: &mut events,
        start_time: &mut start_time,
        current_tag: &mut current_tag,
        last_nonterminal_tag: Some(&mut last_nonterminal_tag),
        shutdown_tag: &mut shutdown_tag,
        shutdown_tx: &shutdown_tx,
        upstream_enclaves: &mut upstream_enclaves,
        downstream_enclaves: &downstream_enclaves,
        #[cfg(feature = "federated")]
        federated_time_barrier: None,
        stats: &mut stats,
        reaction_buffer: &mut reaction_buffer,
        transition_buffer: &mut transition_buffer,
        outcomes: &mut outcomes,
        has_modal_scopes: schedule.has_modal_scopes(),
    }
    .try_event_loop()?;
    Ok(OwnedSchedulerOutcome {
        final_tag: last_nonterminal_tag.unwrap_or(Tag::NEVER),
        stats,
    })
}

/// Successful result of one compiled owned scheduler event loop.
pub(crate) struct OwnedSchedulerOutcome {
    /// Last logical tag containing nonterminal work.
    pub(crate) final_tag: Tag,
    /// Scheduler-local work counters retained after shutdown.
    pub(crate) stats: Stats,
}

#[cfg(test)]
/// Integration coverage for the compiled schedule, storage, and coordination composition.
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    use tinymap::TinyMapView;

    use super::*;
    use crate::{
        image::{
            ActionImage, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
            EnclaveImage, EnclaveImageView, IndexSpan, LifecycleReactionImage, ReactionImage,
            ReactorImage, RequiredBindingImage, ScopeImage, SliceRange, StateSlotIndex,
            StorageBounds, TimerStartupImage,
        },
        keepalive, AsyncEvent, CompiledModeEffectRef, Context, EnclaveBindings,
        FederateCoordinationError, ReactionBindingError, ReactionRefs, SendContext,
    };

    use crate::sched::federate::{
        FederateControlAuthorization, FederateIdleWait, FederateTagAcquisition, FederateTermination,
    };

    /// One observable scheduler-port or compiled-reaction operation.
    #[derive(Clone, Copy, Debug)]
    enum Call {
        /// A candidate was invalidated before publication.
        Active,
        /// The scheduler requested a tag.
        Acquire(Tag),
        /// The scheduler authorized control-only advancement.
        Authorize(Tag),
        /// The real local barrier requested release from its upstream Enclave.
        LocalBarrier(Tag),
        /// The compiled reaction executed at the recorded physical instant.
        Reaction(Tag, std::time::Instant),
        /// The scheduler entered the idle wait path.
        Wait,
        /// The scheduler completed a logical tag.
        Complete(Tag),
        /// The scheduler reported its shared logical horizon.
        Horizon(Tag),
        /// The scheduler reported successful terminal processing.
        Stopped,
        /// The scheduler reported terminal failure.
        Fail,
    }

    /// Scripted backend-neutral port used with real compiled image storage.
    struct RecordingCoordination {
        /// Ordered operations shared with the compiled reaction binding.
        calls: Arc<Mutex<Vec<Call>>>,
        /// Acquisition results returned in request order.
        acquisitions: VecDeque<FederateTagAcquisition>,
        /// Idle-wait results returned in request order.
        waits: VecDeque<FederateIdleWait>,
        /// Control-authorization results returned in request order.
        authorizations: VecDeque<FederateControlAuthorization>,
        /// Scripted terminal result consumed after the scheduler event channel closes.
        terminal_after_close:
            Option<Result<Option<FederateTermination>, FederateCoordinationError>>,
    }

    impl FederateSchedulerCoordination for RecordingCoordination {
        /// Records candidate invalidation.
        fn active(&mut self) {
            self.calls.lock().unwrap().push(Call::Active);
        }

        /// Returns the next scripted idle-wait result.
        fn wait(&mut self) -> Result<FederateIdleWait, FederateCoordinationError> {
            self.calls.lock().unwrap().push(Call::Wait);
            Ok(self.waits.pop_front().expect("scripted idle wait outcome"))
        }

        /// Returns the next scripted acquisition result.
        fn acquire_tag(
            &mut self,
            tag: Tag,
        ) -> Result<FederateTagAcquisition, FederateCoordinationError> {
            self.calls.lock().unwrap().push(Call::Acquire(tag));
            Ok(self
                .acquisitions
                .pop_front()
                .expect("scripted acquisition outcome"))
        }

        /// Returns the next scripted control-authorization result.
        fn authorize_control(
            &mut self,
            tag: Tag,
        ) -> Result<FederateControlAuthorization, FederateCoordinationError> {
            self.calls.lock().unwrap().push(Call::Authorize(tag));
            Ok(self
                .authorizations
                .pop_front()
                .expect("scripted control authorization outcome"))
        }

        /// Returns the scripted Federate terminal result after scheduler wake-channel closure.
        fn terminal_after_event_channel_closed(
            &mut self,
        ) -> Result<Option<FederateTermination>, FederateCoordinationError> {
            self.terminal_after_close.take().unwrap_or(Ok(None))
        }

        /// Records logical completion.
        fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederateCoordinationError> {
            self.calls.lock().unwrap().push(Call::Complete(tag));
            Ok(())
        }

        /// Records a shared logical horizon.
        fn logical_horizon_reached(&mut self, tag: Tag) {
            self.calls.lock().unwrap().push(Call::Horizon(tag));
        }

        /// Records successful terminal processing.
        fn participant_stopped(&mut self) {
            self.calls.lock().unwrap().push(Call::Stopped);
        }

        /// Records terminal failure.
        fn fail(&mut self) {
            self.calls.lock().unwrap().push(Call::Fail);
        }
    }

    /// State value required by the compiled test reactor.
    struct TestState;

    /// Initializes the compiled test reactor state.
    fn initialize_state() -> TestState {
        TestState
    }

    /// Reactor table for the compiled scheduler fixture.
    static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
        BindingSlotIndex::new(0),
        StateSlotIndex::new(0),
        ScopeIndex::new(0),
        IndexSpan::new(0, 0),
        None,
        None,
    )];
    /// Action table for the compiled scheduler fixture.
    static ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Timer { period_nanos: None },
        SliceRange::new(0, 1),
        None,
    )];
    /// Reaction table for the compiled scheduler fixture.
    static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
    )];
    /// Scope table for the compiled scheduler fixture.
    static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
        None,
        ReactorIndex::new(0),
        None,
        SliceRange::new(0, 1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
    )];
    /// Trigger table for the compiled scheduler fixture.
    static TRIGGERS: [LevelReactionImage; 1] = [LevelReactionImage::new(0, ReactionIndex::new(0))];
    /// Scope-descendant table for the compiled scheduler fixture.
    static DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
    /// Timer-startup table for the compiled scheduler fixture.
    static STARTUPS: [TimerStartupImage; 1] =
        [TimerStartupImage::new(ActionIndex::new(0), 50_000_000)];
    /// Required-binding table for the compiled scheduler fixture.
    static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("astate"),
            BindingKind::StateInitializer,
        ),
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("breaction"),
            BindingKind::Reaction,
        ),
    ];
    /// Enclave image assembled from the compiled scheduler fixture tables.
    static IMAGE: EnclaveImage<'static> = EnclaveImage {
        enclave_id: crate::image::EnclaveId::new("enclave"),
        reactors: TinyMapView::new(&REACTORS),
        actions: TinyMapView::new(&ACTIONS),
        ports: TinyMapView::new(&[]),
        reactions: TinyMapView::new(&REACTIONS),
        modes: TinyMapView::new(&[]),
        scopes: TinyMapView::new(&SCOPES),
        reaction_triggers: &TRIGGERS,
        reaction_use_ports: &[],
        reaction_effect_ports: &[],
        reaction_actions: &[],
        reaction_modes: &[],
        scope_descendants: &DESCENDANTS,
        scope_logical_actions: &[],
        scope_timer_startups: &[],
        scope_reset_reactions: &[],
        scope_startup_reactions: &[],
        scope_shutdown_reactions: &[],
        startup_actions: &[],
        timer_startup_actions: &STARTUPS,
        shutdown_reactions: &[],
        shutdown_actions: &[],
        routes: TinyMapView::new(&[]),
        required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
        storage_bounds: StorageBounds::new(1, 1, 1, 0, 0, 0),
    };
    /// Shutdown-reaction table for terminal local-barrier coverage.
    static SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        ActionIndex::new(0),
    )];
    /// Compiled fixture whose reaction runs only as graceful shutdown work.
    static SHUTDOWN_IMAGE: EnclaveImage<'static> = EnclaveImage {
        shutdown_reactions: &SHUTDOWN_REACTIONS,
        ..IMAGE
    };

    /// Builds real owned compiled storage with a configurable reaction result.
    fn build_storage(calls: Arc<Mutex<Vec<Call>>>, fail_reaction: bool) -> OwnedStorage<'static> {
        build_storage_for_image(&IMAGE, calls, fail_reaction)
    }

    /// Builds real owned storage for a selected compiled test image.
    fn build_storage_for_image(
        image: &'static EnclaveImage<'static>,
        calls: Arc<Mutex<Vec<Call>>>,
        fail_reaction: bool,
    ) -> OwnedStorage<'static> {
        let bindings = EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                move |context: &mut Context,
                      _state: &mut dyn ReactorData,
                      _refs: ReactionRefs<'_>,
                      _effect: Option<CompiledModeEffectRef>| {
                    calls
                        .lock()
                        .unwrap()
                        .push(Call::Reaction(context.get_tag(), std::time::Instant::now()));
                    if fail_reaction {
                        Err(ReactionBindingError::missing("injected reaction input"))
                    } else {
                        Ok(())
                    }
                },
            );
        OwnedStorage::new(EnclaveImageView::new(image).unwrap(), bindings).unwrap()
    }

    /// Builds a scripted coordination port over one shared call log.
    fn coordination(
        calls: Arc<Mutex<Vec<Call>>>,
        acquisitions: impl IntoIterator<Item = FederateTagAcquisition>,
        waits: impl IntoIterator<Item = FederateIdleWait>,
        authorizations: impl IntoIterator<Item = FederateControlAuthorization>,
    ) -> RecordingCoordination {
        RecordingCoordination {
            calls,
            acquisitions: acquisitions.into_iter().collect(),
            waits: waits.into_iter().collect(),
            authorizations: authorizations.into_iter().collect(),
            terminal_after_close: None,
        }
    }

    #[test]
    /// Treats a terminal wake-channel close inside a local barrier as a graceful Federate stop.
    fn terminal_local_barrier_close_stops_without_local_coordination_error() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
        let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
        dependencies.add_upstream(
            upstream,
            SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx: upstream_shutdown_rx,
            },
            None,
        );
        let event_tx = storage.scheduler_event_tx();
        let closer = std::thread::spawn(move || {
            upstream_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("scheduler must enter the local barrier");
            event_tx.close().expect("scheduler event channel closes");
        });
        let mut port = coordination(
            Arc::clone(&calls),
            [FederateTagAcquisition::Granted],
            [],
            [],
        );
        port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful { tag: None })));

        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            dependencies,
            Some(&mut port),
        )
        .unwrap();
        closer.join().unwrap();
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == Tag::ZERO)));
    }

    #[test]
    /// Observes closed-channel terminal coordination before entering an idle wait.
    fn closed_event_channel_observes_terminal_before_idle_wait() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
        let event_tx = storage.scheduler_event_tx();
        event_tx.close().unwrap();
        let mut port = coordination(Arc::clone(&calls), [], [], []);
        port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful { tag: None })));

        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
        .unwrap();

        let calls = calls.lock().unwrap();
        assert!(!calls
            .iter()
            .any(|call| matches!(call, Call::Active | Call::Acquire(_))));
        assert!(calls
            .iter()
            .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == Tag::ZERO)));
    }

    #[test]
    /// Preserves an authorized logical-horizon tag through local-barrier channel closure.
    fn terminal_local_barrier_close_preserves_logical_horizon() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
        let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
        dependencies.add_upstream(
            upstream,
            SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx: upstream_shutdown_rx,
            },
            None,
        );
        let event_tx = storage.scheduler_event_tx();
        let closer = std::thread::spawn(move || {
            upstream_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("scheduler must enter the local barrier");
            event_tx.close().expect("scheduler event channel closes");
        });
        let horizon = Tag::new(Duration::milliseconds(50), 0);
        let mut port = coordination(
            Arc::clone(&calls),
            [FederateTagAcquisition::Granted],
            [],
            [],
        );
        port.terminal_after_close = Some(Ok(Some(FederateTermination::Graceful {
            tag: Some(horizon),
        })));

        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            dependencies,
            Some(&mut port),
        )
        .unwrap();
        closer.join().unwrap();
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == horizon)));
    }

    #[test]
    /// Preserves a queued logical horizon while authorizing local control-only work.
    fn control_termination_preserves_logical_horizon() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
        let horizon = Tag::new(Duration::milliseconds(25), 0);
        storage
            .scheduler_event_tx()
            .send(AsyncEvent::provisional(EnclaveKey::from(1), horizon))
            .unwrap();
        let mut port = coordination(
            Arc::clone(&calls),
            [],
            [],
            [FederateControlAuthorization::LogicalHorizon(horizon)],
        );

        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
        .unwrap();
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == horizon)));
    }

    #[test]
    /// Bypasses shutdown reactions when abort closes a scheduler blocked in a local barrier.
    fn aborted_local_barrier_close_bypasses_shutdown_reactions() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&SHUTDOWN_IMAGE, Arc::clone(&calls), false);
        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
        let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
        dependencies.add_upstream(
            upstream,
            SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx: upstream_shutdown_rx,
            },
            None,
        );
        let event_tx = storage.scheduler_event_tx();
        let closer = std::thread::spawn(move || {
            upstream_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("scheduler must enter the local barrier");
            event_tx.close().expect("scheduler event channel closes");
        });
        let mut port = coordination(
            Arc::clone(&calls),
            [FederateTagAcquisition::Granted],
            [],
            [],
        );
        port.terminal_after_close = Some(Ok(Some(FederateTermination::Abort)));

        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            dependencies,
            Some(&mut port),
        )
        .unwrap();
        closer.join().unwrap();
        assert!(!calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Call::Reaction(..))));
    }

    #[test]
    /// Preserves a Federate failure when it closes a scheduler blocked in a local barrier.
    fn terminal_local_barrier_close_preserves_federate_failure() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage(Arc::clone(&calls), false);
        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
        let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
        dependencies.add_upstream(
            upstream,
            SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx: upstream_shutdown_rx,
            },
            None,
        );
        let event_tx = storage.scheduler_event_tx();
        let closer = std::thread::spawn(move || {
            upstream_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("scheduler must enter the local barrier");
            event_tx.close().expect("scheduler event channel closes");
        });
        let mut port = coordination(calls, [FederateTagAcquisition::Granted], [], []);
        port.terminal_after_close = Some(Err(FederateCoordinationError::BackendPublish {
            message: "injected coordinator failure".to_owned(),
        }));

        let error = match run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            dependencies,
            Some(&mut port),
        ) {
            Ok(_) => panic!("the injected Federate failure must be preserved"),
            Err(error) => error,
        };
        closer.join().unwrap();
        assert!(matches!(
            error,
            SchedulerError::FederateFailureReported { source }
                if matches!(
                    *source,
                    SchedulerError::FederateCoordination(
                        FederateCoordinationError::BackendPublish { ref message }
                    ) if message == "injected coordinator failure"
                )
        ));
    }

    #[test]
    /// Exercises ordering, idle, horizon, terminal, and failure ownership in compiled execution.
    fn compiled_scheduler_composes_federate_horizon_with_local_barriers() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let origin = std::time::Instant::now();
        let control_tag = Tag::new(Duration::milliseconds(25), 0);
        let reaction_tag = Tag::new(Duration::milliseconds(50), 0);
        let target = reaction_tag.to_logical_time(origin);
        let mut storage = build_storage(Arc::clone(&calls), false);
        let upstream = EnclaveKey::from(1);
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_upstream_shutdown_tx, upstream_shutdown_rx) = keepalive::channel();
        let mut dependencies = EnclaveDependencies::new(EnclaveKey::default());
        dependencies.add_upstream(
            upstream,
            SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx: upstream_shutdown_rx,
            },
            None,
        );
        let event_tx = storage.scheduler_event_tx();
        event_tx
            .send(AsyncEvent::provisional(upstream, control_tag))
            .unwrap();
        let barrier_calls = Arc::clone(&calls);
        let event_thread = std::thread::spawn(move || {
            for expected in [control_tag, reaction_tag] {
                let request = upstream_rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap_or_else(|error| {
                        let _ = event_tx.send(AsyncEvent::release(upstream, Tag::FOREVER));
                        panic!("local barrier did not request {expected}: {error}");
                    });
                let requested = match request {
                    AsyncEvent::TagReleaseProvisional { enclave, tag }
                        if enclave == EnclaveKey::default() =>
                    {
                        tag
                    }
                    other => {
                        let _ = event_tx.send(AsyncEvent::release(upstream, Tag::FOREVER));
                        panic!("unexpected local barrier request: {other}");
                    }
                };
                barrier_calls
                    .lock()
                    .unwrap()
                    .push(Call::LocalBarrier(requested));
                event_tx
                    .send(AsyncEvent::release(upstream, requested))
                    .unwrap();
                assert_eq!(requested, expected);
            }
        });
        let mut port = coordination(
            Arc::clone(&calls),
            [
                FederateTagAcquisition::Granted,
                FederateTagAcquisition::Granted,
                FederateTagAcquisition::Granted,
            ],
            [FederateIdleWait::Interrupted(AsyncEvent::Shutdown {
                delay: Duration::ZERO,
            })],
            [
                FederateControlAuthorization::Authorized,
                FederateControlAuthorization::Authorized,
            ],
        );
        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_keep_alive(true),
            origin,
            dependencies,
            Some(&mut port),
        )
        .unwrap();
        event_thread.join().unwrap();
        let observed = calls.lock().unwrap();
        let reacted_at = observed.iter().find_map(|call| match call {
            Call::Reaction(tag, at) if *tag == reaction_tag => Some(*at),
            _ => None,
        });
        let authorization = observed
            .iter()
            .position(|call| matches!(call, Call::Authorize(tag) if *tag == control_tag))
            .unwrap();
        let control_barrier = observed
            .iter()
            .position(|call| matches!(call, Call::LocalBarrier(tag) if *tag == control_tag))
            .unwrap();
        let acquisition = observed
            .iter()
            .position(|call| matches!(call, Call::Acquire(tag) if *tag == reaction_tag))
            .unwrap();
        let reaction_barrier = observed
            .iter()
            .position(|call| matches!(call, Call::LocalBarrier(tag) if *tag == reaction_tag))
            .unwrap();
        let reaction = observed
            .iter()
            .position(|call| matches!(call, Call::Reaction(tag, _) if *tag == reaction_tag))
            .unwrap();
        let completion = observed
            .iter()
            .position(|call| matches!(call, Call::Complete(tag) if *tag == reaction_tag))
            .unwrap();
        let stopped = observed
            .iter()
            .position(|call| matches!(call, Call::Stopped))
            .unwrap();
        assert!(authorization < control_barrier);
        assert!(acquisition < reaction_barrier);
        assert!(reaction_barrier < reaction);
        assert!(reaction < completion);
        assert!(completion < stopped);
        assert!(!observed
            .iter()
            .any(|call| matches!(call, Call::Acquire(tag) if *tag == control_tag)));
        assert!(reacted_at.unwrap() >= target);
        assert!(observed.iter().any(|call| matches!(call, Call::Wait)));
        assert!(observed
            .iter()
            .any(|call| { matches!(call, Call::Complete(tag) if *tag == reaction_tag) }));
        drop(observed);

        calls.lock().unwrap().clear();
        let mut storage = build_storage(Arc::clone(&calls), false);
        let horizon = Tag::new(Duration::milliseconds(100), 0);
        let mut port = coordination(
            Arc::clone(&calls),
            [
                FederateTagAcquisition::Granted,
                FederateTagAcquisition::Granted,
            ],
            [FederateIdleWait::LogicalHorizon(horizon)],
            [],
        );
        let outcome = run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default()
                .with_fast_forward(true)
                .with_timeout(Duration::milliseconds(100)),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
        .unwrap();
        assert_eq!(outcome.stats.processed_tags(), 2);
        let observed = calls.lock().unwrap();
        assert!(!observed
            .iter()
            .any(|call| matches!(call, Call::Horizon(tag) if *tag == horizon)));
        assert!(!observed
            .iter()
            .any(|call| matches!(call, Call::Complete(tag) if *tag == horizon)));
        drop(observed);

        calls.lock().unwrap().clear();
        let mut storage = build_storage(Arc::clone(&calls), false);
        let mut port = coordination(
            Arc::clone(&calls),
            [FederateTagAcquisition::Stopped],
            [],
            [],
        );
        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
        .unwrap();
        let observed = calls.lock().unwrap();
        assert!(!observed
            .iter()
            .any(|call| matches!(call, Call::Reaction(..) | Call::Complete(_))));
        drop(observed);

        calls.lock().unwrap().clear();
        let mut storage = build_storage(Arc::clone(&calls), true);
        let mut port = coordination(
            Arc::clone(&calls),
            [FederateTagAcquisition::Granted],
            [],
            [],
        );
        let error = match run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default().with_fast_forward(true),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        ) {
            Ok(_) => panic!("the injected compiled reaction must fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            SchedulerError::FederateFailureReported { source }
                if matches!(*source, SchedulerError::Execution(_))
        ));
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, Call::Fail))
                .count(),
            1
        );
    }
}
