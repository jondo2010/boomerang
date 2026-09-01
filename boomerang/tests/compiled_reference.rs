//! Exercises the host-runtime seam from a compiled image and direct bindings through owned storage and scheduling to typed results, excluding compiler lowering and live-graph construction.

use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

use boomerang::runtime::AsyncEvent;
use boomerang::runtime::{
    execute_owned, execute_owned_federate,
    image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
        BoundaryId, CompiledDeploymentImage, CoordinationProjection, EnclaveImage, EnclaveIndex,
        FederateImage, FederateIndex, GlobalFederationImage, IdentityRange, ImageValidationError,
        LevelReactionImage, LifecycleReactionImage, ModeImage, ModeIndex, PortImage, PortIndex,
        ReactionImage, ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage,
        RouteDirection, RouteImage, ScopeImage, ScopeIndex, StateSlotIndex, StorageBounds,
        TableRange, TimerStartupImage, TimingDomain, TinyMapView,
    },
    ActionRef, CommonContext, CompiledModeEffectRef, Config, Context, Duration, EnclaveBindings,
    EnclaveKey, ExecuteOwnedError, ExecuteOwnedFederateError, FederateBindings, InputRef,
    ModeEffectRef, ModeKey, OutputRef, OwnedStorageError, PayloadType, ReactionBindingError,
    ReactionRefs, ReactorData, RuntimeError, StateAccessError, Tag, TransitionKind,
};

macro_rules! r {
    ($start:expr, $len:expr) => {
        TableRange::new($start, $len)
    };
}

/// Mutable reactor state whose startup reaction records one execution.
#[derive(Debug)]
struct CounterState {
    count: usize,
    tags: Vec<Tag>,
}

/// Initializes the counter state supplied by the direct state binding.
fn initialize_counter() -> CounterState {
    CounterState {
        count: 0,
        tags: Vec::new(),
    }
}

/// Increments the directly bound reactor state when its startup reaction executes.
fn increment_counter(
    _context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    state
        .downcast_mut::<CounterState>()
        .expect("the image's state binding initializes CounterState")
        .count += 1;
    Ok(())
}

fn reference_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), increment_counter)
}

static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    r!(0, 0),
    None,
    None,
)];
static ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    r!(0, 1),
    None,
)];
static COALESCED_ACTIONS: [ActionImage; 2] = [
    ACTIONS[0],
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Timer { period_nanos: None },
        r!(1, 1),
        None,
    ),
];
static PORTS: [boomerang::runtime::image::PortImage; 0] = [];
static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
)];
static MODES: [boomerang::runtime::image::ModeImage; 0] = [];
static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
    None,
    ReactorIndex::new(0),
    None,
    r!(0, 1),
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
)];
static REACTION_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
static COALESCED_REACTION_TRIGGERS: [LevelReactionImage; 2] = [
    REACTION_TRIGGERS[0],
    LevelReactionImage::new(0, ReactionIndex::new(0)),
];
static SCOPE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
static SCOPE_LOGICAL_ACTIONS: [ActionIndex; 1] = [ActionIndex::new(0)];
static SCOPE_STARTUP_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    ActionIndex::new(0),
)];
static STARTUP_ACTIONS: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 5)];
static COALESCED_STARTUP_ACTIONS: [TimerStartupImage; 2] = [
    TimerStartupImage::new(ActionIndex::new(0), 0),
    TimerStartupImage::new(ActionIndex::new(1), 1),
];
static ROUTES: [boomerang::runtime::image::RouteImage; 0] = [];
static ROUTED_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 0),
    BindingSlotIndex::new(2),
)];
static ROUTED_ROUTES: [RouteImage; 1] = [RouteImage::new(
    IdentityRange::new(0, 18),
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];
static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
    RequiredBindingImage::new(IdentityRange::new(18, 13), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(31, 17), BindingKind::Reaction),
];
static ROUTED_REQUIRED_BINDINGS: [RequiredBindingImage; 3] = [
    REQUIRED_BINDINGS[0],
    REQUIRED_BINDINGS[1],
    RequiredBindingImage::new(IdentityRange::new(48, 11), BindingKind::Port),
];

const fn fixture_reaction(
    scope: u32,
    binding: u32,
    use_ports: TableRange<PortIndex>,
    actions: TableRange<ActionIndex>,
    modes: TableRange<ModeIndex>,
) -> ReactionImage {
    ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(scope),
        0,
        BindingSlotIndex::new(binding),
        use_ports,
        r!(0, 0),
        actions,
        modes,
    )
}

const fn fixture_timer_action(
    slot: u32,
    period_nanos: Option<u64>,
    triggers: TableRange<LevelReactionImage>,
) -> ActionImage {
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(slot),
        ActionTiming::Timer { period_nanos },
        triggers,
        None,
    )
}

const fn fixture_scope(
    parent: Option<ScopeIndex>,
    mode: Option<ModeIndex>,
    descendants: TableRange<ScopeIndex>,
    logical_actions: TableRange<ActionIndex>,
    timer_startups: TableRange<TimerStartupImage>,
    startups: TableRange<LifecycleReactionImage>,
) -> ScopeImage {
    ScopeImage::new(
        parent,
        ReactorIndex::new(0),
        mode,
        descendants,
        logical_actions,
        timer_startups,
        r!(0, 0),
        startups,
        r!(0, 0),
    )
}

#[derive(Debug)]
struct ModeState {
    entered_mode: u32,
    reset_once: bool,
    periodic_tags: Vec<Tag>,
}

fn initialize_mode_state() -> ModeState {
    ModeState {
        entered_mode: 0,
        reset_once: false,
        periodic_tags: Vec::new(),
    }
}

fn request_compiled_mode(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let effect = effect.ok_or_else(|| ReactionBindingError::missing("compiled mode effect"))?;
    effect.set(context);
    Ok(())
}

fn reset_periodic_mode_once(
    context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let effect = effect.ok_or_else(|| ReactionBindingError::missing("compiled mode effect"))?;
    let state = state
        .downcast_mut::<ModeState>()
        .expect("the modal image initializes ModeState");
    state.periodic_tags.push(context.get_tag());
    if !state.reset_once {
        state.reset_once = true;
        effect.set(context);
    }
    Ok(())
}

fn request_forged_compiled_mode(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let _effect = effect.ok_or_else(|| ReactionBindingError::missing("compiled mode effect"))?;
    CompiledModeEffectRef {
        target: ModeIndex::new(0),
        transition: TransitionKind::History,
    }
    .set(context);
    Ok(())
}

fn request_legacy_mode(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    ModeEffectRef::new_key(ModeKey::from(1), TransitionKind::Reset).set(context);
    Ok(())
}

fn record_mode_entry(
    context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    state
        .downcast_mut::<ModeState>()
        .expect("the modal image initializes ModeState")
        .entered_mode = 1;
    context.schedule_shutdown(None);
    Ok(())
}

fn compiled_modal_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_reaction(BindingSlotIndex::new(1), request_compiled_mode)
        .bind_reaction(BindingSlotIndex::new(2), record_mode_entry)
}

fn legacy_mode_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_reaction(BindingSlotIndex::new(1), request_legacy_mode)
        .bind_reaction(BindingSlotIndex::new(2), record_mode_entry)
}

