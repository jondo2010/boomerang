//! Compiled scheduler fixture retaining state between real core turns for protocol checks.
pub(super) use super::super::*;
use crate::sched::{core::ModeTransition, output::OutputState};
pub(super) use crate::{CommonContext, OutputBound, OutputFault, OutputOutcome, OutputWaitError};
pub(super) use std::time::Duration as HostDuration;

// The image has no startup work: each test explicitly chooses the pending tags.
static EFFECT_REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
    SliceRange::new(0, 0),
)
.with_external_effect(true)];
pub(super) static EFFECT_IMAGE: EnclaveImage<'static> = EnclaveImage {
    reactions: TinyMapRef::from_slice(&EFFECT_REACTIONS),
    timer_startup_actions: &[],
    storage_bounds: &StorageBounds::new(1, 1, 1024, 0, 0, 0),
    ..IMAGE
};

pub(super) struct CompiledScheduler {
    // The real owned storage closes its mailbox before dropping user state; the
    // detached output state must likewise close outstanding waits first.
    pub output: OutputState,
    pub storage: OwnedStorage<'static>,
    pub schedule: EnclaveImageView<'static>,
    pub event_tx: crate::Sender<AsyncEvent>,
    pub clock: crate::clock::ExecutionClock,
    pub key: EnclaveKey,
    pub config: Config,
    pub event_rx: crate::Receiver<AsyncEvent>,
    pub events: EventManager<EnclaveImageView<'static>>,
    pub start_time: std::time::Instant,
    pub current_tag: Tag,
    pub last_nonterminal_tag: Option<Tag>,
    pub shutdown_tag: Option<Tag>,
    pub shutdown_tx: keepalive::Sender,
    pub upstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    pub downstream_enclaves: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    pub stats: Stats,
    pub observation: Option<crate::ObservationHandle>,
    reaction_buffer: Vec<ReactionIndex>,
    transition_buffer: Vec<(ReactorIndex, ModeTransition<ModeIndex>)>,
    outcomes: Vec<ReactionOutcome<ActionIndex, ModeIndex>>,
    started: bool,
    terminated: bool,
}

impl CompiledScheduler {
    pub fn new(key: EnclaveKey, mut storage: OwnedStorage<'static>) -> Self {
        let schedule = storage.scheduler_image();
        let limits = schedule.reaction_limits();
        let capacity = limits.num_keys;
        let events = EventManager::new(limits, &schedule, true);
        let event_tx = storage.scheduler_event_tx();
        let event_rx = storage.scheduler_event_rx();
        let shutdown_tx = storage.take_scheduler_shutdown_tx();
        let clock = storage.scheduler_send_context().clock;
        let output = storage.take_output_state();
        Self {
            output,
            storage,
            schedule,
            event_tx,
            clock,
            key,
            config: Config::default(),
            event_rx,
            events,
            start_time: std::time::Instant::now(),
            current_tag: Tag::NEVER,
            last_nonterminal_tag: None,
            shutdown_tag: None,
            shutdown_tx,
            upstream_enclaves: Default::default(),
            downstream_enclaves: Default::default(),
            stats: Stats::default(),
            observation: None,
            reaction_buffer: Vec::with_capacity(capacity),
            transition_buffer: Vec::with_capacity(capacity),
            outcomes: (0..capacity).map(|_| Default::default()).collect(),
            started: false,
            terminated: false,
        }
    }
    pub fn output_observer(
        &self,
        owner: ReactorIndex,
    ) -> Result<crate::OutputObserver, OutputWaitError> {
        self.output.observer(owner)
    }
    pub fn core(
        &mut self,
    ) -> SchedulerCore<'_, '_, EnclaveImageView<'static>, OwnedStorage<'static>> {
        SchedulerCore {
            output: Some(&mut self.output),
            clock: self.clock.clone(),
            key: self.key,
            config: &self.config,
            observation: self.observation.as_ref(),
            schedule: &self.schedule,
            storage: &mut self.storage,
            event_rx: &self.event_rx,
            federate_coordination: None,
            federate_shutdown_tag: None,
            events: &mut self.events,
            start_time: &mut self.start_time,
            current_tag: &mut self.current_tag,
            last_nonterminal_tag: Some(&mut self.last_nonterminal_tag),
            shutdown_tag: &mut self.shutdown_tag,
            shutdown_tx: &self.shutdown_tx,
            upstream_enclaves: &mut self.upstream_enclaves,
            downstream_enclaves: &self.downstream_enclaves,
            stats: &mut self.stats,
            reaction_buffer: &mut self.reaction_buffer,
            transition_buffer: &mut self.transition_buffer,
            outcomes: &mut self.outcomes,
            has_modal_scopes: self.schedule.has_modal_scopes(),
        }
    }
    #[cfg(feature = "hosted-manual-clock")]
    pub fn set_clock(&mut self, clock: crate::clock::ExecutionClock) {
        self.output.select_clock(&clock).unwrap();
        self.storage.set_clock(clock.clone()).unwrap();
        self.clock = clock;
    }
    pub fn startup(&mut self) {
        if self.started {
            return;
        }
        self.start_time = self.storage.initialize_clock(std::time::Instant::now());
        self.core().startup();
        self.started = true;
    }
    pub fn try_next(&mut self) -> Result<bool, SchedulerError<OwnedStorageError>> {
        if self.terminated {
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
            self.terminated = true;
        }
        result
    }
    pub fn try_event_loop(&mut self) -> Result<(), SchedulerError<OwnedStorageError>> {
        self.startup();
        while self.try_next()? {}
        Ok(())
    }
}

