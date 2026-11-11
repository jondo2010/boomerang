//! Output proofs and failure retention through real compiled scheduler fixtures.
use super::*;
#[cfg(feature = "hosted-output")]
#[test]
fn output_checkpoint_compiled_empty_gap_acquires_real_coordinator_permission() {
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    use crate::{image::EnclaveIndex, OutputBound};
    static EMPTY: EnclaveImage<'static> = EnclaveImage {
        timer_startup_actions: &[],
        ..IMAGE
    };
    for timeout in [None, Some(Duration::ZERO)] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = build_storage_for_image(&EMPTY, calls.clone(), false);
        let observer = storage.output_observer(ReactorIndex::new(0)).unwrap();
        let tag = if timeout.is_some() {
            Tag::new(Duration::ZERO, 1)
        } else {
            Tag::new(Duration::nanoseconds(3), 7)
        };
        let wait = observer.request_tag(tag).unwrap();
        let parts = FederateCoordinationParts::new(
            [(
                EnclaveIndex::new(0),
                storage.scheduler_event_tx(),
                storage.scheduler_event_rx(),
            )],
            LifecyclePolicy::KeepAlive,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
        let (_, mut port) = parts.participants.into_iter().next().unwrap();
        let coordinator = std::thread::spawn(move || parts.coordinator.run());
        let scheduler = std::thread::spawn(move || {
            run_owned_scheduler_with_coordination(
                &mut storage,
                &Config {
                    timeout,
                    ..Config::default()
                },
                std::time::Instant::now(),
                EnclaveDependencies::new(EnclaveKey::default()),
                Some(&mut port),
            )
        });
        let result = wait.wait_timeout(std::time::Duration::from_secs(2));
        parts.abort_handle.abort();
        let stats = scheduler.join().unwrap().unwrap().stats;
        coordinator.join().unwrap();
        let completed = result.unwrap();
        assert_eq!(completed.bound(), OutputBound::ThroughTag(tag));
        assert_eq!(
            completed.target(),
            (EnclaveIndex::new(0), ReactorIndex::new(0))
        );
        assert_eq!(completed.epoch(), observer.epoch());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(stats.processed_tags(), usize::from(timeout.is_some()));
    }
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_checkpoint_compiled_effect_failure_and_owned_drop_are_terminal() {
    use crate::{OutputBound, OutputFault, OutputOutcome, OutputWaitError};
    for output in [
        OutputOutcome::Committed,
        OutputOutcome::Failed(OutputFault::Device),
    ] {
        let reactions = [REACTIONS[0].clone().with_external_effect(true)];
        let startups = [TimerStartupImage::new(ActionIndex::new(0), 0)];
        let image = EnclaveImageView::new(EnclaveImage {
            reactions: TinyMapRef::from_slice(&reactions),
            timer_startup_actions: &startups,
            ..IMAGE.clone()
        })
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let bindings = EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                move |context: &mut Context, _, _: ReactionRefs<'_>, _| {
                    captured.lock().unwrap().push(context.get_tag());
                    Ok(output)
                },
            );
        let mut storage = OwnedStorage::new(image, bindings).unwrap();
        assert!(matches!(
            storage.output_observer(ReactorIndex::new(1)),
            Err(OutputWaitError::InvalidTarget)
        ));
        let wait = storage
            .output_observer(ReactorIndex::new(0))
            .unwrap()
            .request_tag(Tag::ZERO)
            .unwrap();
        let result = run_owned_scheduler(&mut storage, &Config::default());
        let completed = wait.wait_timeout(std::time::Duration::from_secs(1));
        assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO]);
        if output == OutputOutcome::Committed {
            result.unwrap();
            assert_eq!(
                completed.unwrap().bound(),
                OutputBound::ThroughTag(Tag::ZERO)
            );
        } else {
            assert!(result.is_err());
            assert_eq!(completed, Err(OutputWaitError::ExecutionFailed));
        }
    }
    let storage = build_storage(Arc::new(Mutex::new(Vec::new())), false);
    let wait = storage
        .output_observer(ReactorIndex::new(0))
        .unwrap()
        .request_tag(Tag::ZERO)
        .unwrap();
    drop(storage);
    assert_eq!(
        wait.wait_timeout(std::time::Duration::from_secs(1)),
        Err(OutputWaitError::Disconnected)
    );
}