fn forged_mode_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_reaction(BindingSlotIndex::new(1), request_forged_compiled_mode)
        .bind_reaction(BindingSlotIndex::new(2), record_mode_entry)
}

fn periodic_modal_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_reaction(BindingSlotIndex::new(1), reset_periodic_mode_once)
}

static MODAL_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    r!(0, 2),
    Some(ModeIndex::new(0)),
    None,
)];
static MODAL_ACTIONS: [ActionImage; 1] = [fixture_timer_action(0, None, r!(0, 1))];
static MODAL_REACTIONS: [ReactionImage; 2] = [
    fixture_reaction(0, 1, r!(0, 0), r!(0, 0), r!(0, 0)).with_mode_effect(CompiledModeEffectRef {
        target: ModeIndex::new(1),
        transition: TransitionKind::Reset,
    }),
    fixture_reaction(2, 2, r!(0, 0), r!(0, 0), r!(0, 1)),
];
static MODAL_MODES: [ModeImage; 2] = [
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1)),
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(2)),
];
static MODAL_SCOPES: [ScopeImage; 3] = [
    fixture_scope(None, None, r!(0, 3), r!(0, 0), r!(0, 0), r!(0, 0)),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(0)),
        r!(3, 1),
        r!(0, 0),
        r!(0, 0),
        r!(0, 0),
    ),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(1)),
        r!(4, 1),
        r!(0, 0),
        r!(0, 0),
        r!(0, 1),
    ),
];
static MODAL_REACTION_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
static MODAL_REACTION_MODES: [ModeIndex; 1] = [ModeIndex::new(1)];
static MODAL_SCOPE_DESCENDANTS: [ScopeIndex; 5] = [
    ScopeIndex::new(0),
    ScopeIndex::new(1),
    ScopeIndex::new(2),
    ScopeIndex::new(1),
    ScopeIndex::new(2),
];
static MODAL_SCOPE_STARTUPS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
    LevelReactionImage::new(0, ReactionIndex::new(1)),
    ActionIndex::new(0),
)];
static MODAL_TIMER_STARTUPS: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), 0)];
static MODAL_REQUIRED_BINDINGS: [RequiredBindingImage; 3] = [
    RequiredBindingImage::new(IdentityRange::new(18, 7), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(25, 12), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(37, 7), BindingKind::Reaction),
];

static MODAL_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "compiled/referencea-stateb-transitionc-entry",
    reactors: TinyMapView::new(&MODAL_REACTORS),
    actions: TinyMapView::new(&MODAL_ACTIONS),
    reactions: TinyMapView::new(&MODAL_REACTIONS),
    modes: TinyMapView::new(&MODAL_MODES),
    scopes: TinyMapView::new(&MODAL_SCOPES),
    reaction_triggers: &MODAL_REACTION_TRIGGERS,
    reaction_modes: &MODAL_REACTION_MODES,
    scope_descendants: &MODAL_SCOPE_DESCENDANTS,
    scope_logical_actions: &[],
    scope_startup_reactions: &MODAL_SCOPE_STARTUPS,
    timer_startup_actions: &MODAL_TIMER_STARTUPS,
    required_bindings: TinyMapView::new(&MODAL_REQUIRED_BINDINGS),
    storage_bounds: StorageBounds::new(1, 1, 2, 0, 0, 0),
    ..IMAGE
};

static PERIODIC_MODAL_ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(1),
    ActionSlotIndex::new(0),
    ActionTiming::Timer {
        period_nanos: Some(2),
    },
    r!(0, 1),
    None,
)];
static PERIODIC_MODAL_REACTIONS: [ReactionImage; 1] =
    [
        fixture_reaction(1, 1, r!(0, 0), r!(0, 0), r!(0, 1)).with_mode_effect(
            CompiledModeEffectRef {
                target: ModeIndex::new(0),
                transition: TransitionKind::Reset,
            },
        ),
    ];
static PERIODIC_MODAL_SCOPES: [ScopeImage; 3] = [
    fixture_scope(None, None, r!(0, 3), r!(0, 0), r!(0, 0), r!(0, 0)),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(0)),
        r!(3, 1),
        r!(0, 1),
        r!(0, 1),
        r!(0, 0),
    ),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(1)),
        r!(4, 1),
        r!(1, 0),
        r!(1, 0),
        r!(0, 0),
    ),
];
static PERIODIC_MODAL_STARTUPS: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), 5)];
static PERIODIC_MODAL_BINDINGS: [RequiredBindingImage; 2] = [
    RequiredBindingImage::new(IdentityRange::new(18, 7), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(25, 12), BindingKind::Reaction),
];
static PERIODIC_MODAL_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "compiled/referencea-stateb-transitionc-entry",
    reactors: TinyMapView::new(&MODAL_REACTORS),
    actions: TinyMapView::new(&PERIODIC_MODAL_ACTIONS),
    reactions: TinyMapView::new(&PERIODIC_MODAL_REACTIONS),
    modes: TinyMapView::new(&MODAL_MODES),
    scopes: TinyMapView::new(&PERIODIC_MODAL_SCOPES),
    reaction_triggers: &[LevelReactionImage::new(0, ReactionIndex::new(0))],
    reaction_modes: &[ModeIndex::new(0)],
    scope_descendants: &MODAL_SCOPE_DESCENDANTS,
    scope_logical_actions: &[ActionIndex::new(0)],
    scope_timer_startups: &PERIODIC_MODAL_STARTUPS,
    timer_startup_actions: &PERIODIC_MODAL_STARTUPS,
    required_bindings: TinyMapView::new(&PERIODIC_MODAL_BINDINGS),
    storage_bounds: StorageBounds::new(1, 1, 1, 0, 0, 0),
    ..IMAGE
};

static PERIODIC_INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

fn initialize_periodic_counter() -> CounterState {
    PERIODIC_INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    initialize_counter()
}

fn record_periodic_tag(
    context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let state = state
        .downcast_mut::<CounterState>()
        .expect("the periodic image initializes CounterState");
    state.tags.push(context.get_tag());
    std::thread::sleep(std::time::Duration::from_millis(3));
    if state.tags.len() == 3 {
        context.schedule_shutdown(None);
    }
    Ok(())
}

fn periodic_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), record_periodic_tag)
}

fn counted_periodic_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_periodic_counter)
        .bind_reaction(BindingSlotIndex::new(1), record_periodic_tag)
}

static RECURRING_ABORT_PEER_READY: AtomicBool = AtomicBool::new(false);

fn signal_recurring_abort_peer_ready(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    RECURRING_ABORT_PEER_READY.store(true, Ordering::SeqCst);
    Ok(())
}

fn panic_after_recurring_abort_peer_starts(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    while !RECURRING_ABORT_PEER_READY.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    panic!("peer scheduler panic");
}

fn recurring_abort_peer_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), signal_recurring_abort_peer_ready)
}

fn aborting_peer_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(
            BindingSlotIndex::new(1),
            panic_after_recurring_abort_peer_starts,
        )
}

fn schedule_later_overflow_timer(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    if context.get_tag() != Tag::ZERO {
        return Ok(());
    }
    let (_startup, mut timer): (ActionRef, ActionRef) = refs.actions.partition_mut()?;
    let delay = Duration::MAX - Duration::nanoseconds(1);
    context.schedule_action(&mut timer, (), Some(delay));
    Ok(())
}