pub(super) fn fixture(
    output: OutputOutcome,
) -> (
    CompiledScheduler,
    ReactorIndex,
    ReactionIndex,
    Arc<Mutex<Vec<Tag>>>,
) {
    fixture_at(EnclaveKey::from(4), output)
}
pub(super) fn fixture_at(
    key: EnclaveKey,
    output: OutputOutcome,
) -> (
    CompiledScheduler,
    ReactorIndex,
    ReactionIndex,
    Arc<Mutex<Vec<Tag>>>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
                captured.lock().unwrap().push(ctx.get_tag());
                Ok(output)
            },
        );
    let storage = OwnedStorage::new(
        EnclaveImageView::new(EFFECT_IMAGE.clone()).unwrap(),
        bindings,
    )
    .unwrap();
    (
        CompiledScheduler::new(key, storage),
        ReactorIndex::new(0),
        ReactionIndex::new(0),
        calls,
    )
}

pub(super) fn callback_fixture<F>(callback: F) -> CompiledScheduler
where
    F: for<'store> FnMut(
            &mut Context,
            &mut dyn ReactorData,
            ReactionRefs<'store>,
            Option<CompiledModeEffectRef>,
        ) -> Result<OutputOutcome, ReactionBindingError>
        + Send
        + Sync
        + 'static,
{
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(BindingSlotIndex::new(1), callback);
    CompiledScheduler::new(
        EnclaveKey::from(4),
        OwnedStorage::new(
            EnclaveImageView::new(EFFECT_IMAGE.clone()).unwrap(),
            bindings,
        )
        .unwrap(),
    )
}

/// One typed payload action usable for physical ingress or logical feedback.
pub(super) fn action_fixture<F>(physical: bool, callback: F) -> CompiledScheduler
where
    F: for<'store> FnMut(
            &mut Context,
            &mut dyn ReactorData,
            ReactionRefs<'store>,
            Option<CompiledModeEffectRef>,
        ) -> Result<OutputOutcome, ReactionBindingError>
        + Send
        + Sync
        + 'static,
{
    use crate::image::{BindingSlotId, TimingDomain};
    static ACTION_BINDINGS: [RequiredBindingImage; 3] = [
        RequiredBindingImage::new(BindingSlotId::new("astate"), BindingKind::StateInitializer),
        RequiredBindingImage::new(BindingSlotId::new("breaction"), BindingKind::Reaction),
        RequiredBindingImage::new(BindingSlotId::new("cpayload"), BindingKind::Action),
    ];
    static LOGICAL_ACTION: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 0,
        },
        SliceRange::new(0, 1),
        Some(BindingSlotIndex::new(2)),
    )];
    static PHYSICAL_ACTION: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Physical,
            min_delay_nanos: 0,
        },
        SliceRange::new(0, 1),
        Some(BindingSlotIndex::new(2)),
    )];
    static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 1),
        SliceRange::new(0, 0),
    )
    .with_external_effect(true)];
    static REACTION_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
    static BOUNDS: StorageBounds = StorageBounds::new(1, 1, 4, 0, 0, 0);
    let image = EnclaveImageView::new(EnclaveImage {
        actions: TinyMapRef::from_slice(if physical {
            &PHYSICAL_ACTION
        } else {
            &LOGICAL_ACTION
        }),
        reactions: TinyMapRef::from_slice(&REACTIONS),
        reaction_actions: &REACTION_ACTIONS,
        timer_startup_actions: &[],
        required_bindings: TinyMapRef::from_slice(&ACTION_BINDINGS),
        storage_bounds: &BOUNDS,
        ..IMAGE.clone()
    })
    .unwrap();
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_action(BindingSlotIndex::new(2), crate::PayloadType::<u32>::new())
        .bind_reaction(BindingSlotIndex::new(1), callback);
    CompiledScheduler::new(
        EnclaveKey::from(1),
        OwnedStorage::new(image, bindings).unwrap(),
    )
}