#[cfg(all(feature = "hosted-output", feature = "hosted-manual-clock"))]
#[test]
fn output_checkpoint_owner_epoch_matches_clock_selected_before_export() {
    let mut storage = build_storage(Arc::new(Mutex::new(Vec::new())), false);
    let clock = crate::clock::ManualClock::new(crate::clock::PhysicalClockDomainId(7)).unwrap();
    storage
        .set_clock(crate::clock::ExecutionClock::manual(clock.clone(), 0))
        .unwrap();
    let observer = storage.output_observer(ReactorIndex::new(0)).unwrap();
    assert_eq!(observer.epoch(), clock.epoch());
    let replacement =
        crate::clock::ManualClock::new(crate::clock::PhysicalClockDomainId(7)).unwrap();
    assert_eq!(
        storage.set_clock(crate::clock::ExecutionClock::manual(replacement, 0)),
        Err(crate::clock::PhysicalClockError::AlreadyUsed)
    );
    assert_eq!(observer.epoch(), clock.epoch());
}

#[cfg(all(feature = "hosted-output", feature = "hosted-manual-clock"))]
#[test]
fn output_checkpoint_clock_terminated_during_effect_never_returns_success() {
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    use crate::{
        clock::{ExecutionClock, ManualClock, PhysicalClockDomainId},
        OutputOutcome, OutputWaitError,
    };
    for close in [false, true] {
        let reactions = [REACTIONS[0].clone().with_external_effect(true)];
        let startups = [TimerStartupImage::new(ActionIndex::new(0), 0)];
        let image = EnclaveImageView::new(EnclaveImage {
            reactions: TinyMapRef::from_slice(&reactions),
            timer_startup_actions: &startups,
            ..IMAGE.clone()
        })
        .unwrap();
        let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
        let captured_clock = clock.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured_calls = calls.clone();
        let bindings = EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                move |context: &mut Context, _, _: ReactionRefs<'_>, _| {
                    captured_calls.lock().unwrap().push(context.get_tag());
                    if close {
                        captured_clock.close();
                    } else {
                        captured_clock.fail();
                    }
                    Ok(OutputOutcome::Committed)
                },
            );
        let mut storage = OwnedStorage::new(image, bindings).unwrap();
        // Supply a real deadline slot; keep its abort handle separate from the standalone mailbox.
        let (abort_tx, abort_rx) = kanal::bounded(1);
        let parts = FederateCoordinationParts::new(
            [(crate::image::EnclaveIndex::new(0), abort_tx, abort_rx)],
            LifecyclePolicy::KeepAlive,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
        clock
            .attach(&[storage.scheduler_event_tx()], parts.abort_handle)
            .unwrap();
        storage.set_clock(ExecutionClock::manual(clock, 0)).unwrap();
        let wait = storage
            .output_observer(ReactorIndex::new(0))
            .unwrap()
            .request_tag(Tag::ZERO)
            .unwrap();
        run_owned_scheduler(&mut storage, &Config::default()).unwrap();
        assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO]);
        assert_eq!(
            wait.wait_timeout(std::time::Duration::from_secs(1)),
            Err(OutputWaitError::ExecutionFailed)
        );
    }
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_time_close_compiled_uses_real_coordinator_ordered_seal() {
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    use crate::{clock::PhysicalInstant, image::EnclaveIndex, OutputBound};
    static EMPTY: EnclaveImage<'static> = EnclaveImage {
        timer_startup_actions: &[],
        ..IMAGE
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&EMPTY, calls.clone(), false);
    let observer = storage.output_observer(ReactorIndex::new(0)).unwrap();
    let wait = observer.close_time(PhysicalInstant(3)).unwrap();
    let parts = FederateCoordinationParts::new(
        [(
            EnclaveIndex::new(0),
            storage.scheduler_event_tx(),
            storage.scheduler_event_rx(),
        )],
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    let (_, mut port) = parts.participants.into_iter().next().unwrap();
    let coordinator = std::thread::spawn(move || parts.coordinator.run());
    let scheduler = std::thread::spawn(move || {
        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default(),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
    });
    let result = wait.wait_timeout(std::time::Duration::from_secs(2));
    parts.abort_handle.abort();
    let stats = scheduler.join().unwrap().unwrap().stats;
    coordinator.join().unwrap();
    assert_eq!(
        result.unwrap().bound(),
        OutputBound::ThroughTime(PhysicalInstant(3))
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(stats.processed_tags(), 0);
}

#[cfg(all(
    feature = "hosted-output",
    feature = "hosted-source-input",
    feature = "hosted-manual-clock"
))]
#[test]
fn output_time_close_executes_preseal_source_batch_and_rejects_fresh_late_batch() {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId, PhysicalInstant};
    use crate::input::{
        AdmissionProcessor, CompiledPhysicalAction, InputBatch, InputConfig, InputErrorKind,
        InputObservation, InputSource, InputValue, SourceAction, SourceKey,
    };
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    use crate::{
        image::{EnclaveIndex, TimingDomain},
        ActionRef, OutputBound, PayloadType,
    };
    static PHYSICAL_ACTIONS: [ActionImage; 1] = [ActionImage::new(
        ScopeIndex::new(0),
        ActionSlotIndex::new(0),
        ActionTiming::Standard {
            domain: TimingDomain::Physical,
            min_delay_nanos: 0,
        },
        SliceRange::new(0, 1),
        Some(BindingSlotIndex::new(2)),
    )];
    static PHYSICAL_REACTIONS: [ReactionImage; 1] = [ReactionImage::new(
        ReactorIndex::new(0),
        ScopeIndex::new(0),
        0,
        BindingSlotIndex::new(1),
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 1),
        SliceRange::new(0, 0),
    )];
    static BINDINGS: [RequiredBindingImage; 3] = [
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("astate"),
            BindingKind::StateInitializer,
        ),
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("breaction"),
            BindingKind::Reaction,
        ),
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("cpayload"),
            BindingKind::Action,
        ),
    ];
    static PHYSICAL: EnclaveImage<'static> = EnclaveImage {
        actions: TinyMapRef::from_slice(&PHYSICAL_ACTIONS),
        reactions: TinyMapRef::from_slice(&PHYSICAL_REACTIONS),
        reaction_actions: &[ActionIndex::new(0)],
        required_bindings: TinyMapRef::from_slice(&BINDINGS),
        timer_startup_actions: &[],
        storage_bounds: &StorageBounds::new(1, 1, 4, 0, 0, 0),
        ..IMAGE
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let record = calls.clone();
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_action(BindingSlotIndex::new(2), PayloadType::<u32>::new())
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |ctx: &mut Context, _, refs: ReactionRefs<'_>, _| {
                let mut action: ActionRef<u32> = refs.actions.partition_mut().unwrap();
                record
                    .lock()
                    .unwrap()
                    .push((ctx.get_tag(), *ctx.get_action_value(&mut action).unwrap()));
                Ok(())
            },
        );
    let mut storage =
        OwnedStorage::new(EnclaveImageView::new(PHYSICAL.clone()).unwrap(), bindings).unwrap();
    let clock = ManualClock::new(PhysicalClockDomainId(21)).unwrap();
    storage
        .set_clock(ExecutionClock::manual(clock.clone(), 0))
        .unwrap();
    let observer = storage.output_observer(ReactorIndex::new(0)).unwrap();
    let mut parts = FederateCoordinationParts::new(
        [(
            EnclaveIndex::new(0),
            storage.scheduler_event_tx(),
            storage.scheduler_event_rx(),
        )],
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    clock
        .attach(&[storage.scheduler_event_tx()], parts.abort_handle.clone())
        .unwrap();
    clock.advance_to(PhysicalInstant(10)).unwrap();
    let target = CompiledPhysicalAction::new(EnclaveIndex::new(0), ActionIndex::new(0));
    let (inputs, processor) = AdmissionProcessor::new(
        InputConfig {
            max_batch_values: 1,
            max_staged_batches: 2,
            sources: [InputSource {
                id: "optional".into(),
                required: false,
                actions: vec![target],
            }]
            .into_iter()
            .collect(),
        },
        [Duration::ZERO].into_iter().collect(),
        clock.clone(),
        [(EnclaveIndex::new(0), storage.scheduler_event_tx())]
            .into_iter()
            .collect(),
        &mut parts.coordinator.state,
        parts.abort_handle.clone(),
    );
    parts.coordinator.inputs = Some(processor);
    let (_, mut port) = parts.participants.into_iter().next().unwrap();
    port.input_enabled = true;
    let source = inputs.source_handle(SourceKey::new(0)).unwrap();
    let action = SourceAction::<u32> {
        reference: target,
        source: source.clone(),
        delay: Duration::ZERO,
        payload: std::marker::PhantomData,
    };
    let batch = |sequence, time, value| {
        InputBatch::new(vec![InputObservation {
            source: source.clone(),
            sequence,
            domain: clock.domain(),
            epoch: clock.epoch(),
            acquired: PhysicalInstant(time),
            values: vec![InputValue::new(&action, value)],
        }])
    };
    let coordinator = std::thread::spawn(move || parts.coordinator.run());
    // Admission completes before any scheduler can reserve or complete the batch's tag.
    let wait = observer.close_time(PhysicalInstant(10)).unwrap();
    inputs.submit(batch(1, 5, 42)).unwrap();
    let scheduler = std::thread::spawn(move || {
        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default(),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
    });
    let result = wait.wait_timeout(std::time::Duration::from_secs(2));
    let late = inputs.submit(batch(2, 10, 99));
    parts.abort_handle.abort();
    let stats = scheduler.join().unwrap().unwrap().stats;
    coordinator.join().unwrap();
    assert_eq!(
        result.unwrap().bound(),
        OutputBound::ThroughTime(PhysicalInstant(10))
    );
    assert_eq!(late.unwrap_err().kind, InputErrorKind::Late);
    assert_eq!(
        &*calls.lock().unwrap(),
        &[(Tag::new(Duration::nanoseconds(5), 0), 42)]
    );
    assert_eq!(stats.processed_tags(), 1);
}