fn schedule_later_overflow_timer_with_shutdown(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let tag = context.get_tag();
    schedule_later_overflow_timer(context, state, refs, mode_effect)?;
    if tag == Tag::new(Duration::MAX, 0) {
        context.schedule_shutdown(None);
    }
    Ok(())
}
static PERIODIC_ACTIONS: [ActionImage; 1] = [fixture_timer_action(0, Some(1_000_000), r!(0, 1))];
static ZERO_PERIOD_ACTIONS: [ActionImage; 1] = [fixture_timer_action(0, Some(0), r!(0, 1))];
static OVERFLOW_PERIOD_ACTIONS: [ActionImage; 1] = [fixture_timer_action(0, Some(1), r!(0, 1))];
static LATER_OVERFLOW_PERIOD_ACTIONS: [ActionImage; 2] = [
    fixture_timer_action(0, None, r!(0, 1)),
    fixture_timer_action(1, Some(1), r!(1, 1)),
];
static PERIODIC_STARTUP: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), 1_000_000)];
static OVERFLOW_PERIOD_STARTUP: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), i64::MAX as u64)];
static PERIODIC_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&PERIODIC_ACTIONS),
    timer_startup_actions: &PERIODIC_STARTUP,
    ..IMAGE
};
static ZERO_PERIOD_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&ZERO_PERIOD_ACTIONS),
    timer_startup_actions: &PERIODIC_STARTUP,
    ..IMAGE
};
static OVERFLOW_PERIOD_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&OVERFLOW_PERIOD_ACTIONS),
    timer_startup_actions: &OVERFLOW_PERIOD_STARTUP,
    ..IMAGE
};
static LATER_OVERFLOW_PERIOD_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&LATER_OVERFLOW_PERIOD_ACTIONS),
    reactions: TinyMapView::new(&COTIMED_REACTIONS),
    scopes: TinyMapView::new(&COTIMED_SCOPE),
    reaction_triggers: &COTIMED_TRIGGERS,
    reaction_actions: &[ActionIndex::new(0), ActionIndex::new(1)],
    scope_logical_actions: &COTIMED_LOGICAL_ACTIONS,
    timer_startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), 0)],
    storage_bounds: StorageBounds::new(1, 2, 3, 0, 0, 0),
    ..IMAGE
};

fn record_cotimed_timers(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let (mut first, mut second): (ActionRef, ActionRef) = refs.actions.partition_mut()?;
    let state = state
        .downcast_mut::<CounterState>()
        .expect("the cotimed image initializes CounterState");
    state.count += usize::from(context.get_action_value(&mut first).is_some());
    state.count += usize::from(context.get_action_value(&mut second).is_some());
    state.tags.push(context.get_tag());
    if state.tags.len() == 3 {
        context.schedule_shutdown(None);
    }
    Ok(())
}

fn cotimed_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), record_cotimed_timers)
}

static COTIMED_ACTIONS: [ActionImage; 2] = [
    fixture_timer_action(0, Some(1_000_000), r!(0, 1)),
    fixture_timer_action(1, Some(1_000_000), r!(1, 1)),
];
static COTIMED_REACTIONS: [ReactionImage; 1] =
    [fixture_reaction(0, 1, r!(0, 0), r!(0, 2), r!(0, 0))];
static COTIMED_TRIGGERS: [LevelReactionImage; 2] = [
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    LevelReactionImage::new(0, ReactionIndex::new(0)),
];
static COTIMED_LOGICAL_ACTIONS: [ActionIndex; 2] = [ActionIndex::new(0), ActionIndex::new(1)];
static COTIMED_SCOPE: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    r!(0, 1),
    r!(0, 2),
    r!(0, 0),
    r!(0, 0),
)];
static COTIMED_STARTUPS: [TimerStartupImage; 2] = [
    TimerStartupImage::new(ActionIndex::new(0), 1_000_000),
    TimerStartupImage::new(ActionIndex::new(1), 1_000_000),
];
static COTIMED_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&COTIMED_ACTIONS),
    reactions: TinyMapView::new(&COTIMED_REACTIONS),
    scopes: TinyMapView::new(&COTIMED_SCOPE),
    reaction_triggers: &COTIMED_TRIGGERS,
    reaction_actions: &[ActionIndex::new(0), ActionIndex::new(1)],
    scope_logical_actions: &COTIMED_LOGICAL_ACTIONS,
    timer_startup_actions: &COTIMED_STARTUPS,
    storage_bounds: StorageBounds::new(1, 2, 4, 0, 0, 0),
    ..IMAGE
};

static IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "compiled/referencecounter-stateincrement-counter",
    enclave_id: IdentityRange::new(0, 18),
    reactors: TinyMapView::new(&REACTORS),
    actions: TinyMapView::new(&ACTIONS),
    ports: TinyMapView::new(&PORTS),
    reactions: TinyMapView::new(&REACTIONS),
    modes: TinyMapView::new(&MODES),
    scopes: TinyMapView::new(&SCOPES),
    reaction_triggers: &REACTION_TRIGGERS,
    reaction_use_ports: &[],
    reaction_effect_ports: &[],
    reaction_actions: &[],
    reaction_modes: &[],
    scope_descendants: &SCOPE_DESCENDANTS,
    scope_logical_actions: &SCOPE_LOGICAL_ACTIONS,
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &SCOPE_STARTUP_REACTIONS,
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &STARTUP_ACTIONS,
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTES),
    required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
    storage_bounds: StorageBounds::new(1, 1, 1, 0, 0, 0),
};

static COALESCED_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapView::new(&COALESCED_ACTIONS),
    reaction_triggers: &COALESCED_REACTION_TRIGGERS,
    startup_actions: &COALESCED_STARTUP_ACTIONS,
    storage_bounds: StorageBounds::new(1, 2, 1, 0, 0, 0),
    ..IMAGE
};

static ROUTED_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "compiled/referencecounter-stateincrement-counterrouted-port",
    ports: TinyMapView::new(&ROUTED_PORTS),
    routes: TinyMapView::new(&ROUTED_ROUTES),
    required_bindings: TinyMapView::new(&ROUTED_REQUIRED_BINDINGS),
    ..IMAGE
};

#[test]
fn compiled_reference_executes_startup_to_shutdown() {
    let result = execute_owned(
        &IMAGE,
        reference_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::nanoseconds(6)),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        1
    );
    assert!(matches!(
        result.state::<CounterState>(StateSlotIndex::new(1)),
        Err(StateAccessError::OutOfRange { slot }) if slot == StateSlotIndex::new(1)
    ));
    assert!(matches!(
        result.state::<u32>(StateSlotIndex::new(0)),
        Err(StateAccessError::TypeMismatch { slot, expected, found })
            if slot == StateSlotIndex::new(0)
                && expected == std::any::type_name::<u32>()
                && found == std::any::type_name::<CounterState>()
    ));
    assert_eq!(result.final_tag(), Tag::new(Duration::nanoseconds(5), 0));
}

#[test]
fn compiled_reference_terminal_only_execution_returns_never() {
    let result = execute_owned(
        &IMAGE,
        reference_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::ZERO),
    )
    .unwrap();

    assert_eq!(result.final_tag(), Tag::NEVER);
}

