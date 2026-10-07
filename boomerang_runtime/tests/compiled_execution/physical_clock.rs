use super::*;
use boomerang_runtime::{physical_clock::ManualClock, physical_time::*};

static FAST: [ActionImage; 1] = [fixture_timer_action(0, Some(2), r!(0, 1))];
static SLOW: [ActionImage; 1] = [fixture_timer_action(0, Some(3), r!(0, 1))];
static START: [TimerStartupImage; 1] = [TimerStartupImage::new(ActionIndex::new(0), 0)];
static CLOCK_ENCLAVES: [EnclaveImage<'static>; 2] = [
    EnclaveImage {
        enclave_id: EnclaveId::new("fast"),
        actions: TinyMapRef::from_slice(&FAST),
        timer_startup_actions: &START,
        ..IMAGE
    },
    EnclaveImage {
        enclave_id: EnclaveId::new("slow"),
        actions: TinyMapRef::from_slice(&SLOW),
        timer_startup_actions: &START,
        ..IMAGE
    },
];
static CLOCK_FEDERATES: [FederateImage; 1] = [fixture_federate(
    "clock",
    "host",
    "std",
    IndexSpan::new(0, 2),
)];
const CLOCK_DEPLOYMENT: CompiledDeploymentImage<'static> = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&[FederateIndex::new(0)], &[]),
    federates: TinyMapRef::from_slice(&CLOCK_FEDERATES),
    enclaves: TinyMapRef::from_slice(&CLOCK_ENCLAVES),
    coordination: CoordinationProjection::Local,
};

#[test]
fn physical_clock_two_enclaves_preserve_all_timer_tags_across_jumps() {
    bounded(|| {
        let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut bindings =
            FederateBindings::new().with_physical_clock(clock.domain(), clock.clone());
        for enclave in 0..2 {
            let tx = tx.clone();
            bindings = bindings.bind_enclave(
                EnclaveIndex::new(enclave),
                EnclaveBindings::new()
                    .bind_state(BindingSlotIndex::new(0), initialize_counter)
                    .bind_reaction(
                        BindingSlotIndex::new(1),
                        move |ctx: &mut Context,
                              state: &mut dyn ReactorData,
                              _: ReactionRefs<'_>,
                              _: Option<CompiledModeEffectRef>| {
                            let time = ctx.physical_time().unwrap().unwrap();
                            assert_eq!(
                                ctx.try_get_physical_time().unwrap(),
                                time.to_instant(ctx.get_start_time()).unwrap()
                            );
                            state
                                .downcast_mut::<CounterState>()
                                .unwrap()
                                .tags
                                .push(ctx.get_tag());
                            tx.send((enclave, ctx.get_tag().offset().whole_nanoseconds(), time.0))
                                .unwrap();
                            Ok(())
                        },
                    ),
            );
        }
        let runner = std::thread::spawn(move || {
            execute_owned_federate(
                CLOCK_DEPLOYMENT,
                FederateIndex::new(0),
                bindings,
                Config::default().with_timeout(Duration::nanoseconds(12)),
            )
        });
        let mut seen = vec![rx.recv().unwrap(), rx.recv().unwrap()];
        assert!(seen.iter().all(|(_, tag, time)| *tag == 0 && *time == 0));
        for (jump, count) in [(6, 5), (12, 3)] {
            clock.advance_to(PhysicalTimeNanos(jump)).unwrap();
            clock.advance_to(PhysicalTimeNanos(jump)).unwrap();
            for _ in 0..count {
                seen.push(rx.recv().unwrap());
            }
        }
        let result = runner.join().unwrap().unwrap();
        for (enclave, expected) in [(0, vec![0, 2, 4, 6, 8, 10]), (1, vec![0, 3, 6, 9])] {
            let actual: Vec<_> = result
                .enclave(EnclaveIndex::new(enclave))
                .unwrap()
                .state::<CounterState>(StateSlotIndex::new(0))
                .unwrap()
                .tags
                .iter()
                .map(|tag| tag.offset().whole_nanoseconds())
                .collect();
            assert_eq!(actual, expected);
        }
        assert!(seen.iter().all(|(_, tag, time)| *tag <= i128::from(*time)));
    });
}