/// A real backend adapter records the candidates visible outside the coordinator.
#[cfg(feature = "hosted-output")]
#[derive(Default)]
struct OutputPublicationBackend<const LOCAL: bool> {
    /// Real in-process grant delivery, used by both supported and rejected profiles.
    local: crate::sched::federate::LocalFederateCoordinationBackend,
    /// Finite candidates actually sent through the backend publication boundary.
    publications: Arc<Mutex<Vec<Tag>>>,
}
#[cfg(feature = "hosted-output")]
impl<const LOCAL: bool> crate::sched::federate::FederateCoordinationBackend
    for OutputPublicationBackend<LOCAL>
{
    fn supports_output_proofs(&self) -> bool {
        LOCAL
    }
    fn publish(
        &mut self,
        publication: crate::sched::federate::FederatePublication,
    ) -> Result<(), FederateCoordinationError> {
        if let Some(tag) = publication.next_event() {
            self.publications.lock().unwrap().push(tag);
        }
        self.local.publish(publication)
    }
    fn progress(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<crate::sched::federate::FederateAcquisition>, FederateCoordinationError>
    {
        self.local.progress(timeout)
    }
    fn complete(
        &mut self,
        completion: crate::sched::federate::FederateCompletion,
    ) -> Result<(), FederateCoordinationError> {
        self.local.complete(completion)
    }
    fn stop(&mut self) -> Result<(), FederateCoordinationError> {
        self.local.stop()
    }
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_time_close_publishes_earliest_real_work_before_the_virtual_cut() {
    use crate::sched::federate::{FederateCoordinationParts, LifecyclePolicy};
    use crate::{clock::PhysicalInstant, image::EnclaveIndex};
    static EARLY: EnclaveImage<'static> = EnclaveImage {
        timer_startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), 5)],
        ..IMAGE
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut storage = build_storage_for_image(&EARLY, calls.clone(), false);
    let wait = storage
        .output_observer(ReactorIndex::new(0))
        .unwrap()
        .close_time(PhysicalInstant(10))
        .unwrap();
    let backend = OutputPublicationBackend::<true>::default();
    let publications = backend.publications.clone();
    let parts = FederateCoordinationParts::new(
        [(
            EnclaveIndex::new(0),
            storage.scheduler_event_tx(),
            storage.scheduler_event_rx(),
        )],
        LifecyclePolicy::KeepAlive,
        backend,
    )
    .unwrap();
    let (_, mut port) = parts.participants.into_iter().next().unwrap();
    let coordinator = std::thread::spawn(move || parts.coordinator.run());
    let scheduler = std::thread::spawn(move || {
        run_owned_scheduler_with_coordination(
            &mut storage,
            &Config::default(),
            std::time::Instant::now(),
            EnclaveDependencies::new(EnclaveKey::default()),
            Some(&mut port),
        )
    });
    let result = wait.wait_timeout(std::time::Duration::from_secs(2));
    parts.abort_handle.abort();
    scheduler.join().unwrap().unwrap();
    coordinator.join().unwrap();
    result.unwrap();
    let event = Tag::new(Duration::nanoseconds(5), 0);
    assert_eq!(
        publications.lock().unwrap().first(),
        Some(&event),
        "the backend must see queued work before the virtual closure candidate"
    );
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Call::Reaction(tag, _) if *tag == event)));
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_observers_reject_backend_without_final_ingress_exclusion_before_cutoff() {
    use crate::sched::federate::{FederateCoordinationParts, LifecyclePolicy};
    use crate::{clock::PhysicalInstant, image::EnclaveIndex, OutputBound, OutputWaitError};
    static EARLY: EnclaveImage<'static> = EnclaveImage {
        timer_startup_actions: &[TimerStartupImage::new(ActionIndex::new(0), 5)],
        ..IMAGE
    };
    for bound in [
        OutputBound::ThroughTag(Tag::new(Duration::nanoseconds(10), 0)),
        OutputBound::ThroughTime(PhysicalInstant(10)),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (effect_tx, effect_rx) = std::sync::mpsc::channel();
        let recorded = calls.clone();
        let bindings = EnclaveBindings::new()
            .bind_state(BindingSlotIndex::new(0), initialize_state)
            .bind_reaction(
                BindingSlotIndex::new(1),
                move |context: &mut Context, _, _: ReactionRefs<'_>, _| {
                    recorded
                        .lock()
                        .unwrap()
                        .push(Call::Reaction(context.get_tag(), std::time::Instant::now()));
                    effect_tx.send(()).unwrap();
                    Ok(())
                },
            );
        let mut storage =
            OwnedStorage::new(EnclaveImageView::new(EARLY.clone()).unwrap(), bindings).unwrap();
        let observer = storage.output_observer(ReactorIndex::new(0)).unwrap();
        let wait = match bound {
            OutputBound::ThroughTag(tag) => observer.request_tag(tag),
            OutputBound::ThroughTime(time) => observer.close_time(time),
        }
        .unwrap();
        let backend = OutputPublicationBackend::<false>::default();
        let publications = backend.publications.clone();
        let parts = FederateCoordinationParts::new(
            [(
                EnclaveIndex::new(0),
                storage.scheduler_event_tx(),
                storage.scheduler_event_rx(),
            )],
            LifecyclePolicy::KeepAlive,
            backend,
        )
        .unwrap();
        let (_, mut port) = parts.participants.into_iter().next().unwrap();
        let coordinator = std::thread::spawn(move || parts.coordinator.run());
        let scheduler = std::thread::spawn(move || {
            let result = run_owned_scheduler_with_coordination(
                &mut storage,
                &Config::default(),
                std::time::Instant::now(),
                EnclaveDependencies::new(EnclaveKey::default()),
                Some(&mut port),
            );
            let output = storage.take_output_state();
            let progress = (output.progress.closing, output.progress.sealed);
            (result, progress)
        });
        let result = wait.wait_timeout(std::time::Duration::from_secs(2));
        let executed = effect_rx.recv_timeout(std::time::Duration::from_secs(2));
        parts.abort_handle.abort();
        let (execution, progress) = scheduler.join().unwrap();
        coordinator.join().unwrap();
        assert!(executed.is_ok(), "ordinary execution must still run");
        execution.unwrap();
        assert_eq!(
            result,
            Err(OutputWaitError::UnsupportedBackend),
            "{bound:?}"
        );
        assert!(
            progress.0.is_none() && progress.1.is_none(),
            "rejection cannot install a cutoff"
        );
        assert!(!publications.lock().unwrap().contains(&bound.horizon()));
        assert!(calls.lock().unwrap().iter().any(|call| matches!(call, Call::Reaction(tag, _) if *tag == Tag::new(Duration::nanoseconds(5), 0))));
    }
}