#[test]
fn compiled_reference_final_tag_includes_work_coalesced_with_shutdown() {
    let result = execute_owned(
        &COALESCED_IMAGE,
        reference_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::nanoseconds(1)),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        2
    );
    assert_eq!(result.final_tag(), Tag::new(Duration::nanoseconds(1), 0));
}

#[test]
fn compiled_reference_returns_typed_image_validation_error() {
    fn validate_local_image(image: EnclaveImage<'static>) -> ExecuteOwnedError<'static> {
        match execute_owned(&image, EnclaveBindings::new(), Config::default()) {
            Err(error) => error,
            Ok(_) => panic!("invalid image must fail validation"),
        }
    }

    let invalid_image = EnclaveImage {
        enclave_id: IdentityRange::new(u32::MAX, 1),
        ..IMAGE
    };
    let error = validate_local_image(invalid_image);

    assert!(matches!(
        error,
        ExecuteOwnedError::ImageValidation(ImageValidationError::IdentityRangeInvalid {
            table: "image",
            index: 0,
            field: "enclave_id",
            ..
        })
    ));
}

#[test]
fn compiled_reference_rejects_routes_until_route_execution_is_supported() {
    let route_free_image = EnclaveImage {
        routes: TinyMapView::new(&ROUTES),
        ..ROUTED_IMAGE
    };
    execute_owned(
        &route_free_image,
        reference_bindings().bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new()),
        Config::default().with_fast_forward(true),
    )
    .expect("the same image must execute when its route table is empty");

    for direction in [RouteDirection::Inbound, RouteDirection::Outbound] {
        let routes = [RouteImage::new(
            IdentityRange::new(0, 18),
            PortIndex::new(0),
            direction,
            TimingDomain::Logical,
            0,
        )];
        let routed = EnclaveImage {
            routes: TinyMapView::new(&routes),
            ..ROUTED_IMAGE
        };
        match execute_owned(
            &routed,
            reference_bindings().bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new()),
            Config::default().with_fast_forward(true),
        ) {
            Ok(_) => panic!("{direction:?} routes must require a route-capable executor"),
            Err(ExecuteOwnedError::RoutesUnsupported { count: 1 }) => {}
            Err(error) => panic!("unexpected route rejection: {error}"),
        }
    }
}

/// Source state used to verify the shared Federate origin.
#[derive(Debug)]
struct RoutedSourceState {
    /// Origin observed from the source reaction context.
    origin: Option<Instant>,
}

fn initialize_routed_source() -> RoutedSourceState {
    RoutedSourceState { origin: None }
}

fn emit_routed_value(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let state = state
        .downcast_mut::<RoutedSourceState>()
        .expect("the source state binding initializes RoutedSourceState");
    state.origin = Some(context.get_start_time());
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(42);
    Ok(())
}

/// Destination state used to verify typed route delivery and the shared origin.
#[derive(Debug)]
struct RoutedSinkState {
    /// Typed values observed by the destination reaction.
    values: Vec<u32>,
    /// Origin observed from the destination reaction context.
    origin: Option<Instant>,
}

fn initialize_routed_sink() -> RoutedSinkState {
    RoutedSinkState {
        values: Vec::new(),
        origin: None,
    }
}

fn receive_routed_value(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let input: InputRef<u32> = refs.ports.partition()?;
    let state = state
        .downcast_mut::<RoutedSinkState>()
        .expect("the sink state binding initializes RoutedSinkState");
    state.values.push(
        *input
            .as_ref()
            .expect("the inbound route must set the triggering port"),
    );
    state.origin = Some(context.get_start_time());
    Ok(())
}

fn source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn route_boundary() -> BoundaryId<'static> {
    BoundaryId::new("pipe")
}

static ROUTED_SOURCE_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    r!(0, 0),
    None,
    None,
)];
static ROUTED_SOURCE_ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    r!(0, 1),
    None,
)];
static ROUTED_SOURCE_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 0),
    BindingSlotIndex::new(2),
)];
static ROUTED_SOURCE_REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
)];
static ROUTED_SOURCE_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
static ROUTED_SOURCE_EFFECT_PORTS: [PortIndex; 1] = [PortIndex::new(0)];
static ROUTED_SOURCE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
static ROUTED_SOURCE_TIMER_STARTUPS: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), 0)];
static ROUTED_SOURCE_SCOPES: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    r!(0, 1),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
)];
static ROUTED_SOURCE_ROUTES: [RouteImage; 1] = [RouteImage::new(
    IdentityRange::new(9, 4),
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    1_000_000,
)];
static ROUTED_SOURCE_BINDINGS: [RequiredBindingImage; 3] = [
    RequiredBindingImage::new(IdentityRange::new(5, 1), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(6, 1), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(7, 1), BindingKind::Port),
];
static ROUTED_SOURCE_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "alphaabcxpipe",
    enclave_id: IdentityRange::new(0, 5),
    reactors: TinyMapView::new(&ROUTED_SOURCE_REACTORS),
    actions: TinyMapView::new(&ROUTED_SOURCE_ACTIONS),
    ports: TinyMapView::new(&ROUTED_SOURCE_PORTS),
    reactions: TinyMapView::new(&ROUTED_SOURCE_REACTIONS),
    modes: TinyMapView::new(&[]),
    scopes: TinyMapView::new(&ROUTED_SOURCE_SCOPES),
    reaction_triggers: &ROUTED_SOURCE_TRIGGERS,
    reaction_use_ports: &[],
    reaction_effect_ports: &ROUTED_SOURCE_EFFECT_PORTS,
    reaction_actions: &[],
    reaction_modes: &[],
    scope_descendants: &ROUTED_SOURCE_DESCENDANTS,
    scope_logical_actions: &[],
    scope_timer_startups: &ROUTED_SOURCE_TIMER_STARTUPS,
    scope_reset_reactions: &[],
    scope_startup_reactions: &[],
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &ROUTED_SOURCE_TIMER_STARTUPS,
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTED_SOURCE_ROUTES),
    required_bindings: TinyMapView::new(&ROUTED_SOURCE_BINDINGS),
    storage_bounds: StorageBounds::new(1, 1, 8, 0, 0, 0),
};

static ROUTED_SINK_REACTORS: [ReactorImage; 1] = ROUTED_SOURCE_REACTORS;
static ROUTED_SINK_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 1),
    BindingSlotIndex::new(2),
)];
static ROUTED_SINK_REACTIONS: [ReactionImage; 1] =
    [fixture_reaction(0, 1, r!(0, 1), r!(0, 0), r!(0, 0))];
