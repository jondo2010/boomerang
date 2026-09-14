//! Periodic recurrence, cotimed timers, shutdown, and overflow validation.
use super::*;

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
pub(super) static PERIODIC_ACTIONS: [ActionImage; 1] =
    [fixture_timer_action(0, Some(1_000_000), r!(0, 1))];
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
    storage_bounds: &StorageBounds::new(1, 2, 3, 0, 0, 0),
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
    storage_bounds: &StorageBounds::new(1, 2, 4, 0, 0, 0),
    ..IMAGE
};
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