/// A shared binding and swapped storage slots do not merge parent and child effects.
#[cfg(feature = "hosted-output")]
#[test]
fn output_initialization_uses_state_owner_with_shared_binding_and_swapped_slots() {
    use crate::initialization::StateCapabilities;
    use crate::{OutputBound, OutputOutcome};

    let reactors = [
        ReactorImage::new(
            BindingSlotIndex::new(0),
            StateSlotIndex::new(1),
            ScopeIndex::new(0),
            IndexSpan::new(0, 0),
            None,
            None,
        ),
        ReactorImage::new(
            BindingSlotIndex::new(0),
            StateSlotIndex::new(0),
            ScopeIndex::new(1),
            IndexSpan::new(0, 0),
            None,
            None,
        ),
    ];
    let scopes = [
        ScopeImage::new(
            None,
            ReactorIndex::new(0),
            None,
            SliceRange::new(0, 2),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        ),
        ScopeImage::new(
            Some(ScopeIndex::new(0)),
            ReactorIndex::new(1),
            None,
            SliceRange::new(2, 1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        ),
    ];
    let descendants = [ScopeIndex::new(0), ScopeIndex::new(1), ScopeIndex::new(1)];
    let actions = [
        ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(0),
            ActionTiming::Timer { period_nanos: None },
            SliceRange::new(0, 1),
            None,
        ),
        ActionImage::new(
            ScopeIndex::new(1),
            ActionSlotIndex::new(1),
            ActionTiming::Timer { period_nanos: None },
            SliceRange::new(1, 1),
            None,
        ),
    ];
    let reactions = [
        ReactionImage::new(
            ReactorIndex::new(0),
            ScopeIndex::new(0),
            0,
            BindingSlotIndex::new(1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
        .with_external_effect(true),
        ReactionImage::new(
            ReactorIndex::new(1),
            ScopeIndex::new(1),
            1,
            BindingSlotIndex::new(2),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
        .with_external_effect(true),
    ];
    let triggers = [
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(1, ReactionIndex::new(1)),
    ];
    let required_bindings = [
        REQUIRED_BINDINGS[0],
        REQUIRED_BINDINGS[1],
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("creaction"),
            BindingKind::Reaction,
        ),
    ];
    let bounds = StorageBounds::new(2, 2, 2, 0, 0, 0);
    for parent_effect in [false, true] {
        let mut startups = Vec::new();
        if parent_effect {
            startups.push(TimerStartupImage::new(ActionIndex::new(0), 0));
        }
        startups.push(TimerStartupImage::new(ActionIndex::new(1), 0));
        let image = EnclaveImageView::new(EnclaveImage {
            reactors: TinyMapRef::from_slice(&reactors),
            scopes: TinyMapRef::from_slice(&scopes),
            scope_descendants: &descendants,
            actions: TinyMapRef::from_slice(&actions),
            reactions: TinyMapRef::from_slice(&reactions),
            reaction_triggers: &triggers,
            timer_startup_actions: &startups,
            required_bindings: TinyMapRef::from_slice(&required_bindings),
            storage_bounds: &bounds,
            ..IMAGE.clone()
        })
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut bindings = EnclaveBindings::new().bind_state_with_capabilities(
            BindingSlotIndex::new(0),
            StateCapabilities {
                output_observer: true,
                ..Default::default()
            },
            move |context| {
                tx.send(context.output_observer()?).unwrap();
                Ok(TestState)
            },
        );
        for (owner, binding, output) in [
            (
                ReactorIndex::new(0),
                BindingSlotIndex::new(1),
                OutputOutcome::Unchanged,
            ),
            (
                ReactorIndex::new(1),
                BindingSlotIndex::new(2),
                OutputOutcome::Committed,
            ),
        ] {
            let recorded = calls.clone();
            bindings = bindings.bind_reaction(
                binding,
                move |context: &mut Context, _, _: ReactionRefs<'_>, _| {
                    recorded
                        .lock()
                        .unwrap()
                        .push((owner, context.get_tag(), output));
                    Ok(output)
                },
            );
        }
        bindings.validate_ordinary_capabilities(&image).unwrap();
        let mut storage =
            OwnedStorage::prepare_for_enclave(image, bindings, EnclaveKey::new(9)).unwrap();
        storage.configure_output(
            crate::image::EnclaveIndex::new(9),
            crate::clock::ExecutionEpoch(27),
        );
        storage
            .initialize_states(
                Some(crate::image::EnclaveIndex::new(9)),
                #[cfg(feature = "hosted-source-input")]
                None,
                #[cfg(feature = "hosted-manual-clock")]
                None,
            )
            .unwrap();
        let observers = [rx.recv().unwrap(), rx.recv().unwrap()];
        let waits = observers
            .iter()
            .map(|observer| {
                assert_eq!(observer.epoch(), crate::clock::ExecutionEpoch(27));
                observer.request_tag(Tag::ZERO).unwrap()
            })
            .collect::<Vec<_>>();
        run_owned_scheduler(&mut storage, &Config::default()).unwrap();
        let mut expected_calls = Vec::new();
        if parent_effect {
            expected_calls.push((ReactorIndex::new(0), Tag::ZERO, OutputOutcome::Unchanged));
        }
        expected_calls.push((ReactorIndex::new(1), Tag::ZERO, OutputOutcome::Committed));
        assert_eq!(*calls.lock().unwrap(), expected_calls);
        for (wait, expected) in waits
            .into_iter()
            .zip([ReactorIndex::new(1), ReactorIndex::new(0)])
        {
            let completion = wait
                .wait_timeout(std::time::Duration::from_secs(1))
                .unwrap();
            assert_eq!(
                completion.target(),
                (crate::image::EnclaveIndex::new(9), expected)
            );
            assert_eq!(completion.bound(), OutputBound::ThroughTag(Tag::ZERO));
            assert_eq!(completion.epoch(), crate::clock::ExecutionEpoch(27));
        }
        let output = storage.take_output_state();
        assert_eq!(
            output.retained_outcome(ReactorIndex::new(0)),
            parent_effect.then_some((Tag::ZERO, OutputOutcome::Unchanged)),
            "a child's committed effect must not become a parent report"
        );
        assert_eq!(
            output.retained_outcome(ReactorIndex::new(1)),
            Some((Tag::ZERO, OutputOutcome::Committed))
        );
    }
}

/// A real outbound route must clone this payload after the callback returns.
#[cfg(feature = "hosted-output")]
#[derive(Debug)]
struct OutputUnwindPayload;
#[cfg(feature = "hosted-output")]
impl Clone for OutputUnwindPayload {
    fn clone(&self) -> Self {
        panic!("outbound clone panicked");
    }
}

/// Checks scratch, owner retention, and restoration through actual scheduler unwinding.
#[cfg(feature = "hosted-output")]
fn check_output_unwind_owned(route_panic: bool, adapter_only: bool) {
    use crate::{OutputOutcome, OutputWaitError};
    let reactors = [0, 1].map(|i| {
        ReactorImage::new(
            BindingSlotIndex::new(0),
            StateSlotIndex::new(i),
            ScopeIndex::new(i),
            IndexSpan::new(0, 0),
            None,
            None,
        )
    });
    let scopes = [0, 1].map(|i| {
        ScopeImage::new(
            None,
            ReactorIndex::new(i),
            None,
            SliceRange::new(i, 1),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
    });
    let descendants = [ScopeIndex::new(0), ScopeIndex::new(1)];
    let actions = [
        ActionImage::new(
            ScopeIndex::new(1),
            ActionSlotIndex::new(0),
            ActionTiming::Timer { period_nanos: None },
            SliceRange::new(0, 1),
            None,
        ),
        ActionImage::new(
            ScopeIndex::new(0),
            ActionSlotIndex::new(1),
            ActionTiming::Timer { period_nanos: None },
            SliceRange::new(1, 2),
            None,
        ),
    ];
    let reactions = [0, 1].map(|i| {
        ReactionImage::new(
            ReactorIndex::new(i),
            ScopeIndex::new(i),
            0,
            BindingSlotIndex::new(i + 1),
            SliceRange::new(0, 0),
            SliceRange::new(i, u32::from(i == 0)),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )
        .with_external_effect(true)
    });
    let triggers = [
        LevelReactionImage::new(0, ReactionIndex::new(1)),
        LevelReactionImage::new(0, ReactionIndex::new(0)),
        LevelReactionImage::new(0, ReactionIndex::new(1)),
    ];
    let starts = [
        TimerStartupImage::new(ActionIndex::new(0), 0),
        TimerStartupImage::new(ActionIndex::new(1), 1),
    ];
    let ports = [crate::image::PortImage::new(
        ScopeIndex::new(0),
        SliceRange::new(0, 0),
        BindingSlotIndex::new(3),
    )];
    let bindings = [
        REQUIRED_BINDINGS[0],
        REQUIRED_BINDINGS[1],
        RequiredBindingImage::new(
            crate::image::BindingSlotId::new("creaction"),
            BindingKind::Reaction,
        ),
        RequiredBindingImage::new(crate::image::BindingSlotId::new("dport"), BindingKind::Port),
    ];
    let bounds = StorageBounds::new(2, 2, 2, 0, 0, 0);
    let effect_ports = [PortIndex::new(0)];
    let image = EnclaveImageView::new(EnclaveImage {
        reactors: TinyMapRef::from_slice(&reactors),
        scopes: TinyMapRef::from_slice(&scopes),
        scope_descendants: &descendants,
        actions: TinyMapRef::from_slice(&actions),
        reactions: TinyMapRef::from_slice(&reactions),
        reaction_triggers: &triggers,
        timer_startup_actions: &starts,
        ports: TinyMapRef::from_slice(&ports),
        reaction_effect_ports: &effect_ports,
        required_bindings: TinyMapRef::from_slice(&bindings),
        storage_bounds: &bounds,
        ..IMAGE.clone()
    })
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_port(
            BindingSlotIndex::new(3),
            crate::PayloadType::<OutputUnwindPayload>::new(),
        );
    for (owner, binding) in [
        (ReactorIndex::new(0), BindingSlotIndex::new(1)),
        (ReactorIndex::new(1), BindingSlotIndex::new(2)),
    ] {
        let recorded = calls.clone();
        bindings = bindings.bind_reaction(
            binding,
            move |context: &mut Context, _, refs: ReactionRefs<'_>, _| {
                let later = owner == ReactorIndex::new(1);
                if later && context.get_tag() != Tag::ZERO {
                    panic!("later callback panicked");
                }
                if !later && route_panic {
                    let mut port: crate::OutputRef<OutputUnwindPayload> =
                        refs.ports_mut.partition_mut()?;
                    *port = Some(OutputUnwindPayload);
                }
                recorded.lock().unwrap().push((owner, context.get_tag()));
                Ok(if later {
                    OutputOutcome::Unchanged
                } else {
                    OutputOutcome::Committed
                })
            },
        );
    }
    let mut storage = OwnedStorage::new(image, bindings).unwrap();
    let (tx, _rx) = kanal::bounded(1);
    storage.bind_outbound_route::<OutputUnwindPayload>(
        PortIndex::new(0),
        crate::image::BoundaryId::new("output"),
        crate::image::EnclaveIndex::new(1),
        PortIndex::new(0),
        crate::image::TimingDomain::Logical,
        0,
        tx,
    );
    let tag = Tag::new(Duration::nanoseconds(1), 0);
    let expected_panic = if route_panic {
        "outbound clone panicked"
    } else {
        "later callback panicked"
    };
    if adapter_only {
        use crate::sched::core::{ExecutionStorage, ReactionOutcome};
        let mut outcomes = [ReactionOutcome::default()];
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            storage.execute_reactions(&[ReactionIndex::new(0)], tag, &mut outcomes)
        }))
        .unwrap_err();
        assert_eq!(panic.downcast_ref::<&str>(), Some(&expected_panic));
        assert_eq!(*calls.lock().unwrap(), [(ReactorIndex::new(0), tag)]);
        assert_eq!(outcomes[0].output, Some(OutputOutcome::Committed));
        return;
    }
    let waits = [0, 1].map(|i| {
        storage
            .output_observer(ReactorIndex::new(i))
            .unwrap()
            .request_tag(tag)
            .unwrap()
    });
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_owned_scheduler(&mut storage, &Config::default()).map(|_| ())
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&expected_panic));
    for wait in waits {
        assert_eq!(
            wait.wait_timeout(std::time::Duration::from_secs(1)),
            Err(OutputWaitError::ExecutionFailed)
        );
    }
    let mut output = storage.take_output_state();
    assert_eq!(
        output.retained_outcome(ReactorIndex::new(1)),
        Some((Tag::ZERO, OutputOutcome::Unchanged))
    );
    assert_eq!(
        output.retained_outcome(ReactorIndex::new(0)),
        Some((tag, OutputOutcome::Committed))
    );
    storage.restore_output_state(&mut output);
    // Owned storage allows one scheduler attachment. A caught unwind must not
    // bypass that rule or replay a callback on a second attachment attempt.
    let retry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_owned_scheduler(&mut storage, &Config::default()).map(|_| ())
    }));
    assert!(retry.is_err());
    assert_eq!(
        *calls.lock().unwrap(),
        [
            (ReactorIndex::new(1), Tag::ZERO),
            (ReactorIndex::new(0), tag)
        ]
    );
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_unwind_owned_callback_restores_prior_and_returned_evidence() {
    check_output_unwind_owned(false, false);
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_unwind_owned_routing_restores_prior_and_returned_evidence() {
    check_output_unwind_owned(true, false);
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_unwind_owned_routing_adapter_retains_returned_effect() {
    check_output_unwind_owned(true, true);
}

#[cfg(feature = "hosted-output")]
fn check_output_terminal_horizon(output: crate::OutputOutcome) {
    use crate::{OutputBound, OutputOutcome};
    let reactions = [REACTIONS[0].clone().with_external_effect(true)];
    let image = EnclaveImageView::new(EnclaveImage {
        reactions: TinyMapRef::from_slice(&reactions),
        timer_startup_actions: &[],
        ..SHUTDOWN_IMAGE.clone()
    })
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
                recorded
                    .lock()
                    .unwrap()
                    .push(Call::Reaction(ctx.get_tag(), std::time::Instant::now()));
                Ok(output)
            },
        );
    let mut storage = OwnedStorage::new(image, bindings).unwrap();
    let tag = Tag::new(Duration::ZERO, 1);
    let wait = storage
        .output_observer(ReactorIndex::new(0))
        .unwrap()
        .request_tag(tag)
        .unwrap();
    let mut port = coordination(calls.clone(), [FederateTagAcquisition::Granted], [], []);
    // Model the real coordinator's earliest legal response, without a thread race.
    port.close_on_horizon = Some(storage.scheduler_event_rx());
    let result = run_owned_scheduler_with_coordination(
        &mut storage,
        &Config::default().with_timeout(Duration::ZERO),
        std::time::Instant::now(),
        EnclaveDependencies::new(EnclaveKey::default()),
        Some(&mut port),
    );
    if output == OutputOutcome::Committed {
        result.unwrap();
        let completion = wait
            .wait_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(completion.bound(), OutputBound::ThroughTag(tag));
        assert_eq!(
            completion.target(),
            (crate::image::EnclaveIndex::new(0), ReactorIndex::new(0))
        );
        assert!(port.close_on_horizon.as_ref().unwrap().is_closed());
    } else {
        assert!(
            matches!(result, Err(SchedulerError::FederateFailureReported { .. })),
            "a terminal callback failure must reach the coordinator"
        );
        assert_eq!(
            wait.wait_timeout(std::time::Duration::from_secs(1)),
            Err(crate::OutputWaitError::ExecutionFailed)
        );
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|call| matches!(call, Call::Fail)));
        assert!(!calls.iter().any(|call| matches!(call, Call::Horizon(_))));
    }
    assert_eq!(
        storage
            .take_output_state()
            .retained_outcome(ReactorIndex::new(0)),
        Some((tag, output))
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Reaction(at, _) if *at == tag))
            .count(),
        1
    );
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_terminal_effect_is_proved_before_horizon_closes_mailbox() {
    check_output_terminal_horizon(crate::OutputOutcome::Committed);
}

#[cfg(feature = "hosted-output")]
#[test]
fn output_terminal_failure_reports_to_coordinator_before_horizon_notification() {
    check_output_terminal_horizon(crate::OutputOutcome::Failed(crate::OutputFault::Device));
}

#[cfg(feature = "hosted-output")]
mod checkpoint;
#[cfg(feature = "hosted-output")]
mod harness;
#[cfg(feature = "hosted-output")]
mod time;

mod modal;
