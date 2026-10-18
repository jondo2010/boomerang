//! Compiled two-Enclave physical-input integration and preflight tests.

use super::*;
use boomerang_runtime::{clock::*, input::*};
// Store the physical action in slot 0 and the timer in slot 1.
// Their image rows have the opposite order.
const PHYSICAL: ActionImage = ActionImage::new(
    ScopeIndex::new(0),
    ActionSlotIndex::new(0),
    ActionTiming::Standard {
        domain: TimingDomain::Physical,
        min_delay_nanos: 0,
    },
    r!(1, 1),
    Some(BindingSlotIndex::new(2)),
);
static FAST: [ActionImage; 2] = [fixture_timer_action(1, Some(2), r!(0, 1)), PHYSICAL];
static SLOW: [ActionImage; 2] = [fixture_timer_action(1, Some(3), r!(0, 1)), PHYSICAL];
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
/// Declares two required u32 action sources with two-value and two-batch bounds.
fn config() -> InputConfig {
    InputConfig {
        max_batch_values: 2,
        max_staged_batches: 2,
        sources: ["a", "b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| InputSource {
                id: id.into(),
                required: true,
                actions: vec![CompiledPhysicalAction::new(
                    EnclaveIndex::from(index),
                    ActionIndex::new(1),
                )],
            })
            .collect(),
    }
}
/// Driver bindings handed off before execution; no runtime names are resolved.
#[cfg(feature = "hosted-manual-clock")]
struct DriverInputs {
    admission: InputAdmission,
    sources: Vec<InputSourceHandle>,
    actions: Vec<boomerang_runtime::input::SourceAction<u32>>,
}
#[cfg(feature = "hosted-manual-clock")]
impl std::ops::Deref for DriverInputs {
    type Target = InputAdmission;
    fn deref(&self) -> &Self::Target {
        &self.admission
    }
}
/// Binds Enclaves, selected clock, admission handoff, and reactions with source requiredness.
#[cfg(feature = "hosted-manual-clock")]
fn bindings(
    clock: &ManualClock,
    ready: std::sync::mpsc::Sender<DriverInputs>,
    events: std::sync::mpsc::Sender<(u32, i128, u32)>,
    required: bool,
) -> FederateBindings<'static> {
    let mut config = config();
    for source in config.sources.values_mut() {
        source.required = required;
    }
    let gathered = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut bindings = FederateBindings::new()
        .with_manual_clock(clock.domain(), clock.clone())
        .with_physical_inputs(config);
    for index in 0..2 {
        let events = events.clone();
        let gathered = gathered.clone();
        let ready = ready.clone();
        bindings = bindings.bind_enclave(
            EnclaveIndex::new(index),
            EnclaveBindings::new()
                .bind_state_with_capabilities(
                    BindingSlotIndex::new(0),
                    boomerang_runtime::initialization::StateCapabilities {
                        source: Some(SourceKey::new(index)),
                        actions: vec![boomerang_runtime::initialization::ActionCapability::new(
                            ActionIndex::new(1),
                            PayloadType::<u32>::new(),
                        )],
                        clock_control: false,
                    },
                    move |init| {
                        let source = init.source()?;
                        let action = init.source_action::<u32>(ActionIndex::new(1))?;
                        let mut gathered = gathered.lock().unwrap();
                        gathered.push((source.clone(), action));
                        if gathered.len() == 2 {
                            let (sources, actions) =
                                std::mem::take(&mut *gathered).into_iter().unzip();
                            ready
                                .send(DriverInputs {
                                    admission: source.admission(),
                                    sources,
                                    actions,
                                })
                                .unwrap();
                        }
                        Ok(initialize_counter())
                    },
                )
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
/// Checks coherent input precedes timers and clock advance cannot bypass required progress.
#[test]
#[cfg(feature = "hosted-manual-clock")]
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
        clock.advance_to(PhysicalInstant(12)).unwrap();
        let sources = &inputs.sources;
        inputs
            .submit(
                InputBatch::new(
                    sources
                        .iter()
                        .enumerate()
                        .map(|(index, source)| InputObservation {
                            source: source.clone(),
                            sequence: 1,
                            domain: clock.domain(),
                            epoch: clock.epoch(),
                            acquired: PhysicalInstant(0),
                            values: vec![InputValue::new(&inputs.actions[index], 7u32)],
                        })
                        .collect(),
                )
                .with_progress(
                    sources
                        .iter()
                        .map(|source| (source.clone(), PhysicalInstant(1)))
                        .collect::<Vec<_>>(),
                ),
            )
            .unwrap();
        for _ in 0..2 {
            let (_, tag, value) = rx.recv().unwrap();
            assert_eq!((tag, value), (0, 7));
        }
        inputs.advance(&sources[0], PhysicalInstant(13)).unwrap();
        assert!(rx.try_recv().is_err());
        inputs.advance(&sources[1], PhysicalInstant(7)).unwrap();
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(rx.recv().unwrap());
        }
        seen.sort();
        assert_eq!(
            seen,
            [(0, 2, 0), (0, 4, 0), (0, 6, 0), (1, 3, 0), (1, 6, 0)]
        );
        inputs.advance(&sources[1], PhysicalInstant(13)).unwrap();
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
                .advance(&sources[0], PhysicalInstant(14))
                .unwrap_err()
                .kind,
            InputErrorKind::Disconnected
        );
    });
}
/// Checks input and clock failures release participants blocked before their first grant.
#[test]
#[cfg(feature = "hosted-manual-clock")]
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
                    inputs.disconnect(&inputs.sources[0]);
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

