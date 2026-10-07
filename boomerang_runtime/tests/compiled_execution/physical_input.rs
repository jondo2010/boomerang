use super::*;
use boomerang_runtime::{physical_clock::ManualClock, physical_input::*, physical_time::*};
const PHYSICAL: ActionImage = ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(1),
    ActionTiming::Standard {
        domain: TimingDomain::Physical,
        min_delay_nanos: 0,
    },
    r!(1, 1),
    Some(BindingSlotIndex::new(2)),
);
static FAST: [ActionImage; 2] = [fixture_timer_action(0, Some(2), r!(0, 1)), PHYSICAL];
static SLOW: [ActionImage; 2] = [fixture_timer_action(0, Some(3), r!(0, 1)), PHYSICAL];
static REQUIRED: [RequiredBindingImage; 3] = [
    REQUIRED_BINDINGS[0],
    REQUIRED_BINDINGS[1],
    fixture_binding("sample", BindingKind::Action),
];
static INPUT_REACTIONS: [ReactionImage; 1] = [fixture_reaction(0, 1, r!(0, 0), r!(0, 1), r!(0, 0))];
const INPUT_IMAGE: EnclaveImage<'static> = EnclaveImage {
    actions: TinyMapRef::from_slice(&FAST),
    reaction_triggers: &[LevelReactionImage::new(0, ReactionIndex::new(0)); 2],
    reactions: TinyMapRef::from_slice(&INPUT_REACTIONS),
    reaction_actions: &[ActionIndex::new(1)],
    required_bindings: TinyMapRef::from_slice(&REQUIRED),
    timer_startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), 0)],
    storage_bounds: &StorageBounds::new(1, 2, 8, 0, 0, 0),
    ..IMAGE
};
static ENCLAVES: [EnclaveImage<'static>; 2] = [
    EnclaveImage {
        enclave_id: EnclaveId::new("a"),
        ..INPUT_IMAGE
    },
    EnclaveImage {
        enclave_id: EnclaveId::new("b"),
        actions: TinyMapRef::from_slice(&SLOW),
        ..INPUT_IMAGE
    },
];
static FEDERATES: [FederateImage; 1] = [fixture_federate("inputs", "host", "std", s!(0, 2))];
const DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&[FederateIndex::new(0)], &[]),
    federates: TinyMapRef::from_slice(&FEDERATES),
    enclaves: TinyMapRef::from_slice(&ENCLAVES),
    coordination: CoordinationProjection::Local,
};
fn config() -> InputConfig {
    InputConfig {
        max_batch_values: 2,
        max_staged_batches: 2,
        sources: ["a", "b"]
            .into_iter()
            .map(|id| InputSource {
                id: id.into(),
                required: true,
                targets: vec![InputTarget::action::<u32>("sample", id, "sample")],
            })
            .collect(),
    }
}
fn bindings(
    clock: &ManualClock,
    ready: std::sync::mpsc::Sender<InputAdmission>,
    events: std::sync::mpsc::Sender<(u32, i128, u32)>,
    required: bool,
) -> FederateBindings<'static> {
    let mut config = config();
    for source in &mut config.sources {
        source.required = required;
    }
    let mut bindings = FederateBindings::new()
        .with_physical_clock(clock.domain(), clock.clone())
        .with_physical_inputs(config, move |inputs| {
            ready.send(inputs).unwrap();
            Ok(())
        });
    for index in 0..2 {
        let events = events.clone();
        bindings = bindings.bind_enclave(
            EnclaveIndex::new(index),
            EnclaveBindings::new()
                .bind_state(BindingSlotIndex::new(0), initialize_counter)
                .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                .bind_reaction(
                    BindingSlotIndex::new(1),
                    move |ctx: &mut Context,
                          state: &mut dyn ReactorData,
                          refs: ReactionRefs<'_>,
                          _: Option<CompiledModeEffectRef>| {
                        let (mut action,): (ActionRef<u32>,) = refs.actions.partition_mut()?;
                        let value = ctx
                            .get_action_value(&mut action)
                            .copied()
                            .unwrap_or_default();
                        let state = state.downcast_mut::<CounterState>().unwrap();
                        state.tags.push(ctx.get_tag());
                        state.count += value as usize;
                        events
                            .send((index, ctx.get_tag().offset().whole_nanoseconds(), value))
                            .unwrap();
                        Ok(())
                    },
                ),
        );
    }
    bindings
}
#[test]
fn physical_input_two_enclave_batch_precedes_timers_and_clock_cannot_outrun_progress() {
    bounded(|| {
        let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
        let (ready, handle) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        let binding = bindings(&clock, ready, tx, true);
        let runner = std::thread::spawn(move || {
            execute_owned_federate(
                DEPLOYMENT,
                FederateIndex::new(0),
                binding,
                Config::default().with_timeout(Duration::nanoseconds(12)),
            )
        });
        let inputs = handle.recv().unwrap();
        clock.advance_to(PhysicalTimeNanos(12)).unwrap();
        let sources: Vec<_> = ["a", "b"].map(|id| inputs.source(id).unwrap()).into();
        inputs
            .submit(
                sources
                    .iter()
                    .map(|&source| InputObservation {
                        source,
                        sequence: 1,
                        domain: clock.domain(),
                        epoch: clock.epoch(),
                        acquired: PhysicalTimeNanos(0),
                        values: vec![InputValue::new(
                            inputs.target(source, "sample").unwrap(),
                            7u32,
                        )],
                    })
                    .collect(),
                &sources
                    .iter()
                    .map(|&source| (source, PhysicalTimeNanos(1)))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        for _ in 0..2 {
            let (_, tag, value) = rx.recv().unwrap();
            assert_eq!((tag, value), (0, 7));
        }
        inputs.advance(sources[0], PhysicalTimeNanos(13)).unwrap();
        assert!(rx.try_recv().is_err());
        inputs.advance(sources[1], PhysicalTimeNanos(7)).unwrap();
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(rx.recv().unwrap());
        }
        seen.sort();
        assert_eq!(
            seen,
            [(0, 2, 0), (0, 4, 0), (0, 6, 0), (1, 3, 0), (1, 6, 0)]
        );
        inputs.advance(sources[1], PhysicalTimeNanos(13)).unwrap();
        let result = runner.join().unwrap().unwrap();
        for index in 0..2 {
            assert_eq!(
                result
                    .enclave(EnclaveIndex::new(index))
                    .unwrap()
                    .state::<CounterState>(StateSlotIndex::new(0))
                    .unwrap()
                    .count,
                7
            );
        }
        assert_eq!(
            inputs
                .advance(sources[0], PhysicalTimeNanos(14))
                .unwrap_err()
                .kind,
            InputErrorKind::Disconnected
        );
    });
}
#[test]
fn physical_input_failure_releases_enclaves_blocked_before_first_grant() {
    bounded(|| {
        for (clock_failure, idle) in [(false, false), (true, false), (false, true)] {
            let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
            let (ready, handle) = std::sync::mpsc::channel();
            let (events, _rx) = std::sync::mpsc::channel();
            let binding = bindings(&clock, ready, events, true);
            let observations: Vec<_> = (0..2)
                .map(|i| {
                    (
                        EnclaveIndex::new(i),
                        Arc::new(boomerang_runtime::ObservationState::new(Instant::now())),
                    )
                })
                .collect();
            let enclaves = [
                EnclaveImage {
                    timer_startup_actions: &[],
                    ..ENCLAVES[0]
                },
                EnclaveImage {
                    timer_startup_actions: &[],
                    ..ENCLAVES[1]
                },
            ];
            std::thread::scope(|scope| {
                let runner = scope.spawn(|| {
                    boomerang_runtime::execute_owned_federate_with_observations(
                        CompiledDeploymentImage {
                            enclaves: if idle {
                                TinyMapRef::from_slice(&enclaves)
                            } else {
                                DEPLOYMENT.enclaves
                            },
                            ..DEPLOYMENT
                        },
                        FederateIndex::new(0),
                        binding,
                        Config::default(),
                        &observations,
                    )
                });
                let inputs = handle.recv().unwrap();
                let bound = Instant::now() + std::time::Duration::from_secs(3);
                while !observations.iter().all(|(_, obs)| {
                    obs.snapshot(Instant::now()).is_some_and(|s| {
                        s.current_phase == boomerang_runtime::SchedulerPhase::CoordinationWait
                    })
                }) {
                    assert!(Instant::now() < bound);
                    std::thread::yield_now();
                }
                if clock_failure {
                    clock.fail();
                } else {
                    inputs.disconnect(inputs.source("a").unwrap());
                }
                let error = runner.join().unwrap().unwrap_err();
                assert!(
                    matches!(error, ExecuteOwnedFederateError::PhysicalClock(_) if clock_failure)
                        || matches!(error, ExecuteOwnedFederateError::PhysicalInput(_) if !clock_failure)
                );
            });
        }
    });
}

