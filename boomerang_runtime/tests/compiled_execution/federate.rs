//! Local multi-enclave coordination, shared origins, routed values, and quiescence.
use super::*;

static HORIZON_SHUTDOWN_REACTIONS: [LifecycleReactionImage; 1] = [LifecycleReactionImage::new(
    LevelReactionImage::new(0, ReactionIndex::new(0)),
    ActionIndex::new(0),
)];
const HORIZON_WORK_IMAGE: EnclaveImage<'static> = EnclaveImage {
    shutdown_reactions: &HORIZON_SHUTDOWN_REACTIONS,
    ..IMAGE
};

const HORIZON_IDLE_IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: EnclaveId::new("idle"),
    scope_timer_startups: &[],
    timer_startup_actions: &[],
    shutdown_reactions: &HORIZON_SHUTDOWN_REACTIONS,
    ..IMAGE
};

static HORIZON_ENCLAVES: [EnclaveImage<'static>; 2] = [HORIZON_WORK_IMAGE, HORIZON_IDLE_IMAGE];
const QUIESCENT_HORIZON_IMAGE: EnclaveImage<'static> = EnclaveImage {
    scope_timer_startups: &[],
    timer_startup_actions: &[],
    ..HORIZON_WORK_IMAGE
};
static QUIESCENT_HORIZON_ENCLAVES: [EnclaveImage<'static>; 2] =
    [QUIESCENT_HORIZON_IMAGE, HORIZON_IDLE_IMAGE];
static HORIZON_FEDERATES: [FederateImage; 1] =
    [fixture_federate("host", "target", "runtime", s!(0, 2))];
static HORIZON_FEDERATE_MEMBERS: [FederateIndex; 1] = [FederateIndex::new(0)];
const HORIZON_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&HORIZON_FEDERATE_MEMBERS, &[]),
    federates: TinyMapView::new(&HORIZON_FEDERATES),
    enclaves: TinyMapView::new(&HORIZON_ENCLAVES),
    coordination: CoordinationProjection::Local,
};
const QUIESCENT_HORIZON_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    enclaves: TinyMapView::new(&QUIESCENT_HORIZON_ENCLAVES),
    ..HORIZON_DEPLOYMENT
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

static MULTI_LEFT_ROUTES: [RouteImage; 1] = [fixture_route(
    "left",
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];
static MULTI_RIGHT_ROUTES: [RouteImage; 1] = [fixture_route(
    "right",
    PortIndex::new(0),
    RouteDirection::Outbound,
    TimingDomain::Logical,
    0,
)];

const fn multi_source_image(
    enclave_id: &'static str,
    routes: &'static [RouteImage],
) -> EnclaveImage<'static> {
    EnclaveImage {
        enclave_id: EnclaveId::new(enclave_id),
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
    fixture_route(
        "left",
        PortIndex::new(0),
        RouteDirection::Inbound,
        TimingDomain::Logical,
        0,
    ),
    fixture_route(
        "right",
        PortIndex::new(1),
        RouteDirection::Inbound,
        TimingDomain::Logical,
        0,
    ),
];
static MULTI_SINK_BINDINGS: [RequiredBindingImage; 4] = [
    fixture_binding("a", BindingKind::StateInitializer),
    fixture_binding("b", BindingKind::Reaction),
    fixture_binding("c", BindingKind::Port),
    fixture_binding("d", BindingKind::Port),
];
const MULTI_SINK_IMAGE: EnclaveImage<'static> = EnclaveImage {
    enclave_id: EnclaveId::new("sink"),
    ports: TinyMapView::new(&MULTI_SINK_PORTS),
    reactions: TinyMapView::new(&MULTI_SINK_REACTIONS),
    reaction_triggers: &MULTI_SINK_TRIGGERS,
    reaction_use_ports: &MULTI_SINK_USE_PORTS,
    routes: TinyMapView::new(&MULTI_SINK_ROUTES),
    required_bindings: TinyMapView::new(&MULTI_SINK_BINDINGS),
    ..ROUTED_SINK_IMAGE
};

static MULTI_ENCLAVES: [EnclaveImage<'static>; 3] = [
    multi_source_image("alpha", &MULTI_LEFT_ROUTES),
    multi_source_image("gamma", &MULTI_RIGHT_ROUTES),
    MULTI_SINK_IMAGE,
];
static MULTI_FEDERATES: [FederateImage; 1] =
    [fixture_federate("host", "target", "runtime", s!(0, 3))];
const MULTI_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&ROUTED_FEDERATE_MEMBERS, &[]),
    federates: TinyMapView::new(&MULTI_FEDERATES),
    enclaves: TinyMapView::new(&MULTI_ENCLAVES),
    coordination: CoordinationProjection::Local,
};

#[test]
fn owned_federate_coordinates_multiple_same_tag_sources_before_destination_execution() {
    let result = execute_owned_federate(
        MULTI_DEPLOYMENT,
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
                ROUTED_DEPLOYMENT,
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
        assert_eq!(source.final_tag(), Tag::ZERO);
        assert_eq!(sink.final_tag(), Tag::new(Duration::milliseconds(1), 0));
        assert_eq!(result.final_tag(), source.final_tag().max(sink.final_tag()));
        assert_eq!(
            result.stats().processed_tags(),
            source
                .stats()
                .processed_tags()
                .saturating_add(sink.stats().processed_tags())
        );
        assert_eq!(
            result.stats().processed_reactions(),
            source
                .stats()
                .processed_reactions()
                .saturating_add(sink.stats().processed_reactions())
        );
        assert_eq!(
            result.stats().processed_events(),
            source
                .stats()
                .processed_events()
                .saturating_add(sink.stats().processed_events())
        );
        assert_eq!(
            result.stats().set_ports(),
            source
                .stats()
                .set_ports()
                .saturating_add(sink.stats().set_ports())
        );
        assert_eq!(
            result.stats().scheduled_actions(),
            source
                .stats()
                .scheduled_actions()
                .saturating_add(sink.stats().scheduled_actions())
        );
        assert_eq!(source_state.origin, Some(result.origin()));
        assert_eq!(sink_state.origin, Some(result.origin()));
    }
}

