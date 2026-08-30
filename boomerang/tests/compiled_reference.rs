//! Exercises the host-runtime seam from a compiled image and direct bindings through owned storage and scheduling to typed results, excluding compiler lowering and live-graph construction.

use boomerang::runtime::{
    execute_owned,
    image::{
        ActionImage, ActionIndex, ActionSlotIndex, ActionTiming, BindingKind, BindingSlotIndex,
        EnclaveImage, IdentityRange, ImageValidationError, LevelReactionImage,
        LifecycleReactionImage, PortImage, PortIndex, ReactionImage, ReactionIndex, ReactorImage,
        ReactorIndex, RequiredBindingImage, RouteDirection, RouteImage, ScopeImage, ScopeIndex,
        StateSlotIndex, StorageBounds, TableRange, TimerStartupImage, TimingDomain, TinyMapView,
    },
    Config, Context, Duration, ExecuteOwnedError, OwnedBindings, ReactionBindingError,
    ReactionRefs, ReactorData, StateAccessError, Tag,
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
static STARTUP_ACTIONS: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 5)];
static COALESCED_STARTUP_ACTIONS: [TimerStartupImage; 2] = [
    TimerStartupImage::new(ActionIndex::new(0), 0),
    TimerStartupImage::new(ActionIndex::new(1), 1),
];
static ROUTES: [boomerang::runtime::image::RouteImage; 0] = [];
static ROUTED_PORTS: [PortImage; 1] = [PortImage::new(ScopeIndex::new(0), TableRange::new(0, 0))];
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
    ports: TinyMapView::new(&ROUTED_PORTS),
    routes: TinyMapView::new(&ROUTED_ROUTES),
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
        reference_bindings().bind_port::<u32>(PortIndex::new(0)),
        Config::default().with_fast_forward(true),
    )
    .expect("the same image must execute when its route table is empty");

    match execute_owned(
        &ROUTED_IMAGE,
        reference_bindings().bind_port::<u32>(PortIndex::new(0)),
        Config::default().with_fast_forward(true),
    ) {
        Ok(_) => panic!("routed images must not execute without route support"),
        Err(ExecuteOwnedError::RoutesUnsupported { count: 1 }) => {}
        Err(error) => panic!("unexpected route rejection: {error}"),
    }
}
