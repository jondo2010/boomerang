//! Minimal typed route fixture for compiled RTI integration contracts.
use super::*;

macro_rules! r {
    ($start:expr, $len:expr) => {
        SliceRange::new($start, $len)
    };
}

macro_rules! s {
    ($start:expr, $len:expr) => {
        IndexSpan::new($start, $len)
    };
}

pub(super) const fn fixture_federate(
    id: &'static str,
    target: &'static str,
    runtime: &'static str,
    enclaves: IndexSpan<EnclaveIndex>,
) -> FederateImage<'static> {
    FederateImage::new(
        FederateId::new(id),
        TargetId::new(target),
        RuntimeBackendId::new(runtime),
        enclaves,
    )
}

const fn fixture_route(
    boundary: &'static str,
    local_port: PortIndex,
    direction: RouteDirection,
    timing: TimingDomain,
    after_nanos: u64,
) -> RouteImage<'static> {
    RouteImage::new(
        BoundaryId::new(boundary),
        local_port,
        direction,
        timing,
        after_nanos,
    )
}

const fn fixture_binding(id: &'static str, kind: BindingKind) -> RequiredBindingImage<'static> {
    RequiredBindingImage::new(BindingSlotId::new(id), kind)
}
const fn fixture_reaction(
    scope: u32,
    binding: u32,
    use_ports: SliceRange<PortIndex>,
    actions: SliceRange<ActionIndex>,
    modes: SliceRange<ModeIndex>,
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
const fn fixture_scope(
    parent: Option<ScopeIndex>,
    mode: Option<ModeIndex>,
    descendants: SliceRange<ScopeIndex>,
    logical_actions: SliceRange<ActionIndex>,
    timer_startups: SliceRange<TimerStartupImage>,
    startups: SliceRange<LifecycleReactionImage>,
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
static ROUTED_SOURCE_REACTORS: [ReactorImage; 1] = [ReactorImage::new(
    BindingSlotIndex::new(0),
    StateSlotIndex::new(0),
    ScopeIndex::new(0),
    s!(0, 0),
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
static ROUTED_SOURCE_ROUTES: [RouteImage; 1] = [fixture_route(
    "pipe",
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    1_000_000,
)];
static ROUTED_SOURCE_BINDINGS: [RequiredBindingImage; 3] = [
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

static ROUTED_SINK_REACTORS: &[ReactorImage; 1] = &ROUTED_SOURCE_REACTORS;
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
static ROUTED_SINK_ROUTES: [RouteImage; 1] = [fixture_route(
    "pipe",
    PortIndex::new(0),
    RouteDirection::Inbound,
    TimingDomain::Logical,
    1_000_000,
)];
static ROUTED_SINK_BINDINGS: [RequiredBindingImage; 3] = [
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

#[derive(Debug)]
pub(super) struct RoutedSinkState {
    pub(super) values: Vec<u32>,
}

pub(super) fn source_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), || ())
        .bind_reaction(BindingSlotIndex::new(1), emit_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn emit_value(
    _: &mut Context,
    _: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(42);
    Ok(())
}

pub(super) fn sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), || RoutedSinkState {
            values: Vec::new(),
        })
        .bind_reaction(BindingSlotIndex::new(1), receive_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

fn receive_value(
    _: &mut Context,
    state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    let input: InputRef<u32> = refs.ports.partition()?;
    state
        .downcast_mut::<RoutedSinkState>()
        .unwrap()
        .values
        .push(
            *input
                .as_ref()
                .expect("the inbound route sets the triggering port"),
        );
    Ok(())
}