#[test]
fn physical_clock_failure_releases_physical_and_idle_coordination_waits() {
    bounded(|| {
        for idle in [false, true] {
            let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
            let observations: Vec<_> = (0..2)
                .map(|i| {
                    (
                        EnclaveIndex::new(i),
                        Arc::new(boomerang_runtime::ObservationState::new(Instant::now())),
                    )
                })
                .collect();
            let watched = observations.clone();
            let enclaves = [
                EnclaveImage {
                    timer_startup_actions: &[],
                    ..CLOCK_ENCLAVES[0]
                },
                EnclaveImage {
                    timer_startup_actions: &[],
                    ..CLOCK_ENCLAVES[1]
                },
            ];
            let mut bindings =
                FederateBindings::new().with_physical_clock(clock.domain(), clock.clone());
            for i in 0..2 {
                bindings = bindings.bind_enclave(EnclaveIndex::new(i), reference_bindings());
            }
            std::thread::scope(|scope| {
                let runner = scope.spawn(|| {
                    boomerang_runtime::execute_owned_federate_with_observations(
                        CompiledDeploymentImage {
                            enclaves: if idle {
                                TinyMapRef::from_slice(&enclaves)
                            } else {
                                CLOCK_DEPLOYMENT.enclaves
                            },
                            ..CLOCK_DEPLOYMENT
                        },
                        FederateIndex::new(0),
                        bindings,
                        Config::default().with_keep_alive(true),
                        &observations,
                    )
                });
                let bound = Instant::now() + std::time::Duration::from_secs(3);
                loop {
                    let phases: Vec<_> = watched
                        .iter()
                        .filter_map(|(_, obs)| {
                            obs.snapshot(Instant::now()).map(|snap| snap.current_phase)
                        })
                        .collect();
                    if (idle
                        && phases
                            .iter()
                            .all(|p| *p == boomerang_runtime::SchedulerPhase::CoordinationWait))
                        || (!idle
                            && phases.contains(&boomerang_runtime::SchedulerPhase::PhysicalWait))
                    {
                        break;
                    }
                    assert!(
                        Instant::now() < bound,
                        "schedulers never entered the expected wait: {phases:?}"
                    );
                    std::thread::yield_now();
                }
                if idle {
                    clock.close();
                } else {
                    clock.fail();
                }
                assert!(
                    matches!(runner.join().unwrap(), Err(ExecuteOwnedFederateError::PhysicalClock(error)) if error == if idle { PhysicalClockError::Closed } else { PhysicalClockError::Failed })
                );
            });
        }
    });
}

#[test]
fn physical_clock_preflight_rejects_domain_closed_and_reuse() {
    let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
    let run = |domain| {
        let bindings = (0..2).fold(
            FederateBindings::new().with_physical_clock(domain, clock.clone()),
            |bindings, i| bindings.bind_enclave(EnclaveIndex::new(i), reference_bindings()),
        );
        execute_owned_federate(
            CLOCK_DEPLOYMENT,
            FederateIndex::new(0),
            bindings,
            Config::default()
                .with_fast_forward(true)
                .with_timeout(Duration::nanoseconds(1)),
        )
    };
    assert!(matches!(
        run(PhysicalClockDomainId(8)),
        Err(ExecuteOwnedFederateError::PhysicalClock(
            PhysicalClockError::DomainMismatch
        ))
    ));
    run(clock.domain()).unwrap();
    assert!(matches!(
        run(clock.domain()),
        Err(ExecuteOwnedFederateError::PhysicalClock(
            PhysicalClockError::AlreadyUsed
        ))
    ));
    clock.close();
    assert!(matches!(
        run(clock.domain()),
        Err(ExecuteOwnedFederateError::PhysicalClock(
            PhysicalClockError::Closed
        ))
    ));
}

#[test]
fn physical_clock_rejects_mailboxes_without_a_retained_wake_slot() {
    let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
    let enclaves = [
        EnclaveImage {
            storage_bounds: &StorageBounds::new(1, 1, 0, 0, 0, 0),
            ..CLOCK_ENCLAVES[0]
        },
        CLOCK_ENCLAVES[1].clone(),
    ];
    let bindings = (0..2).fold(
        FederateBindings::new().with_physical_clock(clock.domain(), clock),
        |bindings, i| bindings.bind_enclave(EnclaveIndex::new(i), reference_bindings()),
    );
    let result = execute_owned_federate(
        CompiledDeploymentImage {
            enclaves: TinyMapRef::from_slice(&enclaves),
            ..CLOCK_DEPLOYMENT
        },
        FederateIndex::new(0),
        bindings,
        Config::default(),
    );
    assert!(matches!(
        result,
        Err(ExecuteOwnedFederateError::PhysicalClock(
            PhysicalClockError::WakeCapacity
        ))
    ));
}