/// Checks optional input interrupts stale grants waiting for clock advancement.
#[test]
#[cfg(feature = "hosted-manual-clock")]
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
            clock.advance_to(PhysicalInstant(1)).unwrap();
            let batch = [0, 1].map(|index| {
                let source = inputs.sources[index].clone();
                InputObservation {
                    source,
                    sequence: 1,
                    domain: clock.domain(),
                    epoch: clock.epoch(),
                    acquired: PhysicalInstant(1),
                    values: vec![InputValue::new(&inputs.actions[index], 9u32)],
                }
            });
            inputs.submit(InputBatch::new(batch.into())).unwrap();
            for _ in 0..2 {
                let (_, tag, value) = rx.recv().unwrap();
                assert_eq!((tag, value), (1, 9));
            }
            clock.advance_to(PhysicalInstant(6)).unwrap();
            runner.join().unwrap().unwrap();
        });
    });
}

/// Rejects invalid input declarations before the driver starts.
#[test]
fn physical_input_preflight_rejects_invalid_declarations_before_driver_start() {
    for case in 0..8 {
        let mut config = config();
        match case {
            0 => config.max_batch_values = 0,
            1 => config.max_staged_batches = 0,
            2 => config.sources[SourceKey::new(1)].id = "a".into(),
            3 => config.sources[SourceKey::new(0)].actions.clear(),
            4 => {
                config.sources[SourceKey::new(0)].actions = vec![CompiledPhysicalAction::new(
                    EnclaveIndex::new(0),
                    ActionIndex::new(0),
                )]
            }
            5 => {
                config.sources[SourceKey::new(0)].actions = vec![CompiledPhysicalAction::new(
                    EnclaveIndex::new(8),
                    ActionIndex::new(1),
                )]
            }
            6 => {
                config.sources[SourceKey::new(0)].actions = vec![CompiledPhysicalAction::new(
                    EnclaveIndex::new(0),
                    ActionIndex::new(8),
                )]
            }
            _ => config.sources[SourceKey::new(0)]
                .actions
                .push(CompiledPhysicalAction::new(
                    EnclaveIndex::new(0),
                    ActionIndex::new(1),
                )),
        }
        let mut bindings = FederateBindings::new().with_physical_inputs(config);
        for index in 0..2 {
            bindings = bindings.bind_enclave(
                EnclaveIndex::new(index),
                EnclaveBindings::new()
                    .bind_state_with_capabilities(
                        BindingSlotIndex::new(0),
                        boomerang_runtime::initialization::StateCapabilities {
                            source: Some(SourceKey::new(index)),
                            actions: vec![
                                boomerang_runtime::initialization::ActionCapability::new(
                                    ActionIndex::new(1),
                                    PayloadType::<u32>::new(),
                                ),
                            ],
                            clock_control: false,
                        },
                        |_| -> Result<CounterState, _> {
                            panic!("invalid input declarations must fail before initialization")
                        },
                    )
                    .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                    .bind_reaction(BindingSlotIndex::new(1), |_, _, _, _| Ok(())),
            );
        }
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

/// Required sources using the host clock initialize at epoch zero before logical startup.
#[test]
fn required_host_clock_source_initializes_at_epoch_zero() {
    for min_delay_nanos in [0, 5] {
        use boomerang_runtime::initialization::StateCapabilities;
        let readers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut inputs = config();
        inputs
            .sources
            .values_mut()
            .for_each(|source| source.required = true);
        let mut bindings = FederateBindings::new().with_physical_inputs(inputs);
        for index in 0..2 {
            let readers = readers.clone();
            bindings = bindings.bind_enclave(
                EnclaveIndex::new(index),
                EnclaveBindings::new()
                    .bind_state_with_capabilities(
                        BindingSlotIndex::new(0),
                        StateCapabilities {
                            source: Some(SourceKey::new(index)),
                            actions: vec![
                                boomerang_runtime::initialization::ActionCapability::new(
                                    ActionIndex::new(1),
                                    PayloadType::<u32>::new(),
                                ),
                            ],
                            clock_control: false,
                        },
                        move |init| {
                            let source = init.source()?;
                            let sample: boomerang_runtime::input::SourceAction<u32> =
                                init.source_action(ActionIndex::new(1))?;
                            assert!(matches!(
                                init.async_action::<u32>(ActionIndex::new(1)),
                                Err(boomerang_runtime::initialization::InitializationError::InvalidCapabilities)
                            ));
                            #[cfg(feature = "hosted-manual-clock")]
                            assert!(matches!(
                                init.clock_control(),
                                Err(boomerang_runtime::initialization::InitializationError::InvalidCapabilities)
                            ));
                            assert!(matches!(
                                init.source_action::<u64>(ActionIndex::new(1)),
                                Err(boomerang_runtime::initialization::InitializationError::InvalidCapabilities)
                            ));
                            let clock = source.clock();
                            assert!(clock.now().is_ok());
                            readers.lock().unwrap().push(clock.clone());
                            source.submit(InputBatch::new(vec![InputObservation {
                                    source: source.clone(),
                                    sequence: 1,
                                    acquired: PhysicalInstant(0),
                                    domain: clock.domain(),
                                    epoch: clock.epoch(),
                                    values: vec![InputValue::new(&sample, 42)],
                                }]).with_progress(vec![(source.clone(), PhysicalInstant(100))]))?;
                            Ok(initialize_counter())
                        },
                    )
                    .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                    .bind_reaction(BindingSlotIndex::new(1), |ctx, state, refs, _| {
                        let (mut sample,): (ActionRef<u32>,) = refs.actions.partition_mut()?;
                        if let Some(value) = ctx.get_action_value(&mut sample) {
                            let state = state.downcast_mut::<CounterState>().unwrap();
                            state.count += *value as usize;
                            state.tags.push(ctx.get_tag());
                            ctx.schedule_shutdown(None);
                        }
                        Ok(())
                    }),
            );
        }
        let physical = ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(0),
            ActionTiming::Standard {
                domain: TimingDomain::Physical,
                min_delay_nanos,
            },
            r!(1, 1),
            Some(BindingSlotIndex::new(2)),
        );
        let fast = [FAST[0].clone(), physical.clone()];
        let slow = [SLOW[0].clone(), physical];
        let enclaves = [
            EnclaveImage {
                actions: TinyMapRef::from_slice(&fast),
                ..ENCLAVES[0].clone()
            },
            EnclaveImage {
                actions: TinyMapRef::from_slice(&slow),
                ..ENCLAVES[1].clone()
            },
        ];
        let result = execute_owned_federate(
            CompiledDeploymentImage {
                enclaves: TinyMapRef::from_slice(&enclaves),
                ..DEPLOYMENT
            },
            FederateIndex::new(0),
            bindings,
            Config::default().with_fast_forward(true),
        )
        .unwrap();
        for reader in readers.lock().unwrap().iter() {
            assert_eq!(
                reader.now(),
                Err(boomerang_runtime::clock::PhysicalClockError::Closed)
            );
        }
        for index in 0..2 {
            let state = result
                .enclave(EnclaveIndex::new(index))
                .unwrap()
                .state::<CounterState>(StateSlotIndex::new(0))
                .unwrap();
            assert_eq!(state.count, 42);
            assert_eq!(
                state.tags,
                [Tag::new(Duration::nanoseconds(min_delay_nanos as i64), 0)]
            );
        }
    }
}

/// A bad later payload binding must not run even an earlier valid initializer.
#[test]
fn all_capability_payloads_are_checked_before_state_side_effects() {
    use boomerang_runtime::initialization::{ActionCapability, StateCapabilities};
    let initialized = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut bindings = FederateBindings::new().with_physical_inputs(config());
    for index in 0..2 {
        let initialized = initialized.clone();
        let action = if index == 0 {
            ActionCapability::new(ActionIndex::new(1), PayloadType::<u32>::new())
        } else {
            ActionCapability::new(ActionIndex::new(1), PayloadType::<u64>::new())
        };
        bindings = bindings.bind_enclave(
            EnclaveIndex::new(index),
            EnclaveBindings::new()
                .bind_state_with_capabilities(
                    BindingSlotIndex::new(0),
                    StateCapabilities {
                        source: Some(SourceKey::new(index)),
                        actions: vec![action],
                        clock_control: false,
                    },
                    move |_| {
                        initialized.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(initialize_counter())
                    },
                )
                .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                .bind_reaction(BindingSlotIndex::new(1), |_, _, _, _| Ok(())),
        );
    }
    assert!(matches!(
        execute_owned_federate(
            DEPLOYMENT,
            FederateIndex::new(0),
            bindings,
            Config::default()
        ),
        Err(ExecuteOwnedFederateError::PhysicalInput(_))
    ));
    assert_eq!(initialized.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// A state guard must be dropped only after its blocked ordinary sender is released.
#[derive(Debug)]
struct InitializationWorker {
    worker: Option<std::thread::JoinHandle<bool>>,
    dropped: std::sync::mpsc::Sender<bool>,
}
impl Drop for InitializationWorker {
    fn drop(&mut self) {
        let accepted = self.worker.take().unwrap().join().unwrap();
        self.dropped.send(accepted).unwrap();
    }
}
/// Failure and panic close mailboxes before dropping states returned by earlier initializers.
#[test]
fn initializer_failure_and_panic_release_previously_initialized_workers() {
    bounded(|| {
        use boomerang_runtime::initialization::StateCapabilities;
        for panic in [false, true] {
            let readers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let retained_readers = readers.clone();
            let (dropped, received) = std::sync::mpsc::channel();
            let first = EnclaveBindings::new()
                .bind_state_with_capabilities(
                    BindingSlotIndex::new(0),
                    StateCapabilities {
                        source: Some(SourceKey::new(0)),
                        actions: vec![boomerang_runtime::initialization::ActionCapability::new(
                            ActionIndex::new(1),
                            PayloadType::<u32>::new(),
                        )],
                        clock_control: false,
                    },
                    move |init| {
                        readers.lock().unwrap().push(init.source()?.clock());
                        let mut send = init.send_context();
                        while send.try_shutdown(None).is_ok() {}
                        assert_eq!(
                            send.try_shutdown(None),
                            Err(boomerang_runtime::TryShutdownError::Full)
                        );
                        let worker = std::thread::spawn(move || send.schedule_shutdown(None));
                        Ok(InitializationWorker {
                            worker: Some(worker),
                            dropped: dropped.clone(),
                        })
                    },
                )
                .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                .bind_reaction(BindingSlotIndex::new(1), |_, _, _, _| Ok(()));
            let second = EnclaveBindings::new()
                .bind_state_with_capabilities(
                    BindingSlotIndex::new(0),
                    StateCapabilities {
                        source: Some(SourceKey::new(1)),
                        actions: vec![boomerang_runtime::initialization::ActionCapability::new(
                            ActionIndex::new(1),
                            PayloadType::<u32>::new(),
                        )],
                        clock_control: false,
                    },
                    move |_| -> Result<CounterState, _> {
                        assert!(!panic, "injected initializer panic");
                        Err(std::io::Error::other("injected initializer failure").into())
                    },
                )
                .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                .bind_reaction(BindingSlotIndex::new(1), |_, _, _, _| Ok(()));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute_owned_federate(
                    DEPLOYMENT,
                    FederateIndex::new(0),
                    FederateBindings::new()
                        .with_physical_inputs(config())
                        .bind_enclave(EnclaveIndex::new(0), first)
                        .bind_enclave(EnclaveIndex::new(1), second),
                    Config::default(),
                )
            }));
            if panic {
                assert!(result.is_err());
            } else {
                assert!(matches!(
                    result.unwrap(),
                    Err(ExecuteOwnedFederateError::EnclaveInitialization { .. })
                ));
            }
            for reader in retained_readers.lock().unwrap().iter() {
                assert_eq!(
                    reader.now(),
                    Err(boomerang_runtime::clock::PhysicalClockError::Closed)
                );
            }
            assert!(
                !received.recv().unwrap(),
                "closed mailbox must reject the blocked sender"
            );
        }
    });
}