#[test]
fn physical_input_optional_batch_interrupts_old_grants_waiting_on_clock() {
    bounded(|| {
        let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
        let (ready, handle) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        let binding = bindings(&clock, ready, tx, false);
        let observations: Vec<_> = (0..2)
            .map(|i| {
                (
                    EnclaveIndex::new(i),
                    Arc::new(boomerang_runtime::ObservationState::new(Instant::now())),
                )
            })
            .collect();
        std::thread::scope(|scope| {
            let runner = scope.spawn(|| {
                boomerang_runtime::execute_owned_federate_with_observations(
                    DEPLOYMENT,
                    FederateIndex::new(0),
                    binding,
                    Config::default().with_timeout(Duration::nanoseconds(6)),
                    &observations,
                )
            });
            let inputs = handle.recv().unwrap();
            for _ in 0..2 {
                let (_, tag, value) = rx.recv().unwrap();
                assert_eq!((tag, value), (0, 0));
            }
            let bound = Instant::now() + std::time::Duration::from_secs(3);
            while !observations.iter().any(|(_, obs)| {
                obs.snapshot(Instant::now()).is_some_and(|s| {
                    s.current_phase == boomerang_runtime::SchedulerPhase::PhysicalWait
                })
            }) {
                assert!(Instant::now() < bound);
                std::thread::yield_now();
            }
            clock.advance_to(PhysicalTimeNanos(1)).unwrap();
            let batch = ["a", "b"].map(|id| {
                let source = inputs.source(id).unwrap();
                InputObservation {
                    source,
                    sequence: 1,
                    domain: clock.domain(),
                    epoch: clock.epoch(),
                    acquired: PhysicalTimeNanos(1),
                    values: vec![InputValue::new(
                        inputs.target(source, "sample").unwrap(),
                        9u32,
                    )],
                }
            });
            inputs.submit(batch.into(), &[]).unwrap();
            for _ in 0..2 {
                let (_, tag, value) = rx.recv().unwrap();
                assert_eq!((tag, value), (1, 9));
            }
            clock.advance_to(PhysicalTimeNanos(6)).unwrap();
            runner.join().unwrap().unwrap();
        });
    });
}

