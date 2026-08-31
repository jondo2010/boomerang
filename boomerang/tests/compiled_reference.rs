//! Exercises the host-runtime seam from a compiled image and direct bindings through owned storage and scheduling to typed results, excluding compiler lowering and live-graph construction.

use std::sync::atomic::{AtomicUsize, Ordering};

use boomerang::runtime::{
    execute_owned,
    image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
        EnclaveImage, IdentityRange, ImageValidationError, LevelReactionImage,
        LifecycleReactionImage, ModeImage, ModeIndex, PortImage, PortIndex, ReactionImage,
        ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage, RouteDirection,
        RouteImage, ScopeImage, ScopeIndex, StateSlotIndex, StorageBounds, TableRange,
        TimerStartupImage, TimingDomain, TinyMapView,
    },
    ActionKey, ActionRef, AsyncEvent, AsyncEventTarget, CommonContext, CompiledModeEffectRef,
    Config, Context, Duration, ExecuteOwnedError, InputRef, ModeEffectRef, ModeKey, OwnedBindings,
    OwnedStorageError, PayloadType, PortKey, ReactionBindingError, ReactionRefs, ReactorData,
    StateAccessError, Tag, TransitionKind,
};

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
) -> Result<(), ReactionBindingError> {
    state
        .downcast_mut::<CounterState>()
        .expect("the image's state binding initializes CounterState")
        .count += 1;
    Ok(())
}

fn reference_bindings() -> OwnedBindings {
    OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), increment_counter)
}

static REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    TableRange::new(0, 0),
    None,
    None,
)];
static ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    TableRange::new(0, 1),
    None,
)];
static COALESCED_ACTIONS: [ActionImage; 2] = [
    ACTIONS[0],
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Timer { period_nanos: None },
        TableRange::new(1, 1),
        None,
    ),
];
static PORTS: [boomerang::runtime::image::PortImage; 0] = [];
static REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    TableRange::new(0, 0),
    TableRange::new(0, 0),
    TableRange::new(0, 0),
    TableRange::new(0, 0),
)];
static MODES: [boomerang::runtime::image::ModeImage; 0] = [];
static SCOPES: [ScopeImage; 1] = [ScopeImage::new(
    None,
    ReactorIndex::new(0),
    None,
    TableRange::new(0, 1),
    TableRange::new(0, 1),
    TableRange::new(0, 0),
    TableRange::new(0, 0),
    TableRange::new(0, 1),
    TableRange::new(0, 0),
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
    TableRange::new(0, 0),
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
        TableRange::new(0, 0),
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
    startups: TableRange<LifecycleReactionImage>,
) -> ScopeImage {
    ScopeImage::new(
        parent,
        ReactorIndex::new(0),
        mode,
        descendants,
        logical_actions,
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        startups,
        TableRange::new(0, 0),
    )
}

#[derive(Debug)]
struct ModeState {
    entered_mode: u32,
}

fn initialize_mode_state() -> ModeState {
    ModeState { entered_mode: 0 }
}

fn request_compiled_mode(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    effect: CompiledModeEffectRef,
) -> Result<(), ReactionBindingError> {
    effect.set(context);
    Ok(())
}

fn request_legacy_mode(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    ModeEffectRef::new_key(ModeKey::from(1), TransitionKind::Reset).set(context);
    Ok(())
}

fn record_mode_entry(
    context: &mut Context,
    state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    state
        .downcast_mut::<ModeState>()
        .expect("the modal image initializes ModeState")
        .entered_mode = 1;
    context.schedule_shutdown(None);
    Ok(())
}

fn compiled_modal_bindings() -> OwnedBindings {
    OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_compiled_reaction(BindingSlotIndex::new(1), request_compiled_mode)
        .bind_reaction(BindingSlotIndex::new(2), record_mode_entry)
}

fn legacy_mode_bindings() -> OwnedBindings {
    OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_mode_state)
        .bind_reaction(BindingSlotIndex::new(1), request_legacy_mode)
        .bind_reaction(BindingSlotIndex::new(2), record_mode_entry)
}

