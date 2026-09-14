//! Shared typed source/sink image fixtures for local routes and isolated backends.
use super::*;

/// Source state used to verify the shared Federate origin.
#[derive(Debug)]
pub(super) struct RoutedSourceState {
    /// Completion time of this source's user initializer.
    pub(super) initialized_at: Instant,
    /// Origin observed from the source reaction context.
    pub(super) origin: Option<Instant>,
    /// Physical time at which the source timer reaction ran.
    pub(super) fired_at: Option<Instant>,
}

pub(super) fn initialize_routed_source() -> RoutedSourceState {
    RoutedSourceState {
        initialized_at: Instant::now(),
        origin: None,
        fired_at: None,
    }
}

pub(super) fn initialize_delayed_routed_source() -> RoutedSourceState {
    std::thread::sleep(std::time::Duration::from_millis(150));
    initialize_routed_source()
}

pub(super) fn emit_routed_value(
    context: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let state = state
        .downcast_mut::<RoutedSourceState>()
        .expect("the source state binding initializes RoutedSourceState");
    state.origin = Some(context.get_start_time());
    state.fired_at = Some(Instant::now());
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(42);
    Ok(())
}

/// Destination state used to verify typed route delivery and the shared origin.
#[derive(Debug)]
pub(super) struct RoutedSinkState {
    /// Typed values observed by the destination reaction.
    pub(super) values: Vec<u32>,
    /// Origin observed from the destination reaction context.
    pub(super) origin: Option<Instant>,
}

pub(super) fn initialize_routed_sink() -> RoutedSinkState {
    RoutedSinkState {
        values: Vec::new(),
        origin: None,
    }
}

pub(super) fn receive_routed_value(
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

pub(super) fn source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

pub(super) fn paced_source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_delayed_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), emit_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

pub(super) fn sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), receive_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

pub(super) fn route_boundary() -> BoundaryId<'static> {
    BoundaryId::new("pipe")
}

pub(super) static ROUTED_SOURCE_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    s!(0, 0),
    None,
    None,
)];
pub(super) static ROUTED_SOURCE_ACTIONS: [ActionImage; 1] = [ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Timer { period_nanos: None },
    r!(0, 1),
    None,
)];
pub(super) static ROUTED_SOURCE_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 0),
    BindingSlotIndex::new(2),
)];
pub(super) static ROUTED_SOURCE_REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
    ReactorIndex::new(0),
    ScopeIndex::new(0),
    0,
    BindingSlotIndex::new(1),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
)];
pub(super) static ROUTED_SOURCE_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
pub(super) static ROUTED_SOURCE_EFFECT_PORTS: [PortIndex; 1] = [PortIndex::new(0)];
pub(super) static ROUTED_SOURCE_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
pub(super) static ROUTED_SOURCE_TIMER_STARTUPS: [TimerStartupImage; 1] =
    [TimerStartupImage::new(ActionIndex::new(0), 0)];
pub(super) static ROUTED_SOURCE_SCOPES: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    r!(0, 1),
    r!(0, 0),
    r!(0, 1),
    r!(0, 0),
)];
pub(super) static ROUTED_SOURCE_ROUTES: [RouteImage; 1] = [fixture_route(
    "pipe",
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    1_000_000,
)];
pub(super) static ROUTED_SOURCE_BINDINGS: [RequiredBindingImage; 3] = [
    fixture_binding("a", BindingKind::StateInitializer),
    fixture_binding("b", BindingKind::Reaction),
    fixture_binding("c", BindingKind::Port),
];
pub(super) const ROUTED_SOURCE_IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: EnclaveId::new("alpha"),
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
    storage_bounds: &StorageBounds::new(1, 1, 8, 0, 0, 0),
};

pub(super) static ROUTED_SINK_REACTORS: &[ReactorImage; 1] = &ROUTED_SOURCE_REACTORS;
pub(super) static ROUTED_SINK_PORTS: [PortImage; 1] = [PortImage::new(
    ScopeIndex::new(0),
    r!(0, 1),
    BindingSlotIndex::new(2),
)];
pub(super) static ROUTED_SINK_REACTIONS: [ReactionImage; 1] =
    [fixture_reaction(0, 1, r!(0, 1), r!(0, 0), r!(0, 0))];
pub(super) static ROUTED_SINK_TRIGGERS: [LevelReactionImage; 1] =
    [LevelReactionImage::new(0, ReactionIndex::new(0))];
pub(super) static ROUTED_SINK_USE_PORTS: [PortIndex; 1] = [PortIndex::new(0)];
pub(super) static ROUTED_SINK_DESCENDANTS: [ScopeIndex; 1] = [ScopeIndex::new(0)];
pub(super) static ROUTED_SINK_SCOPES: [ScopeImage; 1] = [fixture_scope(
    None,
    None,
    r!(0, 1),
    r!(0, 0),
    r!(0, 0),
    r!(0, 0),
)];
pub(super) static ROUTED_SINK_ROUTES: [RouteImage; 1] = [fixture_route(
    "pipe",
    PortIndex::new(0),
    RouteDirection::Inbound,
    TimingDomain::Logical,
    1_000_000,
)];
pub(super) static ROUTED_SINK_BINDINGS: [RequiredBindingImage; 3] = [
    fixture_binding("a", BindingKind::StateInitializer),
    fixture_binding("b", BindingKind::Reaction),
    fixture_binding("c", BindingKind::Port),
];
pub(super) const ROUTED_SINK_IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: EnclaveId::new("beta"),
    reactors: TinyMapView::new(ROUTED_SINK_REACTORS),
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
    storage_bounds: &StorageBounds::new(1, 0, 8, 0, 0, 0),
};

pub(super) static ROUTED_FEDERATES: [FederateImage; 1] =
    [fixture_federate("host", "target", "runtime", s!(0, 2))];
pub(super) static ROUTED_ENCLAVES: [EnclaveImage<'static>; 2] =
    [ROUTED_SOURCE_IMAGE, ROUTED_SINK_IMAGE];
pub(super) static ROUTED_FEDERATE_MEMBERS: [FederateIndex; 1] = [FederateIndex::new(0)];
pub(super) const ROUTED_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&ROUTED_FEDERATE_MEMBERS, &[]),
    federates: TinyMapView::new(&ROUTED_FEDERATES),
    enclaves: TinyMapView::new(&ROUTED_ENCLAVES),
    coordination: CoordinationProjection::Local,
};

type RoutedReaction = fn(
    &mut Context,
    &mut dyn ReactorData,
    ReactionRefs<'_>,
    Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError>;

pub(super) fn routed_reaction_bindings(reaction: RoutedReaction) -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_source)
        .bind_reaction(BindingSlotIndex::new(1), reaction)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}