#[test]
fn physical_input_preflight_rejects_invalid_declarations_before_driver_start() {
    for case in 0..8 {
        let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
        let (ready, _) = std::sync::mpsc::channel();
        let (events, _) = std::sync::mpsc::channel();
        let mut config = config();
        match case {
            0 => config.max_batch_values = 0,
            1 => config.max_staged_batches = 0,
            2 => config.sources[1].id = "a".into(),
            3 => config.sources[0].targets.clear(),
            4 => {
                config.sources[0].targets =
                    vec![InputTarget::action::<bool>("sample", "a", "sample")]
            }
            5 => {
                config.sources[0].targets =
                    vec![InputTarget::from_binding("sample", "missing", "sample")]
            }
            6 => {
                config.sources[0].targets =
                    vec![InputTarget::from_binding("sample", "a", "missing")]
            }
            _ => config.sources[0].targets.push(InputTarget::from_binding(
                "duplicate",
                "a",
                "sample",
            )),
        }
        let bindings = bindings(&clock, ready, events, true)
            .with_physical_inputs(config, |_| panic!("invalid configuration started driver"));
        assert!(matches!(
            execute_owned_federate(
                DEPLOYMENT,
                FederateIndex::new(0),
                bindings,
                Config::default()
            ),
            Err(ExecuteOwnedFederateError::PhysicalInput(InputError {
                kind: InputErrorKind::Malformed,
                ..
            }))
        ));
    }
}