static ROUTED_SINK_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
static ROUTED_SINK_USE_PORTS: [PortIndex; 1] = [PortIndex::new(0)];
static ROUTED_SINK_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
static ROUTED_SINK_SCOPES: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
)];
static ROUTED_SINK_ROUTES: [RouteImage; 1] = [RouteImage::new(
    IdentityRange::new(7, 4),
    PortIndex::new(0),
    RouteDirection::Inbound,
    TimingDomain::Logical,
    1_000_000,
)];
static ROUTED_SINK_BINDINGS: [RequiredBindingImage; 3] = [
    RequiredBindingImage::new(IdentityRange::new(4, 1), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(5, 1), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(6, 1), BindingKind::Port),
];
static ROUTED_SINK_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "betaabcpipe",
    enclave_id: IdentityRange::new(0, 4),
    reactors: TinyMapView::new(&ROUTED_SINK_REACTORS),
    actions: TinyMapView::new(&[]),
    ports: TinyMapView::new(&ROUTED_SINK_PORTS),
    reactions: TinyMapView::new(&ROUTED_SINK_REACTIONS),
    modes: TinyMapView::new(&[]),
    scopes: TinyMapView::new(&ROUTED_SINK_SCOPES),
    reaction_triggers: &ROUTED_SINK_TRIGGERS,
    reaction_use_ports: &ROUTED_SINK_USE_PORTS,
    reaction_effect_ports: &[],
    reaction_actions: &[],
    reaction_modes: &[],
    scope_descendants: &ROUTED_SINK_DESCENDANTS,
    scope_logical_actions: &[],
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &[],
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &[],
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTED_SINK_ROUTES),
    required_bindings: TinyMapView::new(&ROUTED_SINK_BINDINGS),
    storage_bounds: StorageBounds::new(1, 0, 8, 0, 0, 0),
};

static ROUTED_FEDERATES: [FederateImage; 1] = [FederateImage::new(
    IdentityRange::new(0, 4),
    IdentityRange::new(4, 6),
    IdentityRange::new(10, 7),
    r!(0, 2),
)];
static ROUTED_ENCLAVES: [EnclaveImage<'static>; 2] = [ROUTED_SOURCE_IMAGE, ROUTED_SINK_IMAGE];
static ROUTED_FEDERATE_MEMBERS: [FederateIndex; 1] = [FederateIndex::new(0)];
static ROUTED_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    identity_data: "hosttargetruntime",
    federation: GlobalFederationImage::new(&ROUTED_FEDERATE_MEMBERS, &[]),
    federates: TinyMapView::new(&ROUTED_FEDERATES),
    enclaves: TinyMapView::new(&ROUTED_ENCLAVES),
    coordination: CoordinationProjection::Local,
};

#[derive(Debug)]
struct MultiSourceState {
    value: u32,
    delay: bool,
}

fn initialize_fast_source() -> MultiSourceState {
    MultiSourceState {
        value: 1,
        delay: false,
    }
}

fn initialize_slow_source() -> MultiSourceState {
    MultiSourceState {
        value: 2,
        delay: true,
    }
}

fn emit_multi_source_value(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let state = state.downcast_mut::<MultiSourceState>().unwrap();
    if state.delay {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(state.value);
    context.schedule_shutdown(Some(Duration::ZERO));
    Ok(())
}

#[derive(Debug, Default)]
struct MultiSinkState(Vec<(u32, u32)>);

fn receive_both_source_values(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    assert_eq!(context.enclave_id(), EnclaveKey::from(2));
    let (left, right): (InputRef<u32>, InputRef<u32>) = refs.ports.partition()?;
    state.downcast_mut::<MultiSinkState>().unwrap().0.push((
        *left
            .as_ref()
            .expect("left source must be admitted at this tag"),
        *right
            .as_ref()
            .expect("right source must be admitted at this tag"),
    ));
    context.schedule_shutdown(Some(Duration::ZERO));
    Ok(())
}

fn multi_source_bindings(initializer: fn() -> MultiSourceState) -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initializer)
        .bind_reaction(BindingSlotIndex::new(1), emit_multi_source_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn multi_sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), MultiSinkState::default)
        .bind_reaction(BindingSlotIndex::new(1), receive_both_source_values)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
        .bind_port(BindingSlotIndex::new(3), PayloadType::<u32>::new())
}

static MULTI_LEFT_ROUTES: [RouteImage; 1] = [RouteImage::new(
    IdentityRange::new(9, 4),
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];
static MULTI_RIGHT_ROUTES: [RouteImage; 1] = [RouteImage::new(
    IdentityRange::new(9, 5),
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];

const fn multi_source_image(
    identity_data: &'static str,
    routes: &'static [RouteImage],
) -> EnclaveImage<'static> {
    EnclaveImage {
        identity_data,
        routes: TinyMapView::new(routes),
        ..ROUTED_SOURCE_IMAGE
    }
}

static MULTI_SINK_PORTS: [PortImage; 2] = [
    PortImage::new(ScopeIndex::new(0), r!(0, 1), BindingSlotIndex::new(2)),
    PortImage::new(ScopeIndex::new(0), r!(1, 1), BindingSlotIndex::new(3)),
];
static MULTI_SINK_TRIGGERS: [LevelReactionImage; 2] = [
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    LevelReactionImage::new(0, ReactionIndex::new(0)),
];
static MULTI_SINK_REACTIONS: [ReactionImage; 1] =
    [fixture_reaction(0, 1, r!(0, 2), r!(0, 0), r!(0, 0))];
static MULTI_SINK_USE_PORTS: [PortIndex; 2] = [PortIndex::new(0), PortIndex::new(1)];
static MULTI_SINK_ROUTES: [RouteImage; 2] = [
    RouteImage::new(
        IdentityRange::new(8, 4),
        PortIndex::new(0),
        RouteDirection::Inbound,
        TimingDomain::Logical,
        0,
    ),
    RouteImage::new(
        IdentityRange::new(12, 5),
        PortIndex::new(1),
        RouteDirection::Inbound,
        TimingDomain::Logical,
        0,
    ),
];
static MULTI_SINK_BINDINGS: [RequiredBindingImage; 4] = [
    RequiredBindingImage::new(IdentityRange::new(4, 1), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(5, 1), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(6, 1), BindingKind::Port),
    RequiredBindingImage::new(IdentityRange::new(7, 1), BindingKind::Port),
];
static MULTI_SINK_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "sinkabcdleftright",
    ports: TinyMapView::new(&MULTI_SINK_PORTS),
    reactions: TinyMapView::new(&MULTI_SINK_REACTIONS),
    reaction_triggers: &MULTI_SINK_TRIGGERS,
    reaction_use_ports: &MULTI_SINK_USE_PORTS,
    routes: TinyMapView::new(&MULTI_SINK_ROUTES),
    required_bindings: TinyMapView::new(&MULTI_SINK_BINDINGS),
    ..ROUTED_SINK_IMAGE
};

static MULTI_ENCLAVES: [EnclaveImage<'static>; 3] = [
    multi_source_image("alphaabcxleft", &MULTI_LEFT_ROUTES),
    multi_source_image("gammaabcxright", &MULTI_RIGHT_ROUTES),
    MULTI_SINK_IMAGE,
];
static MULTI_FEDERATES: [FederateImage; 1] = [FederateImage::new(
    IdentityRange::new(0, 4),
    IdentityRange::new(4, 6),
    IdentityRange::new(10, 7),
    r!(0, 3),
)];
static MULTI_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    identity_data: "hosttargetruntime",
    federation: GlobalFederationImage::new(&ROUTED_FEDERATE_MEMBERS, &[]),
    federates: TinyMapView::new(&MULTI_FEDERATES),
    enclaves: TinyMapView::new(&MULTI_ENCLAVES),
    coordination: CoordinationProjection::Local,
};