/// An initializer owns cancellation until it successfully returns its worker in state.
#[test]
fn initializer_local_worker_guard_cancels_on_error_and_panic() {
    #[derive(Debug)]
    struct LocalGuard {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
        joined: std::sync::mpsc::Sender<()>,
    }
    impl Drop for LocalGuard {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            self.worker.take().unwrap().join().unwrap();
            self.joined.send(()).unwrap();
        }
    }
    bounded(|| {
        for panic in [false, true] {
            let (joined, received) = std::sync::mpsc::channel();
            let mut bindings = FederateBindings::new();
            for index in 0..2 {
                let joined = joined.clone();
                bindings = bindings.bind_enclave(
                    EnclaveIndex::new(index),
                    EnclaveBindings::new()
                        .bind_state_with_capabilities(
                            BindingSlotIndex::new(0),
                            Default::default(),
                            move |init| -> Result<CounterState, _> {
                                let stop =
                                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                                let worker_stop = stop.clone();
                                let send = init.send_context();
                                let _guard = LocalGuard {
                                    stop,
                                    joined: joined.clone(),
                                    worker: Some(std::thread::spawn(move || {
                                        // The runtime remains live during initializer-local unwinding.
                                        while !worker_stop
                                            .load(std::sync::atomic::Ordering::Acquire)
                                        {
                                            let _ = send.try_shutdown(None);
                                            std::thread::yield_now();
                                        }
                                    })),
                                };
                                assert!(!panic, "injected local initializer panic");
                                Err(std::io::Error::other("injected local initializer failure")
                                    .into())
                            },
                        )
                        .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
                        .bind_reaction(BindingSlotIndex::new(1), |_, _, _, _| Ok(())),
                );
            }
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute_owned_federate(
                    DEPLOYMENT,
                    FederateIndex::new(0),
                    bindings,
                    Config::default(),
                )
            }));
            if panic {
                assert!(outcome.is_err());
            } else {
                assert!(outcome.unwrap().is_err());
            }
            received.recv().unwrap();
        }
    });
}
