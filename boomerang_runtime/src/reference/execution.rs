//! Prepare compiled Federate execution and supervise its worker threads.
//!
//! Validate capabilities before user initialization. Start the coordinator before
//! deferred initializers submit input. Open execution only after all initializers succeed.
//! Close run resources before returned reactor state or worker guards are dropped.

use super::*;

/// Prepare validated storage and supervise execution with the selected backend.
pub(super) fn execute_prepared_federate<'image, B: FederateCoordinationBackend>(
    prepared: PreparedFederate<'image>,
    bindings: FederateBindings<'_>,
    config: Config,
    observations: &[(EnclaveIndex, crate::ObservationHandle)],
    lifecycle: LifecyclePolicy,
    connect: impl FnOnce(
        &mut [(EnclaveIndex, OwnedStorage<'image>)],
    ) -> Result<B, ExecuteOwnedFederateError>,
    fail_spawn: impl FnMut(Option<EnclaveIndex>) -> bool,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    let observations = prepare_observations(&prepared.images, observations)?;
    #[cfg(feature = "hosted-source-input")]
    let lifecycle = if lifecycle == LifecyclePolicy::TerminateWhenIdle
        && bindings
            .physical_inputs
            .as_ref()
            .is_some_and(|config| config.sources.values().any(|source| source.required))
    {
        LifecyclePolicy::KeepAlive
    } else {
        lifecycle
    };
    #[cfg(feature = "hosted-manual-clock")]
    if bindings.manual_clock.is_some()
        && prepared
            .images
            .values()
            .any(|image| image.storage_bounds().event_capacity() == 0)
    {
        return Err(crate::clock::PhysicalClockError::WakeCapacity.into());
    }
    #[cfg(feature = "hosted-manual-clock")]
    let clock_run = crate::clock::ClockRun::new(bindings.manual_clock.as_ref())?;
    #[cfg(feature = "hosted-source-input")]
    let input_targets = bindings
        .physical_inputs
        .as_ref()
        .map(|config| config.validate(&prepared.images))
        .transpose()?;
    validate_capabilities(&prepared.images, &bindings)?;
    let deferred_initialization = bindings
        .enclaves
        .values()
        .any(EnclaveBindings::has_capabilities);

    let PreparedFederate { images, endpoints } = prepared;
    let FederateBindings {
        enclaves,
        duplicate_enclaves: _,
        #[cfg(feature = "hosted-source-input")]
        physical_inputs,
        routes,
        ..
    } = bindings;
    let route_bindings = resolve_route_bindings(&endpoints, &routes);
    let mut storage_phase = ConstructionPhase::started("storage");
    let participant_indices = images.keys().collect::<Vec<_>>();
    let mut storages = prepare_storages(
        images,
        enclaves,
        deferred_initialization,
        &mut storage_phase,
    )?;
    let mut route_phase = ConstructionPhase::started("routes");
    let event_senders = storages
        .iter()
        .map(|(_, storage)| storage.scheduler_event_tx())
        .collect::<Vec<_>>();
    install_local_routes(&mut storages, &endpoints, &routes, &route_bindings);
    route_phase.completed();

    let coordinations = build_dependencies(&storages, &endpoints);
    let FederateCoordinationParts {
        abort_handle,
        coordinator,
        participants,
        ..
    } = connect_coordinator(&mut storages, &participant_indices, lifecycle, connect)?;
    #[cfg(feature = "hosted-source-input")]
    let mut coordinator = coordinator;
    #[cfg(feature = "hosted-source-input")]
    let mut participants = participants;
    let origin = Instant::now();
    #[cfg(feature = "hosted-manual-clock")]
    if let Some(clock) = &clock_run.0 {
        clock.attach(&event_senders, abort_handle.clone())?;
        for (slot, (_, storage)) in storages.iter_mut().enumerate() {
            storage.set_clock(crate::clock::ExecutionClock::manual(clock.clone(), slot));
        }
    }
    for (_, storage) in &storages {
        storage.initialize_clock(origin);
    }
    #[cfg(any(feature = "hosted-source-input", feature = "hosted-manual-clock"))]
    let selected_clock = {
        let host_clock = || {
            let clock = storages
                .first()
                .expect("validated nonempty Federate")
                .1
                .scheduler_send_context()
                .clock;
            crate::clock::SelectedClock::host_monotonic(clock)
        };
        #[cfg(feature = "hosted-manual-clock")]
        {
            match &clock_run.0 {
                Some(clock) => clock.clone().into(),
                None => host_clock()?,
            }
        }
        #[cfg(not(feature = "hosted-manual-clock"))]
        host_clock()?
    };
    #[cfg(feature = "hosted-source-input")]
    let inputs = physical_inputs.map(|config| {
        let (endpoint, processor) = crate::input::AdmissionProcessor::new(
            config,
            input_targets.unwrap(),
            selected_clock.clone(),
            storages
                .iter()
                .map(|(key, storage)| (*key, storage.scheduler_event_tx()))
                .collect(),
            &mut coordinator.state,
            abort_handle.clone(),
        );
        coordinator.inputs = Some(processor);
        for (_, port) in participants.iter_mut() {
            port.input_enabled = true;
        }
        endpoint
    });
    let shutdown = RunShutdown {
        abort: abort_handle.clone(),
        senders: event_senders.clone(),
        #[cfg(feature = "hosted-source-input")]
        inputs: inputs.clone(),
        #[cfg(feature = "hosted-manual-clock")]
        clock: clock_run.0.clone(),
        #[cfg(feature = "hosted-manual-clock")]
        clock_failure: Default::default(),
        #[cfg(any(feature = "hosted-source-input", feature = "hosted-manual-clock"))]
        selected_clock,
    };
    let _cleanup = CloseRunOnDrop(shutdown.clone());
    let (results, failure) = execute_workers(
        PreparedWorkers {
            storages,
            coordinations,
            coordinator,
            participants,
        },
        &config,
        &observations,
        origin,
        &shutdown,
        &mut storage_phase,
        fail_spawn,
    );

    // Results now own returned states; close their capabilities before any early return.
    shutdown.close();
    finish_execution(results, failure, origin, &shutdown)
}

/// Check all initializer capabilities before constructing reactor state.
fn validate_capabilities(
    images: &TinySecondaryMap<EnclaveIndex, EnclaveImageView<'_>>,
    bindings: &FederateBindings<'_>,
) -> Result<(), ExecuteOwnedFederateError> {
    #[cfg(feature = "hosted-manual-clock")]
    {
        let mut writers = 0;
        for (enclave, owned) in bindings.enclaves.iter() {
            writers += owned
                .validate_clock_capabilities(&images[enclave], bindings.manual_clock.is_some())
                .map_err(|source| ExecuteOwnedFederateError::EnclaveInitialization {
                    enclave,
                    source,
                })?;
            if writers > 1 {
                return Err(ExecuteOwnedFederateError::EnclaveInitialization {
                    enclave,
                    source: crate::initialization::InitializationError::InvalidCapabilities.into(),
                });
            }
        }
    }
    #[cfg(feature = "hosted-source-input")]
    {
        let mut claimed = std::collections::BTreeSet::new();
        for (enclave, owned) in bindings.enclaves.iter() {
            let sources = owned.validate_source_capabilities(
                &images[enclave],
                enclave,
                bindings.physical_inputs.as_ref(),
            )?;
            for source in sources {
                if !claimed.insert(source) {
                    return Err(crate::input::InputError::configuration_error().into());
                }
            }
        }
        if let Some(config) = &bindings.physical_inputs {
            if config.sources.keys().any(|key| !claimed.contains(&key)) {
                return Err(crate::input::InputError::configuration_error().into());
            }
        }
    }
    for (enclave, owned) in bindings.enclaves.iter() {
        owned
            .validate_ordinary_capabilities(&images[enclave])
            .map_err(|source| ExecuteOwnedFederateError::EnclaveInitialization {
                enclave,
                source,
            })?;
    }
    Ok(())
}

/// Construct storage and retain deferred initializers until admission is ready.
fn prepare_storages<'image>(
    images: TinySecondaryMap<EnclaveIndex, EnclaveImageView<'image>>,
    enclaves: TinySecondaryMap<EnclaveIndex, EnclaveBindings>,
    deferred_initialization: bool,
    phase: &mut ConstructionPhase,
) -> Result<Vec<(EnclaveIndex, OwnedStorage<'image>)>, ExecuteOwnedFederateError> {
    let mut storages = Vec::with_capacity(images.len());
    for ((enclave, image), (binding_enclave, owned)) in images.into_iter().zip(enclaves) {
        assert_eq!(
            enclave, binding_enclave,
            "preflight must align every selected Enclave with its bindings"
        );
        let span = tracing::debug_span!(target: "boomerang::runtime", "runtime.enclave",
            enclave = enclave.as_u32(), enclave_id = image.enclave_id().as_str());
        let _span = span.enter();
        let enclave_key = runtime_enclave_key(enclave);
        let storage = match if deferred_initialization {
            OwnedStorage::prepare_for_enclave(image, owned, enclave_key)
        } else {
            OwnedStorage::new_for_enclave(image, owned, enclave_key)
        } {
            Ok(storage) => storage,
            Err(source) => {
                phase.failed("initialization");
                return Err(ExecuteOwnedFederateError::EnclaveInitialization { enclave, source });
            }
        };
        storages.push((enclave, storage));
    }
    if !deferred_initialization {
        phase.completed();
    }
    Ok(storages)
}