#[test]
fn owned_federate_coordinates_multiple_same_tag_sources_before_destination_execution() {
    let result = execute_owned_federate(
        &MULTI_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(
                EnclaveIndex::new(0),
                multi_source_bindings(initialize_fast_source),
            )
            .bind_enclave(
                EnclaveIndex::new(1),
                multi_source_bindings(initialize_slow_source),
            )
            .bind_enclave(EnclaveIndex::new(2), multi_sink_bindings())
            .bind_route(
                BoundaryId::new("left"),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            )
            .bind_route(
                BoundaryId::new("right"),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default().with_fast_forward(true),
    )
    .unwrap();

    assert_eq!(
        result
            .enclave(EnclaveIndex::new(2))
            .unwrap()
            .state::<MultiSinkState>(StateSlotIndex::new(0))
            .unwrap()
            .0,
        [(1, 2)]
    );
}

#[test]
fn owned_federate_routes_typed_values_and_shares_one_origin() {
    for fast_forward in [true, false] {
        let result = bounded(move || {
            let boundary = String::from("pipe");
            execute_owned_federate(
                &ROUTED_DEPLOYMENT,
                FederateIndex::new(0),
                FederateBindings::new()
                    .bind_enclave(EnclaveIndex::new(0), source_bindings())
                    .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                    .bind_route(
                        BoundaryId::new(&boundary),
                        PayloadType::<u32>::new(),
                        PayloadType::<u32>::new(),
                    ),
                Config::default().with_fast_forward(fast_forward),
            )
            .unwrap()
        });

        let source = result
            .enclave(EnclaveIndex::new(0))
            .expect("source result must retain its canonical Enclave index");
        let sink = result
            .enclave(EnclaveIndex::new(1))
            .expect("sink result must retain its canonical Enclave index");
        let source_state = source
            .state::<RoutedSourceState>(StateSlotIndex::new(0))
            .unwrap();
        let sink_state = sink
            .state::<RoutedSinkState>(StateSlotIndex::new(0))
            .unwrap();

        assert_eq!(sink_state.values, [42]);
        assert_eq!(source.final_tag(), Tag::new(Duration::ZERO, usize::MAX));
        assert_eq!(sink.final_tag(), Tag::new(Duration::milliseconds(1), 0));
        assert_eq!(source_state.origin, Some(result.origin()));
        assert_eq!(sink_state.origin, Some(result.origin()));
    }
}

fn bounded<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || tx.send(run()).unwrap());
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("owned Federate execution must complete within one second");
    worker.join().unwrap();
    result
}

#[test]
fn owned_federate_quiesces_when_a_source_emits_no_route_value() {
    let result = bounded(|| {
        execute_owned_federate(
            &ROUTED_DEPLOYMENT,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(
                    EnclaveIndex::new(0),
                    routed_reaction_bindings(|context, _, _, _| {
                        context.schedule_shutdown(Some(Duration::ZERO));
                        Ok(())
                    }),
                )
                .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                .bind_route(
                    route_boundary(),
                    PayloadType::<u32>::new(),
                    PayloadType::<u32>::new(),
                ),
            Config::default().with_fast_forward(true),
        )
        .unwrap()
    });
    assert!(result
        .enclave(EnclaveIndex::new(1))
        .unwrap()
        .state::<RoutedSinkState>(StateSlotIndex::new(0))
        .unwrap()
        .values
        .is_empty());
}

#[test]
fn owned_federate_quiesces_a_positive_delay_route_cycle() {
    bounded(|| {
        let actions = [ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(0),
            ActionTiming::Timer { period_nanos: None },
            r!(0, 1),
            None,
        )];
        let routes = [
            RouteImage::new(
                IdentityRange::new(9, 4),
                PortIndex::new(0),
                RouteDirection::Inbound,
                TimingDomain::Logical,
                1,
            ),
            RouteImage::new(
                IdentityRange::new(9, 4),
                PortIndex::new(0),
                RouteDirection::Outbound,
                TimingDomain::Logical,
                1,
            ),
        ];
        let enclaves = [EnclaveImage {
            actions: TinyMapView::new(&actions),
            routes: TinyMapView::new(&routes),
            ..ROUTED_SOURCE_IMAGE
        }];
        let federates = [FederateImage::new(
            IdentityRange::new(0, 4),
            IdentityRange::new(4, 6),
            IdentityRange::new(10, 7),
            r!(0, 1),
        )];
        let deployment = CompiledDeploymentImage {
            federates: TinyMapView::new(&federates),
            enclaves: TinyMapView::new(&enclaves),
            ..ROUTED_DEPLOYMENT
        };
        execute_owned_federate(
            &deployment,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(
                    EnclaveIndex::new(0),
                    routed_reaction_bindings(|_, _, _, _| Ok(())),
                )
                .bind_route(
                    route_boundary(),
                    PayloadType::<u32>::new(),
                    PayloadType::<u32>::new(),
                ),
            Config::default().with_fast_forward(true),
        )
        .unwrap();
    });
}

static ROUTED_INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

fn initialize_counted_routed_source() -> RoutedSourceState {
    ROUTED_INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
    initialize_routed_source()
}

fn counted_source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counted_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn wrong_sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u64>::new())
}

