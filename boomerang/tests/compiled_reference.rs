use boomerang::runtime::{
    execute_owned,
    image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
        EnclaveImage, IdentityRange, LevelReactionImage, LifecycleReactionImage, ReactionImage,
        ReactionIndex, ReactorImage, ReactorIndex, RequiredBindingImage, ScopeImage, ScopeIndex,
        StateSlotIndex, StorageBounds, TableRange, TimerStartupImage, TinyMapView,
    },
    Config, Context, Duration, OwnedBindings, ReactionBindingError, ReactionRefs, ReactorData, Tag,
};

/// Mutable reactor state whose startup reaction records one execution.
#[derive(Debug)]
struct CounterState {
    count: usize,
}

/// Initializes the counter state supplied by the direct state binding.
fn initialize_counter() -> CounterState {
    CounterState { count: 0 }
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
)];
static COALESCED_ACTIONS: [ActionImage; 2] = [
    ACTIONS[0],
    ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(1),
        ActionTiming::Timer { period_nanos: None },
        TableRange::new(1, 1),
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
static STARTUP_ACTIONS: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 0)];
static COALESCED_STARTUP_ACTIONS: [TimerStartupImage; 2] = [
    STARTUP_ACTIONS[0],
    TimerStartupImage::new(ActionIndex::new(1), 1),
];
static ROUTES: [boomerang::runtime::image::RouteImage; 0] = [];
static REQUIRED_BINDINGS: [RequiredBindingImage; 2] = [
    RequiredBindingImage::new(IdentityRange::new(18, 13), BindingKind::StateInitializer),
    RequiredBindingImage::new(IdentityRange::new(31, 17), BindingKind::Reaction),
];

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
    startup_actions: &STARTUP_ACTIONS,
    timer_startup_actions: &[],
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

#[test]
fn compiled_reference_executes_startup_to_shutdown() {
    let bindings = OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_action::<()>(ActionSlotIndex::new(0))
        .bind_reaction(BindingSlotIndex::new(1), increment_counter);

    let result =
        execute_owned(&IMAGE, bindings, Config::default().with_fast_forward(true)).unwrap();

    assert_eq!(
        result
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap()
            .count,
        1
    );
    assert_eq!(result.final_tag(), Tag::ZERO);
}

#[test]
fn compiled_reference_final_tag_excludes_delayed_shutdown_only_work() {
    let bindings = OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_action::<()>(ActionSlotIndex::new(0))
        .bind_reaction(BindingSlotIndex::new(1), increment_counter);

    let result = execute_owned(
        &IMAGE,
        bindings,
        Config::default()
            .with_fast_forward(true)
            .with_timeout(Duration::nanoseconds(1)),
    )
    .unwrap();

    assert_eq!(result.final_tag(), Tag::ZERO);
}

#[test]
fn compiled_reference_final_tag_includes_work_coalesced_with_shutdown() {
    let bindings = OwnedBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_action::<()>(ActionSlotIndex::new(0))
        .bind_action::<()>(ActionSlotIndex::new(1))
        .bind_reaction(BindingSlotIndex::new(1), increment_counter);

    let result = execute_owned(
        &COALESCED_IMAGE,
        bindings,
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
