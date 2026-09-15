//! Scheduler panic, route failure precedence, and bounded peer shutdown.
use super::*;

static RECURRING_ABORT_PEER_READY: AtomicBool = AtomicBool::new(false);

fn signal_recurring_abort_peer_ready(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    RECURRING_ABORT_PEER_READY.store(true, Ordering::SeqCst);
    Ok(())
}

fn panic_after_recurring_abort_peer_starts(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    while !RECURRING_ABORT_PEER_READY.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    panic!("peer scheduler panic");
}

fn recurring_abort_peer_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(BindingSlotIndex::new(1), signal_recurring_abort_peer_ready)
}

fn aborting_peer_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_counter)
        .bind_reaction(
            BindingSlotIndex::new(1),
            panic_after_recurring_abort_peer_starts,
        )
}
fn panic_on_routed_value(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    panic!("routed sink panic")
}

fn panicking_sink_bindings() -> EnclaveBindings {
    EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_routed_sink)
        .bind_reaction(BindingSlotIndex::new(1), panic_on_routed_value)
        .bind_port(BindingSlotIndex::new(2), PayloadType::<u32>::new())
}

static COMPETING_PANIC_READY: AtomicBool = AtomicBool::new(false);

fn synchronized_route_source(
    _context: &mut Context,
    _state: &mut dyn ReactorData,
    refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    while !COMPETING_PANIC_READY.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let mut output: OutputRef<u32> = refs.ports_mut.partition_mut()?;
    *output = Some(42);
    Ok(())
}

fn competing_panic_after_route_failure(
    context: &mut Context,
    _state: &mut dyn ReactorData,
    _refs: ReactionRefs<'_>,
    _mode_effect: Option<CompiledModeEffectRef>,
) -> Result<(), ReactionBindingError> {
    COMPETING_PANIC_READY.store(true, Ordering::SeqCst);
    assert!(!context.schedule_external(AsyncEvent::Shutdown {
        delay: Duration::ZERO,
    }));
    panic!("competing scheduler panic");
}

#[test]
fn owned_federate_panic_requests_bounded_shutdown_and_joins() {
    let started = Instant::now();
    let error = execute_owned_federate(
        ROUTED_DEPLOYMENT,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(EnclaveIndex::new(0), source_bindings())
            .bind_enclave(EnclaveIndex::new(1), panicking_sink_bindings())
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ExecuteOwnedFederateError::ThreadPanicked { enclave, ref message }
            if enclave == EnclaveIndex::new(1) && message == "routed sink panic"
    ));
    assert!(started.elapsed() < owned_federate_watchdog_timeout());
}

#[test]
#[cfg_attr(
    miri,
    ignore = "the bounded hang regression requires subprocess support"
)]
fn owned_federate_abort_stops_peer_with_recurring_internal_work() {
    let executable = std::env::current_exe().expect("integration test executable is available");
    let mut child = std::process::Command::new(executable)
        .args([
            "--exact",
            "failure::owned_federate_abort_stops_peer_with_recurring_internal_work_child",
            "--nocapture",
        ])
        .env("BOOMERANG_RECURRING_ABORT_CHILD", "1")
        .spawn()
        .expect("recurring-abort child process starts");
    let deadline = Instant::now() + std::time::Duration::from_secs(2);

    loop {
        if let Some(status) = child
            .try_wait()
            .expect("recurring-abort child can be polled")
        {
            assert!(
                status.success(),
                "recurring-abort child failed with {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .expect("timed-out recurring-abort child is killed");
            child
                .wait()
                .expect("killed recurring-abort child is reaped");
            panic!("Federate abort did not stop its recurring-work scheduler within two seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn owned_federate_abort_stops_peer_with_recurring_internal_work_child() {
    if std::env::var_os("BOOMERANG_RECURRING_ABORT_CHILD").is_none() {
        return;
    }

    let recurring_startup = [TimerStartupImage::new(ActionIndex::new(0), 0)];
    let recurring = EnclaveImage {
        enclave_id: EnclaveId::new("compiled/abortpeer"),
        actions: TinyMapView::new(&PERIODIC_ACTIONS),
        timer_startup_actions: &recurring_startup,
        ..IMAGE
    };
    let panicking = EnclaveImage {
        enclave_id: EnclaveId::new("compiled/panicpeer"),
        ..IMAGE
    };
    let enclaves = [recurring, panicking];
    let federates = [fixture_federate("host", "target", "runtime", s!(0, 2))];
    let members = [FederateIndex::new(0)];
    for keep_alive in [false, true] {
        let deployment = CompiledDeploymentImage {
            federation: GlobalFederationImage::new(&members, &[]),
            federates: TinyMapView::new(&federates),
            enclaves: TinyMapView::new(&enclaves),
            coordination: CoordinationProjection::Local,
        };
        RECURRING_ABORT_PEER_READY.store(false, Ordering::SeqCst);
        let error = execute_owned_federate(
            deployment,
            FederateIndex::new(0),
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(0), recurring_abort_peer_bindings())
                .bind_enclave(EnclaveIndex::new(1), aborting_peer_bindings()),
            Config::default()
                .with_fast_forward(true)
                .with_keep_alive(keep_alive),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ExecuteOwnedFederateError::ThreadPanicked { enclave, ref message }
                if enclave == EnclaveIndex::new(1) && message == "peer scheduler panic"
        ));
    }
}

#[test]
fn owned_federate_retains_route_failure_before_competing_scheduler_panic() {
    COMPETING_PANIC_READY.store(false, Ordering::SeqCst);
    let outbound = [fixture_route(
        "pipe",
        PortIndex::new(0),
        RouteDirection::Outbound,
        TimingDomain::Physical,
        0,
    )];
    let inbound = [fixture_route(
        "pipe",
        PortIndex::new(0),
        RouteDirection::Inbound,
        TimingDomain::Physical,
        0,
    )];
    let source = EnclaveImage {
        routes: TinyMapView::new(&outbound),
        ..ROUTED_SOURCE_IMAGE
    };
    let destination = EnclaveImage {
        enclave_id: EnclaveId::new("delta"),
        routes: TinyMapView::new(&inbound),
        storage_bounds: &StorageBounds::new(1, 1, 0, 0, 0, 0),
        ..ROUTED_SOURCE_IMAGE
    };
    let enclaves = [source, destination];
    let deployment = CompiledDeploymentImage {
        enclaves: TinyMapView::new(&enclaves),
        ..ROUTED_DEPLOYMENT
    };
    let started = Instant::now();
    let error = execute_owned_federate(
        deployment,
        FederateIndex::new(0),
        FederateBindings::new()
            .bind_enclave(
                EnclaveIndex::new(0),
                routed_reaction_bindings(synchronized_route_source),
            )
            .bind_enclave(
                EnclaveIndex::new(1),
                routed_reaction_bindings(competing_panic_after_route_failure),
            )
            .bind_route(
                route_boundary(),
                PayloadType::<u32>::new(),
                PayloadType::<u32>::new(),
            ),
        Config::default().with_fast_forward(true),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            ExecuteOwnedFederateError::EnclaveExecution {
                enclave,
                source: OwnedStorageError::OutboundRouteChannelFull { destination, .. },
            } if *enclave == EnclaveIndex::new(0) && *destination == EnclaveIndex::new(1)
        ),
        "{error:?}"
    );
    assert!(started.elapsed() < owned_federate_watchdog_timeout());
}