#[test]
fn owned_federate_preflight_rejects_before_initializers() {
    ROUTED_INITIALIZATIONS.store(0, Ordering::SeqCst);
    let error = execute_owned_federate(
        &ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::MissingEnclaveBinding { enclave }
            if enclave == EnclaveIndex::new(1)
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let unpaired_enclaves = [ROUTED_SOURCE_IMAGE];
    let unpaired_federates = [FederateImage::new(
        IdentityRange::new(0, 4),
        IdentityRange::new(4, 6),
        IdentityRange::new(10, 7),
        r!(0, 1),
    )];
    let unpaired = CompiledDeploymentImage {
        federates: TinyMapView::new(&unpaired_federates),
        enclaves: TinyMapView::new(&unpaired_enclaves),
        ..ROUTED_DEPLOYMENT
    };
    let error = execute_owned_federate(
        &unpaired,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ImageValidation { .. }
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let error = execute_owned_federate(
        &ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(EnclaveIndex::new(0), counted_source_bindings())
            .bind_enclave(EnclaveIndex::new(1), wrong_sink_bindings())
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
            direction: RouteDirection::Inbound,
            enclave,
            ..
        } if enclave == EnclaveIndex::new(1)
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);

    let (source, bindings, source_identity) = {
        #[derive(Clone, Debug)]
        struct Collision;
        (
            EnclaveBindings::new()
                .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
                .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
                .bind_port(BindingSlotIndex::new(2), PayloadType::<Collision>::new()),
            FederateBindings::new().bind_route(
                route_boundary(),
                PayloadType::<Collision>::new(),
                PayloadType::<Collision>::new(),
            ),
            (
                std::any::TypeId::of::<Collision>(),
                std::any::type_name::<Collision>(),
            ),
        )
    };
    let sink = {
        #[derive(Debug)]
        struct Collision;
        assert_eq!(source_identity.1, std::any::type_name::<Collision>());
        assert_ne!(source_identity.0, std::any::TypeId::of::<Collision>());
        EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
            .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
            .bind_port(BindingSlotIndex::new(2), PayloadType::<Collision>::new())
    };
    let error = execute_owned_federate(
        &ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        bindings
            .bind_enclave(EnclaveIndex::new(0), source)
            .bind_enclave(EnclaveIndex::new(1), sink),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::RoutePayloadTypeMismatch {
            direction: RouteDirection::Inbound,
            enclave,
            ..
        } if enclave == EnclaveIndex::new(1)
    ));

    let cross_federates = [
        FederateImage::new(
            IdentityRange::new(0, 1),
            IdentityRange::new(1, 1),
            IdentityRange::new(2, 1),
            r!(0, 1),
        ),
        FederateImage::new(
            IdentityRange::new(3, 1),
            IdentityRange::new(4, 1),
            IdentityRange::new(5, 1),
            r!(1, 1),
        ),
    ];
    let cross_members = [FederateIndex::new(0), FederateIndex::new(1)];
    let cross = CompiledDeploymentImage {
        identity_data: "atrbtr",
        federation: GlobalFederationImage::new(&cross_members, &[]),
        federates: TinyMapView::new(&cross_federates),
        ..ROUTED_DEPLOYMENT
    };
    let error = execute_owned_federate(
        &cross,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), counted_source_bindings()),
        Config::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ExecuteOwnedFederateError::CrossFederateRoute { .. }
    ));
    assert_eq!(ROUTED_INITIALIZATIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn owned_federate_rejects_enclave_without_root_reactor() {
    let rootless_enclaves = [EnclaveImage {
        identity_data: "rootless",
        enclave_id: IdentityRange::new(0, 8),
        reactors: TinyMapView::new(&[]),
        actions: TinyMapView::new(&[]),
        ports: TinyMapView::new(&[]),
        reactions: TinyMapView::new(&[]),
        modes: TinyMapView::new(&[]),
        scopes: TinyMapView::new(&[]),
        reaction_triggers: &[],
        reaction_use_ports: &[],
        reaction_effect_ports: &[],
        reaction_actions: &[],
        reaction_modes: &[],
        scope_descendants: &[],
        scope_logical_actions: &[],
        scope_timer_startups: &[],
        scope_reset_reactions: &[],
        scope_startup_reactions: &[],
        scope_shutdown_reactions: &[],
        startup_actions: &[],
        timer_startup_actions: &[],
        shutdown_reactions: &[],
        shutdown_actions: &[],
        routes: TinyMapView::new(&[]),
        required_bindings: TinyMapView::new(&[]),
        storage_bounds: StorageBounds::new(0, 0, 0, 0, 0, 0),
    }];
    let rootless_federates = [FederateImage::new(
        IdentityRange::new(0, 4),
        IdentityRange::new(4, 6),
        IdentityRange::new(10, 7),
        r!(0, 1),
    )];
    let members = [FederateIndex::new(0)];
    let deployment = CompiledDeploymentImage {
        identity_data: "hosttargetruntime",
        federation: GlobalFederationImage::new(&members, &[]),
        federates: TinyMapView::new(&rootless_federates),
        enclaves: TinyMapView::new(&rootless_enclaves),
        coordination: CoordinationProjection::Local,
    };

    let error = execute_owned_federate(
        &deployment,
        FederateIndex::new(0),
        FederateBindings::new().bind_enclave(EnclaveIndex::new(0), EnclaveBindings::new()),
        Config::default(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ImageValidation { message }
            if message == "image[0].root_reactor references missing reactors[0]"
    ));
}

fn panic_on_routed_value(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    panic!("routed sink panic")
}

fn panicking_sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), panic_on_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

static COMPETING_PANIC_READY: AtomicBool = AtomicBool::new(false);

fn synchronized_route_source(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    while !COMPETING_PANIC_READY.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(42);
    Ok(())
}

fn competing_panic_after_route_failure(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    COMPETING_PANIC_READY.store(true, Ordering::SeqCst);
    assert!(!context.schedule_external(AsyncEvent::Shutdown {
        delay: Duration::ZERO,
    }));
    panic!("competing scheduler panic");
}

type RoutedReaction = fn(
    &mut Context,
    &mut dyn ReactorData,
    ReactionRefs<'_>,
    Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError>;

fn routed_reaction_bindings(reaction: RoutedReaction) -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), reaction)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