static MODAL_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    TableRange::new(0, 2),
    Some(ModeIndex::new(0)),
    None,
)];
static MODAL_ACTIONS: [ActionImage; 1] = [fixture_timer_action(0, None, TableRange::new(0, 1))];
static MODAL_REACTIONS: [ReactionImage; 2] = [
    fixture_reaction(
        0,
        1,
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    )
    .with_mode_effect(CompiledModeEffectRef {
        target: ModeIndex::new(1),
        transition: TransitionKind::Reset,
    }),
    fixture_reaction(
        2,
        2,
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 1),
    ),
];
static MODAL_MODES: [ModeImage; 2] = [
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(1)),
    ModeImage::new(ReactorIndex::new(0), ScopeIndex::new(2)),
];
static MODAL_SCOPES: [ScopeImage; 3] = [
    fixture_scope(
        None,
        None,
        TableRange::new(0, 3),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    ),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(0)),
        TableRange::new(3, 1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    ),
    fixture_scope(
        Some(ScopeIndex::new(0)),
        Some(ModeIndex::new(1)),
        TableRange::new(4, 1),
        TableRange::new(0, 0),
        TableRange::new(0, 1),
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
    enclave_id: IdentityRange::new(0, 18),
    reactors: TinyMapView::new(&MODAL_REACTORS),
    actions: TinyMapView::new(&MODAL_ACTIONS),
    ports: TinyMapView::new(&PORTS),
    reactions: TinyMapView::new(&MODAL_REACTIONS),
    modes: TinyMapView::new(&MODAL_MODES),
    scopes: TinyMapView::new(&MODAL_SCOPES),
    reaction_triggers: &MODAL_REACTION_TRIGGERS,
    reaction_use_ports: &[],
    reaction_effect_ports: &[],
    reaction_actions: &[],
    reaction_modes: &MODAL_REACTION_MODES,
    scope_descendants: &MODAL_SCOPE_DESCENDANTS,
    scope_logical_actions: &[],
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &MODAL_SCOPE_STARTUPS,
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &MODAL_TIMER_STARTUPS,
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTES),
    required_bindings: TinyMapView::new(&MODAL_REQUIRED_BINDINGS),
    storage_bounds: StorageBounds::new(1, 1, 2, 0, 0, 0),
};

#[derive(Debug)]
struct AdmissionState {
    boundary_value: Option<u32>,
    action_value: Option<u32>,
}

fn initialize_admission_state() -> AdmissionState {
    AdmissionState {
        boundary_value: None,
        action_value: None,
    }
}

fn admit_boundary_and_action(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let tag = context.get_tag().delay(Duration::nanoseconds(1));
    assert!(context.schedule_external(AsyncEvent::Logical {
        tag,
        target: AsyncEventTarget::BoundaryPort(PortKey::new(0)),
        value: Box::new(42_u32),
    }));
    assert!(context.schedule_external(AsyncEvent::Logical {
        tag,
        target: AsyncEventTarget::Action(ActionKey::new(1)),
        value: Box::new(7_u32),
    }));
    Ok(())
}

fn record_boundary_value(
    _context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let port: InputRef<u32> = refs.ports.partition()?;
    state
        .downcast_mut::<AdmissionState>()
        .expect("the admission image initializes AdmissionState")
        .boundary_value = port.as_ref().copied();
    Ok(())
}

fn record_action_value(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
) -> Result<(), ReactionBindingError> {
    let mut action: ActionRef<u32> = refs.actions.partition_mut()?;
    state
        .downcast_mut::<AdmissionState>()
        .expect("the admission image initializes AdmissionState")
        .action_value = context.get_action_value(&mut action).copied();
    context.schedule_shutdown(None);
    Ok(())
}

fn admission_bindings() -> OwnedBindings {
    OwnedBindings::new()
        .bind_action(BindingSlotIndex::new(0), PayloadType::<u32>::new())
        .bind_port(BindingSlotIndex::new(1), PayloadType::<u32>::new())
        .bind_reaction(BindingSlotIndex::new(2), admit_boundary_and_action)
        .bind_reaction(BindingSlotIndex::new(3), record_boundary_value)
        .bind_reaction(BindingSlotIndex::new(4), record_action_value)
        .bind_state(BindingSlotIndex::new(5), initialize_admission_state)
}

static ADMISSION_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(5),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    TableRange::new(0, 0),
    None,
    None,
)];
static ADMISSION_ACTIONS: [ActionImage; 2] = [
    fixture_timer_action(0, None, TableRange::new(0, 1)),
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Standard {
            domain: TimingDomain::Logical,
            min_delay_nanos: 0,
        },
        TableRange::new(1, 1),
        Some(BindingSlotIndex::new(0)),
    ),
];
static ADMISSION_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    TableRange::new(2, 1),
    BindingSlotIndex::new(1),
)];
static ADMISSION_REACTIONS: [ReactionImage; 3] = [
    fixture_reaction(
        0,
        2,
        TableRange::new(0, 0),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    ),
    fixture_reaction(
        0,
        3,
        TableRange::new(0, 1),
        TableRange::new(0, 0),
        TableRange::new(0, 0),
    ),
    fixture_reaction(
        0,
        4,
        TableRange::new(1, 0),
        TableRange::new(0, 1),
        TableRange::new(0, 0),
    ),
];
static ADMISSION_TRIGGERS: [LevelReactionImage; 3] = [
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    LevelReactionImage::new(0, ReactionIndex::new(2)),
    LevelReactionImage::new(0, ReactionIndex::new(1)),
];
static ADMISSION_SCOPE: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    TableRange::new(0, 1),
    TableRange::new(0, 1),
    TableRange::new(0, 0),
)];
static ADMISSION_BINDINGS: [RequiredBindingImage; 6] = [
    RequiredBindingImage::new(IdentityRange::new(18, 8), BindingKind::Action),
    RequiredBindingImage::new(IdentityRange::new(26, 6), BindingKind::Port),
    RequiredBindingImage::new(IdentityRange::new(32, 8), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(40, 10), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(50, 17), BindingKind::Reaction),
    RequiredBindingImage::new(IdentityRange::new(67, 7), BindingKind::StateInitializer),
];

