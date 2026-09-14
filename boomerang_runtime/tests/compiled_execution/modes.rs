//! Canonical compiled modes, effect authorization, and periodic reset semantics.
use super::*;

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
    s!(0, 2),
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
    fixture_binding("a-state", BindingKind::StateInitializer),
    fixture_binding("b-transition", BindingKind::Reaction),
    fixture_binding("c-entry", BindingKind::Reaction),
];

static MODAL_IMAGE: EnclaveImage<'static> = EnclaveImage {
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
    storage_bounds: &StorageBounds::new(1, 1, 2, 0, 0, 0),
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
    fixture_binding("a-state", BindingKind::StateInitializer),
    fixture_binding("b-transition", BindingKind::Reaction),
];
static PERIODIC_MODAL_IMAGE: EnclaveImage<'static> = EnclaveImage {
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
    storage_bounds: &StorageBounds::new(1, 1, 1, 0, 0, 0),
    ..IMAGE
};
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