#[test]
/// Verifies a paced source retains the shared origin and delivers downstream in logical order.
fn owned_federate_paced_origin_preserves_downstream_order() {
    const TIMER_DELAY_NANOS: u64 = 100_000_000;
    let result = bounded(|| {
        let timer_startups = [TimerStartupImage::new(
            ActionIndex::new(0),
            TIMER_DELAY_NANOS,
        )];
        let enclaves = [
            EnclaveImage {
                scope_timer_startups: &timer_startups,
                timer_startup_actions: &timer_startups,
                ..ROUTED_SOURCE_IMAGE
            },
            ROUTED_SINK_IMAGE,
        ];
        let deployment = CompiledDeploymentImage {
            enclaves: TinyMapView::new(&enclaves),
            ..ROUTED_DEPLOYMENT
        };
        execute_owned_federate(
            deployment,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), paced_source_bindings())
                .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                .bind_route(
                    route_boundary(),
                    PayloadType::<u32>::new(),
                    PayloadType::<u32>::new(),
                ),
            Config::default().with_fast_forward(false),
        )
        .unwrap()
    });

    let source = result
        .enclave(EnclaveIndex::new(0))
        .expect("paced source result must be present");
    let sink = result
        .enclave(EnclaveIndex::new(1))
        .expect("paced sink result must be present");
    let state = source
        .state::<RoutedSourceState>(StateSlotIndex::new(0))
        .unwrap();
    let origin = result.origin();
    let fired_at = state.fired_at.expect("paced source timer must fire");
    let timer_delay = std::time::Duration::from_nanos(TIMER_DELAY_NANOS);
    let sink_state = sink
        .state::<RoutedSinkState>(StateSlotIndex::new(0))
        .unwrap();

    assert_eq!(state.origin, Some(origin));
    assert_eq!(sink_state.values, [42]);
    assert!(source.final_tag() < sink.final_tag());
    assert!(origin >= state.initialized_at);
    assert!(fired_at.duration_since(origin) >= timer_delay);
    assert!(fired_at.duration_since(state.initialized_at) >= timer_delay);
}
#[test]
fn owned_federate_quiesces_when_a_source_emits_no_route_value() {
    let result = bounded(|| {
        execute_owned_federate(
            ROUTED_DEPLOYMENT,
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

/// Verifies an explicit scheduler shutdown terminates a kept-alive compiled Federate.
#[test]
fn owned_federate_keep_alive_shutdown_stops_idle_peer() {
    let result = bounded(|| {
        execute_owned_federate(
            ROUTED_DEPLOYMENT,
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
            Config::default()
                .with_keep_alive(true)
                .with_fast_forward(false),
        )
        .unwrap()
    });

    assert!(result.enclave(EnclaveIndex::new(0)).is_some());
    assert!(result.enclave(EnclaveIndex::new(1)).is_some());
}

#[test]
fn owned_federate_quiescence_wins_before_logical_horizon() {
    let result = bounded(|| {
        execute_owned_federate(
            QUIESCENT_HORIZON_DEPLOYMENT,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), reference_bindings())
                .bind_enclave(EnclaveIndex::new(1), reference_bindings()),
            Config::default()
                .with_fast_forward(true)
                .with_timeout(Duration::seconds(1)),
        )
        .unwrap()
    });

    for enclave in [EnclaveIndex::new(0), EnclaveIndex::new(1)] {
        let state = result
            .enclave(enclave)
            .unwrap()
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap();
        assert_eq!(
            state.tags,
            [Tag::ZERO],
            "graceful quiescence must run every Enclave's shutdown reaction"
        );
        assert_eq!(result.enclave(enclave).unwrap().final_tag(), Tag::NEVER);
    }
    assert_eq!(result.final_tag(), Tag::NEVER);
}

#[test]
fn owned_federate_logical_horizon_stops_all_enclaves_at_one_tag() {
    let horizon = Duration::nanoseconds(1);
    let result = bounded(move || {
        execute_owned_federate(
            HORIZON_DEPLOYMENT,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), reference_bindings())
                .bind_enclave(EnclaveIndex::new(1), reference_bindings()),
            Config::default()
                .with_fast_forward(true)
                .with_timeout(horizon),
        )
        .unwrap()
    });

    let expected = Tag::new(horizon, 0);
    for enclave in [EnclaveIndex::new(0), EnclaveIndex::new(1)] {
        let state = result
            .enclave(enclave)
            .unwrap()
            .state::<CounterState>(StateSlotIndex::new(0))
            .unwrap();
        assert_eq!(
            state.tags,
            [expected],
            "every Enclave must run shutdown at the shared logical horizon"
        );
        assert_eq!(result.enclave(enclave).unwrap().final_tag(), Tag::NEVER);
    }
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
            fixture_route(
                "pipe",
                PortIndex::new(0),
                RouteDirection::Inbound,
                TimingDomain::Logical,
                1,
            ),
            fixture_route(
                "pipe",
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
        let federates = [fixture_federate("host", "target", "runtime", s!(0, 1))];
        let deployment = CompiledDeploymentImage {
            federates: TinyMapView::new(&federates),
            enclaves: TinyMapView::new(&enclaves),
            ..ROUTED_DEPLOYMENT
        };
        execute_owned_federate(
            deployment,
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