static ADMISSION_IMAGE: EnclaveImage<'static> = EnclaveImage {
    identity_data: "compiled/referencea-actionb-portc-sourced-boundarye-action-reactionf-state",
    enclave_id: IdentityRange::new(0, 18),
    reactors: TinyMapView::new(&ADMISSION_REACTORS),
    actions: TinyMapView::new(&ADMISSION_ACTIONS),
    ports: TinyMapView::new(&ADMISSION_PORTS),
    reactions: TinyMapView::new(&ADMISSION_REACTIONS),
    modes: TinyMapView::new(&MODES),
    scopes: TinyMapView::new(&ADMISSION_SCOPE),
    reaction_triggers: &ADMISSION_TRIGGERS,
    reaction_use_ports: &[PortIndex::new(0)],
    reaction_effect_ports: &[],
    reaction_actions: &[ActionIndex::new(1)],
    reaction_modes: &[],
    scope_descendants: &[ScopeIndex::new(0)],
    scope_logical_actions: &[ActionIndex::new(1)],
    scope_timer_startups: &[],
    scope_reset_reactions: &[],
    scope_startup_reactions: &[],
    scope_shutdown_reactions: &[],
    startup_actions: &[],
    timer_startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), 0)],
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTES),
    required_bindings: TinyMapView::new(&ADMISSION_BINDINGS),
    storage_bounds: StorageBounds::new(1, 2, 4, 0, 0, 0),
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

fn periodic_bindings() -> OwnedBindings {
    OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_periodic_counter)
        .bind_reaction(BindingSlotIndex::new(1), record_periodic_tag)
}

static PERIODIC_ACTIONS: [ActionImage; 1] = [fixture_timer_action(
    0,
    Some(1_000_000),
    TableRange::new(0, 1),
)];
static ZERO_PERIOD_ACTIONS: [ActionImage; 1] =
    [fixture_timer_action(0, Some(0), TableRange::new(0, 1))];
static OVERFLOW_PERIOD_ACTIONS: [ActionImage; 1] =
    [fixture_timer_action(0, Some(1), TableRange::new(0, 1))];
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
    identity_data: "compiled/referencecounter-stateincrement-counter",
    enclave_id: IdentityRange::new(0, 18),
    reactors: TinyMapView::new(&REACTORS),
    actions: TinyMapView::new(&COALESCED_ACTIONS),
    ports: TinyMapView::new(&PORTS),
    reactions: TinyMapView::new(&REACTIONS),
    modes: TinyMapView::new(&MODES),
    scopes: TinyMapView::new(&SCOPES),
    reaction_triggers: &COALESCED_REACTION_TRIGGERS,
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
    startup_actions: &COALESCED_STARTUP_ACTIONS,
    timer_startup_actions: &[],
    shutdown_reactions: &[],
    shutdown_actions: &[],
    routes: TinyMapView::new(&ROUTES),
    required_bindings: TinyMapView::new(&REQUIRED_BINDINGS),
    storage_bounds: StorageBounds::new(1, 2, 1, 0, 0, 0),
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
        match execute_owned(&image, OwnedBindings::new(), Config::default()) {
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

    match execute_owned(
        &ROUTED_IMAGE,
        reference_bindings().bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new()),
        Config::default().with_fast_forward(true),
    ) {
        Ok(_) => panic!("routed images must not execute without route support"),
        Err(ExecuteOwnedError::RoutesUnsupported { count: 1 }) => {}
        Err(error) => panic!("unexpected route rejection: {error}"),
    }
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
fn compiled_async_admission_targets_boundary_ports_and_actions() {
    let result = execute_owned(
        &ADMISSION_IMAGE,
        admission_bindings(),
        Config::default().with_fast_forward(true),
    )
    .unwrap();
    let state = result
        .state::<AdmissionState>(StateSlotIndex::new(0))
        .unwrap();

    assert_eq!(state.boundary_value, Some(42));
    assert_eq!(state.action_value, Some(7));
}

#[test]
fn compiled_periodic_timer_recurrence_uses_prior_logical_tag() {
    PERIODIC_INITIALIZATIONS.store(0, Ordering::SeqCst);
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
fn compiled_periodic_timer_rejects_zero_period_before_state_initialization() {
    PERIODIC_INITIALIZATIONS.store(0, Ordering::SeqCst);
    let error = match execute_owned(&ZERO_PERIOD_IMAGE, periodic_bindings(), Config::default()) {
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
        periodic_bindings(),
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