#[test]
fn owned_federate_panic_requests_bounded_shutdown_and_joins() {
    let started = Instant::now();
    let error = execute_owned_federate(
        &ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(EnclaveIndex::new(0), source_bindings())
            .bind_enclave(EnclaveIndex::new(1), panicking_sink_bindings())
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ThreadPanicked { enclave, ref message }
            if enclave == EnclaveIndex::new(1) && message == "routed sink panic"
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
#[cfg_attr(
    miri,
    ignore = "the bounded hang regression requires subprocess support"
)]
fn owned_federate_abort_stops_peer_with_recurring_internal_work() {
    let executable = std::env::current_exe().expect("integration test executable is available");
    let mut child = std::process::Command::new(executable)
        .args([
            "--exact",
            "owned_federate_abort_stops_peer_with_recurring_internal_work_child",
            "--nocapture",
        ])
        .env("BOOMERANG_RECURRING_ABORT_CHILD", "1")
        .spawn()
        .expect("recurring-abort child process starts");
    let deadline = Instant::now() + std::time::Duration::from_secs(2);

    loop {
        if let Some(status) = child
            .try_wait()
            .expect("recurring-abort child can be polled")
        {
            assert!(
                status.success(),
                "recurring-abort child failed with {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .expect("timed-out recurring-abort child is killed");
            child
                .wait()
                .expect("killed recurring-abort child is reaped");
            panic!("Federate abort did not stop its recurring-work scheduler within two seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn owned_federate_abort_stops_peer_with_recurring_internal_work_child() {
    if std::env::var_os("BOOMERANG_RECURRING_ABORT_CHILD").is_none() {
        return;
    }

    let recurring = EnclaveImage {
        identity_data: "compiled/abortpeercounter-stateincrement-counter",
        actions: TinyMapView::new(&PERIODIC_ACTIONS),
        timer_startup_actions: &PERIODIC_STARTUP,
        ..IMAGE
    };
    let panicking = EnclaveImage {
        identity_data: "compiled/panicpeercounter-stateincrement-counter",
        ..IMAGE
    };
    let enclaves = [recurring, panicking];
    let federates = [FederateImage::new(
        IdentityRange::new(0, 4),
        IdentityRange::new(4, 6),
        IdentityRange::new(10, 7),
        r!(0, 2),
    )];
    let members = [FederateIndex::new(0)];
    let deployment = CompiledDeploymentImage {
        identity_data: "hosttargetruntime",
        federation: GlobalFederationImage::new(&members, &[]),
        federates: TinyMapView::new(&federates),
        enclaves: TinyMapView::new(&enclaves),
        coordination: CoordinationProjection::Local,
    };

    for keep_alive in [false, true] {
        RECURRING_ABORT_PEER_READY.store(false, Ordering::SeqCst);
        let error = execute_owned_federate(
            &deployment,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), recurring_abort_peer_bindings())
                .bind_enclave(EnclaveIndex::new(1), aborting_peer_bindings()),
            Config::default()
                .with_fast_forward(true)
                .with_keep_alive(keep_alive),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ExecuteOwnedFederateError::ThreadPanicked { enclave, ref message }
                if enclave == EnclaveIndex::new(1) && message == "peer scheduler panic"
        ));
    }
}

#[test]
fn owned_federate_retains_route_failure_before_competing_scheduler_panic() {
    COMPETING_PANIC_READY.store(false, Ordering::SeqCst);
    let outbound = [RouteImage::new(
        IdentityRange::new(9, 4),
        PortIndex::new(0),
        RouteDirection::Outbound,
        TimingDomain::Physical,
        0,
    )];
    let inbound = [RouteImage::new(
        IdentityRange::new(9, 4),
        PortIndex::new(0),
        RouteDirection::Inbound,
        TimingDomain::Physical,
        0,
    )];
    let source = EnclaveImage {
        routes: TinyMapView::new(&outbound),
        ..ROUTED_SOURCE_IMAGE
    };
    let destination = EnclaveImage {
        identity_data: "deltaabcxpipe",
        routes: TinyMapView::new(&inbound),
        storage_bounds: StorageBounds::new(1, 1, 0, 0, 0, 0),
        ..ROUTED_SOURCE_IMAGE
    };
    let enclaves = [source, destination];
    let deployment = CompiledDeploymentImage {
        enclaves: TinyMapView::new(&enclaves),
        ..ROUTED_DEPLOYMENT
    };
    let started = Instant::now();
    let error = execute_owned_federate(
        &deployment,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(
                EnclaveIndex::new(0),
                routed_reaction_bindings(synchronized_route_source),
            )
            .bind_enclave(
                EnclaveIndex::new(1),
                routed_reaction_bindings(competing_panic_after_route_failure),
            )
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecuteOwnedFederateError::EnclaveExecution {
            enclave,
            source: OwnedStorageError::OutboundRouteChannelFull { destination, .. },
        } if enclave == EnclaveIndex::new(0) && destination == EnclaveIndex::new(1)
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn compiled_mode_transition_uses_canonical_mode_index() {
    let result = execute_owned(
        &MODAL_IMAGE,
        compiled_modal_bindings(),
        Config::default().with_fast_forward(true),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<ModeState>(StateSlotIndex::new(0))
            .unwrap()
            .entered_mode,
        1
    );
}

#[test]
fn compiled_periodic_timer_same_mode_reset_has_one_restarted_stream() {
    let result = execute_owned(
        &PERIODIC_MODAL_IMAGE,
        periodic_modal_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::nanoseconds(13)),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<ModeState>(StateSlotIndex::new(0))
            .unwrap()
            .periodic_tags,
        [
            Tag::new(Duration::nanoseconds(5), 0),
            Tag::new(Duration::nanoseconds(10), 0),
            Tag::new(Duration::nanoseconds(12), 0),
        ]
    );
}

#[test]
fn legacy_live_mode_key_transition_is_still_rejected() {
    let error = match execute_owned(
        &MODAL_IMAGE,
        legacy_mode_bindings(),
        Config::default().with_fast_forward(true),
    ) {
        Ok(_) => panic!("legacy live mode identity must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ExecuteOwnedError::Storage(OwnedStorageError::LegacyModeTransition { .. })
    ));
}

#[test]
fn compiled_mode_transition_must_match_the_declared_effect() {
    let error = match execute_owned(
        &MODAL_IMAGE,
        forged_mode_bindings(),
        Config::default().with_fast_forward(true),
    ) {
        Ok(_) => panic!("a compiled reaction must not forge a different mode effect"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ExecuteOwnedError::Storage(OwnedStorageError::CompiledModeTransitionMismatch {
            reaction,
            declared: Some(declared),
            requested,
        }) if reaction == ReactionIndex::new(0)
            && declared == (CompiledModeEffectRef {
                target: ModeIndex::new(1),
                transition: TransitionKind::Reset,
            })
            && requested == (CompiledModeEffectRef {
                target: ModeIndex::new(0),
                transition: TransitionKind::History,
            })
    ));
}

#[test]
fn compiled_periodic_timer_recurrence_uses_prior_logical_tag() {
    let result = execute_owned(&PERIODIC_IMAGE, periodic_bindings(), Config::default()).unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .tags,
        [
            Tag::new(Duration::milliseconds(1), 0),
            Tag::new(Duration::milliseconds(2), 0),
            Tag::new(Duration::milliseconds(3), 0),
        ]
    );
}

#[test]
fn compiled_periodic_successor_at_shutdown_is_not_enqueued() {
    let result = execute_owned(
        &PERIODIC_IMAGE,
        periodic_bindings(),
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::milliseconds(2)),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .tags,
        [Tag::new(Duration::milliseconds(1), 0)]
    );

    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(
            BindingSlotIndex::new(1),
            schedule_later_overflow_timer_with_shutdown,
        );
    let result = execute_owned(
        &LATER_OVERFLOW_PERIOD_IMAGE,
        bindings,
        Config::default().with_fast_forward(true),
    )
    .expect("overflowing periodic successor after shutdown must be suppressed");
    assert_eq!(result.final_tag(), Tag::new(Duration::MAX, 0));
}

#[test]
fn compiled_cotimed_periodic_timers_each_recur() {
    let result = execute_owned(
        &COTIMED_IMAGE,
        cotimed_bindings(),
        Config::default().with_fast_forward(true),
    )
    .unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        6
    );
}

#[test]
fn compiled_periodic_timer_rejects_zero_period_before_state_initialization() {
    PERIODIC_INITIALIZATIONS.store(0, Ordering::SeqCst);
    let error = match execute_owned(
        &ZERO_PERIOD_IMAGE,
        counted_periodic_bindings(),
        Config::default(),
    ) {
        Ok(_) => panic!("zero-period timer must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ExecuteOwnedError::Storage(OwnedStorageError::ZeroPeriodTimer { slot })
            if slot == ActionSlotIndex::new(0)
    ));
    assert_eq!(PERIODIC_INITIALIZATIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn compiled_periodic_timer_rejects_first_successor_overflow_before_state_initialization() {
    PERIODIC_INITIALIZATIONS.store(0, Ordering::SeqCst);
    let error = match execute_owned(
        &OVERFLOW_PERIOD_IMAGE,
        counted_periodic_bindings(),
        Config::default(),
    ) {
        Ok(_) => panic!("overflowing periodic successor must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ExecuteOwnedError::Storage(OwnedStorageError::PeriodicTimerTagOverflow {
            slot,
            startup_nanos,
            period_nanos: 1,
        }) if slot == ActionSlotIndex::new(0) && startup_nanos == i64::MAX as u64
    ));
    assert_eq!(PERIODIC_INITIALIZATIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn compiled_periodic_timer_reports_later_recurrence_overflow() {
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), schedule_later_overflow_timer);
    let error = execute_owned(
        &LATER_OVERFLOW_PERIOD_IMAGE,
        bindings,
        Config::default().with_fast_forward(true),
    )
    .err()
    .expect("later periodic recurrence overflow must return an error");
    assert!(matches!(
        error,
        ExecuteOwnedError::Coordination(RuntimeError::LogicalTimeOverflow { tag, period })
            if tag == Tag::new(Duration::MAX, 0) && period == Duration::nanoseconds(1)
    ));
}