/// Install each checked local route in its source storage.
fn install_local_routes<'image>(
    storages: &mut [(EnclaveIndex, OwnedStorage<'image>)],
    endpoints: &[ResolvedLocalRoute<'image>],
    routes: &[Box<dyn RouteBinding + '_>],
    route_bindings: &BTreeMap<LocalRouteKey, usize>,
) {
    for endpoint in endpoints {
        let route = &routes[route_bindings[&endpoint.key]];
        let destination_tx = storages
            .iter()
            .find(|(enclave, _)| *enclave == endpoint.destination)
            .map(|(_, storage)| storage.scheduler_event_tx())
            .expect("Federate preflight required destination storage");
        let source = storages
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.source)
            .map(|(_, storage)| storage)
            .expect("Federate preflight required source storage");
        route.install(source, endpoint, destination_tx);
    }
}

/// Connect logical upstream and downstream scheduler dependencies.
fn build_dependencies(
    storages: &[(EnclaveIndex, OwnedStorage<'_>)],
    endpoints: &[ResolvedLocalRoute<'_>],
) -> Vec<(EnclaveIndex, EnclaveDependencies)> {
    let scheduler_contexts = storages
        .iter()
        .map(|(enclave, storage)| (*enclave, storage.scheduler_send_context()))
        .collect::<Vec<_>>();
    let mut coordinations = storages
        .iter()
        .map(|(enclave, _)| {
            (
                *enclave,
                EnclaveDependencies::new(runtime_enclave_key(*enclave)),
            )
        })
        .collect::<Vec<_>>();
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| endpoint.timing_domain == TimingDomain::Logical)
    {
        let source_key = runtime_enclave_key(endpoint.source);
        let destination_key = runtime_enclave_key(endpoint.destination);
        let source_context = scheduler_contexts
            .iter()
            .find_map(|(enclave, context)| (*enclave == endpoint.source).then(|| context.clone()))
            .expect("selected route source has a configured scheduler context");
        let destination_context = scheduler_contexts
            .iter()
            .find_map(|(enclave, context)| {
                (*enclave == endpoint.destination).then(|| context.clone())
            })
            .expect("selected route destination has a configured scheduler context");
        let delay = (endpoint.delay_nanos != 0)
            .then(|| crate::Duration::nanoseconds(endpoint.delay_nanos as i64));
        coordinations
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.source)
            .expect("selected route source has scheduler coordination")
            .1
            .add_downstream(destination_key, destination_context);
        coordinations
            .iter_mut()
            .find(|(enclave, _)| *enclave == endpoint.destination)
            .expect("selected route destination has scheduler coordination")
            .1
            .add_upstream(source_key, source_context, delay);
    }
    coordinations
}

/// Run deferred initializers while the coordinator accepts input and execution remains closed.
fn initialize_states(
    storages: &mut [(EnclaveIndex, OwnedStorage<'_>)],
    shutdown: &RunShutdown,
    phase: &mut ConstructionPhase,
) -> Result<(), ExecuteOwnedFederateError> {
    for (enclave, storage) in storages {
        let initialized = std::panic::catch_unwind(AssertUnwindSafe(|| {
            storage.initialize_states(
                Some(*enclave),
                #[cfg(feature = "hosted-source-input")]
                shutdown.inputs.as_ref(),
                #[cfg(feature = "hosted-manual-clock")]
                shutdown.clock.as_ref(),
            )
        }));
        match initialized {
            Ok(Ok(())) => {}
            Ok(Err(source)) => {
                phase.failed("initialization");
                shutdown.close();
                return Err(ExecuteOwnedFederateError::EnclaveInitialization {
                    enclave: *enclave,
                    source,
                });
            }
            Err(payload) => {
                shutdown.close();
                std::panic::resume_unwind(payload);
            }
        }
    }
    if phase.active {
        phase.completed();
    }
    Ok(())
}

/// Run one scheduler and report each failure through its responsible owner.
fn run_enclave(
    enclave: EnclaveIndex,
    storage: &mut WorkerStorage<'_>,
    config: &Config,
    origin: Instant,
    coordination: EnclaveDependencies,
    participant: &mut crate::sched::federate::EnclaveCoordinationPort,
    observation: Option<&crate::ObservationHandle>,
) -> Result<OwnedSchedulerOutcome, ExecuteOwnedFederateError> {
    let execution = std::panic::catch_unwind(AssertUnwindSafe(|| {
        run_owned_scheduler_with_coordination_and_observation(
            storage,
            config,
            origin,
            coordination,
            Some(participant),
            observation,
        )
    }));
    match execution {
        Ok(Ok(outcome)) => {
            participant
                .finish_success()
                .map_err(|source| ExecuteOwnedFederateError::FederateCoordination { source })?;
            Ok(outcome)
        }
        Ok(Err(error)) => {
            let (error, ownership) = classify_scheduler_failure(enclave, error);
            if ownership == FailureReportOwnership::Supervisor {
                participant.fail();
            }
            Err(error)
        }
        Err(payload) => {
            participant.fail();
            Err(ExecuteOwnedFederateError::ThreadPanicked {
                enclave,
                message: panic_message(payload),
            })
        }
    }
}

/// Results and failures retained until the coordinator identifies the first failing participant.
struct WorkerResults {
    results: TinySecondaryMap<EnclaveIndex, EnclaveExecution>,
    worker_errors: TinySecondaryMap<EnclaveIndex, ExecuteOwnedFederateError>,
    first_worker: Option<EnclaveIndex>,
    failure: Option<ExecuteOwnedFederateError>,
}

/// Receive every started worker result and join all scheduler threads.
fn collect_worker_results(
    handles: Vec<(EnclaveIndex, std::thread::ScopedJoinHandle<'_, ()>)>,
    result_rx: std::sync::mpsc::Receiver<(
        EnclaveIndex,
        Result<EnclaveExecution, ExecuteOwnedFederateError>,
    )>,
    shutdown: &RunShutdown,
    mut failure: Option<ExecuteOwnedFederateError>,
    enclave_count: usize,
) -> WorkerResults {
    let mut worker_errors = TinySecondaryMap::new();
    let mut first_worker = None;
    let started_count = handles.len();
    let mut results = TinySecondaryMap::with_capacity(enclave_count);
    for _ in 0..started_count {
        match result_rx.recv() {
            Ok((enclave, Ok(result))) => {
                results.insert(enclave, result);
            }
            Ok((enclave, Err(error))) => {
                if first_worker.is_none() && failure.is_none() {
                    shutdown.close();
                }
                first_worker.get_or_insert(enclave);
                worker_errors.insert(enclave, error);
            }
            Err(_) => {
                if failure.is_none() && first_worker.is_none() {
                    shutdown.close();
                    failure = Some(ExecuteOwnedFederateError::ResultChannelClosed);
                }
                break;
            }
        }
    }
    for (enclave, handle) in handles {
        if let Err(payload) = handle.join() {
            if failure.is_none() && first_worker.is_none() {
                shutdown.close();
                failure = Some(ExecuteOwnedFederateError::ThreadPanicked {
                    enclave,
                    message: panic_message(payload),
                });
            }
        }
    }
    WorkerResults {
        results,
        worker_errors,
        first_worker,
        failure,
    }
}

/// Select the final error or combine successful Enclave results after shutdown.
fn finish_execution(
    results: TinySecondaryMap<EnclaveIndex, EnclaveExecution>,
    failure: Option<ExecuteOwnedFederateError>,
    origin: Instant,
    _shutdown: &RunShutdown,
) -> Result<FederateExecution, ExecuteOwnedFederateError> {
    if matches!(
        failure,
        Some(ExecuteOwnedFederateError::EnclaveInitialization { .. })
    ) {
        return Err(failure.expect("matched initialization failure"));
    }
    #[cfg(feature = "hosted-manual-clock")]
    if let Some(clock) = &_shutdown.clock {
        if let Some(error) = _shutdown
            .clock_failure
            .get()
            .copied()
            .unwrap_or_else(|| clock.now().err())
        {
            return Err(error.into());
        }
    }
    #[cfg(feature = "hosted-source-input")]
    if let Some(inputs) = &_shutdown.inputs {
        inputs.finish()?;
    }
    if let Some(error) = failure {
        Err(error)
    } else {
        let mut stats = Stats::default();
        let mut final_tag = Tag::NEVER;
        for execution in results.values() {
            stats.saturating_add_assign(execution.stats());
            final_tag = final_tag.max(execution.final_tag());
        }
        Ok(FederateExecution {
            enclaves: results,
            origin,
            stats,
            final_tag,
        })
    }
}

/// Scheduler storage and coordination ports ready for worker startup.
struct PreparedWorkers<'image, B: FederateCoordinationBackend> {
    storages: Vec<(EnclaveIndex, OwnedStorage<'image>)>,
    coordinations: Vec<(EnclaveIndex, EnclaveDependencies)>,
    coordinator: crate::sched::federate::FederateCoordinator<B>,
    participants: TinySecondaryMap<EnclaveIndex, crate::sched::federate::EnclaveCoordinationPort>,
}

/// Start the coordinator, initialize state, then supervise every scheduler worker.
fn execute_workers<B: FederateCoordinationBackend>(
    workers: PreparedWorkers<'_, B>,
    config: &Config,
    observations: &TinySecondaryMap<EnclaveIndex, crate::ObservationHandle>,
    origin: Instant,
    shutdown: &RunShutdown,
    storage_phase: &mut ConstructionPhase,
    mut fail_spawn: impl FnMut(Option<EnclaveIndex>) -> bool,
) -> (
    TinySecondaryMap<EnclaveIndex, EnclaveExecution>,
    Option<ExecuteOwnedFederateError>,
) {
    let PreparedWorkers {
        mut storages,
        coordinations,
        mut coordinator,
        participants,
    } = workers;
    let enclave_count = storages.len();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let parent_span = tracing::Span::current();
    let mut worker_phase = ConstructionPhase::started("workers");
    coordinator.execution_open = false;
    std::thread::scope(|scope| {
        let _startup_cleanup = CloseRunOnDrop(shutdown.clone());
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let coordinator_thread = if fail_spawn(None) {
            Err(std::io::Error::other(
                "injected scoped thread spawn failure",
            ))
        } else {
            let dispatch = dispatch.clone();
            let parent_span = parent_span.clone();
            std::thread::Builder::new()
                .name("federate-coordination".to_owned())
                .spawn_scoped(scope, move || {
                    let _dispatch = tracing::dispatcher::set_default(&dispatch);
                    #[cfg(feature = "bounded-tracing")]
                    let _producer = tracing_bounded::prepare_current_thread();
                    let _parent = parent_span.entered();
                    coordinator.run()
                })
        };
        let coordinator_thread = match coordinator_thread {
            Ok(handle) => handle,
            Err(source) => {
                worker_phase.failed("spawn");
                drop(participants);
                shutdown.close();
                return (
                    TinySecondaryMap::with_capacity(enclave_count),
                    Some(ExecuteOwnedFederateError::CoordinatorThreadSpawn { source }),
                );
            }
        };
        if let Err(error) = initialize_states(&mut storages, shutdown, storage_phase) {
            return (TinySecondaryMap::with_capacity(enclave_count), Some(error));
        }
        shutdown.abort.open_execution();
        let mut handles = Vec::with_capacity(enclave_count);
        let mut failure = None;
        let mut participant_ports = participants.into_iter();
        for ((enclave, storage), (coordination_enclave, coordination)) in
            storages.into_iter().zip(coordinations)
        {
            assert_eq!(
                enclave, coordination_enclave,
                "compiled scheduler coordination must retain its Enclave identity"
            );
            let result_tx = result_tx.clone();
            let config = config.clone();
            let observation = observations.get(enclave).cloned();
            let (participant_enclave, mut participant) = participant_ports
                .next()
                .expect("compiled Federate construction provides every scheduler participant");
            assert_eq!(
                enclave, participant_enclave,
                "compiled coordination port must retain its deployment-global Enclave identity"
            );
            let mut storage = WorkerStorage {
                storage: Some(storage),
                shutdown: shutdown.clone(),
            };
            let spawned = if fail_spawn(Some(enclave)) {
                Err(std::io::Error::other(
                    "injected scoped thread spawn failure",
                ))
            } else {
                let dispatch = dispatch.clone();
                let parent_span = parent_span.clone();
                std::thread::Builder::new()
                    .name(format!("enclave-{enclave}"))
                    .spawn_scoped(scope, move || {
                        let _dispatch = tracing::dispatcher::set_default(&dispatch);
                        #[cfg(feature = "bounded-tracing")]
                        let _producer = tracing_bounded::prepare_current_thread();
                        let _parent = parent_span.entered();
                        let _enclave = tracing::debug_span!(target: "boomerang::coordination",
                            "enclave", enclave = enclave.as_u32())
                        .entered();
                        let result = match run_enclave(
                            enclave,
                            &mut storage,
                            &config,
                            origin,
                            coordination,
                            &mut participant,
                            observation.as_ref(),
                        ) {
                            Ok(OwnedSchedulerOutcome { final_tag, stats }) => {
                                Ok(EnclaveExecution {
                                    states: storage.into_states(),
                                    final_tag,
                                    stats,
                                })
                            }
                            Err(error) => Err(error),
                        };
                        drop(participant);
                        let _ = result_tx.send((enclave, result));
                    })
            };
            match spawned {
                Ok(handle) => handles.push((enclave, handle)),
                Err(source) => {
                    worker_phase.failed("spawn");
                    shutdown.close();
                    failure = Some(ExecuteOwnedFederateError::ThreadSpawn { enclave, source });
                    break;
                }
            }
        }
        drop(participant_ports);
        drop(result_tx);
        if failure.is_none() {
            worker_phase.completed();
        }

        let WorkerResults {
            results,
            worker_errors,
            first_worker,
            mut failure,
        } = collect_worker_results(handles, result_rx, shutdown, failure, enclave_count);
        let coordinator_result = coordinator_thread.join();
        if failure.is_none() {
            let origin = coordinator_result
                .as_ref()
                .ok()
                .and_then(|exit| exit.first_failed_participant);
            failure = select_scheduler_failure(worker_errors, origin, first_worker);
        }
        let coordinator_result = coordinator_result.map(|exit| exit.coordination_result);
        latch_coordinator_result(&mut failure, coordinator_result);
        (results, failure)
    })
}

/// Match each checked local route to its typed binding.
fn resolve_route_bindings(
    endpoints: &[ResolvedLocalRoute<'_>],
    routes: &[Box<dyn RouteBinding + '_>],
) -> BTreeMap<LocalRouteKey, usize> {
    endpoints
        .iter()
        .map(|route| {
            let binding = routes
                .iter()
                .position(|binding| binding.boundary() == route.boundary)
                .expect("Federate preflight required every local route binding");
            (route.key, binding)
        })
        .collect::<BTreeMap<_, _>>()
}

/// Connect the selected backend and construct its coordinator before starting workers.
fn connect_coordinator<'image, B: FederateCoordinationBackend>(
    storages: &mut [(EnclaveIndex, OwnedStorage<'image>)],
    participant_indices: &[EnclaveIndex],
    lifecycle: LifecyclePolicy,
    connect: impl FnOnce(
        &mut [(EnclaveIndex, OwnedStorage<'image>)],
    ) -> Result<B, ExecuteOwnedFederateError>,
) -> Result<FederateCoordinationParts<B>, ExecuteOwnedFederateError> {
    let mut backend_phase = ConstructionPhase::started("backend");
    let backend = match connect(storages) {
        Ok(backend) => backend,
        Err(error) => {
            backend_phase.failed("connection");
            return Err(error);
        }
    };
    let parts = build_federate_coordination(
        participant_indices,
        storages.iter().map(|(enclave, storage)| {
            (
                *enclave,
                storage.scheduler_event_tx(),
                storage.scheduler_event_rx(),
            )
        }),
        lifecycle,
        backend,
    );
    let parts = match parts {
        Ok(parts) => parts,
        Err(error) => {
            backend_phase.failed("setup");
            return Err(error);
        }
    };
    backend_phase.completed();
    Ok(parts)
}
