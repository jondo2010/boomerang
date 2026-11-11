//! Tag proofs, instance binding, and reaction-outcome retention.
use super::harness::*;

#[test]
fn output_checkpoint_completes_after_effect_and_preserves_exact_owner() {
    let (mut scheduler, reactor, reaction, calls) = fixture(OutputOutcome::Committed);
    let observer = scheduler.output_observer(reactor).unwrap();
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    assert!(scheduler.try_next().unwrap());
    let completed = wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO]);
    assert_eq!(completed.bound(), OutputBound::ThroughTag(Tag::ZERO));
    assert_eq!(
        completed.target(),
        (crate::image::EnclaveIndex::new(0), reactor)
    );
    assert_eq!(completed.epoch(), observer.epoch());
    assert!(matches!(
        scheduler.output_observer(ReactorIndex::new(99)),
        Err(OutputWaitError::InvalidTarget)
    ));
}

#[test]
fn output_checkpoint_crosses_an_empty_gap_without_executing_a_reaction() {
    let (mut scheduler, reactor, _, calls) = fixture(OutputOutcome::Committed);
    let observer = scheduler.output_observer(reactor).unwrap();
    let tag = Tag::new(Duration::nanoseconds(3), 0);
    let wait = observer.request_tag(tag).unwrap();
    scheduler.startup();
    assert!(scheduler.try_next().unwrap());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTag(tag)
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(scheduler.current_tag, tag);
    assert_eq!(scheduler.stats.processed_reactions(), 0);
}

#[test]
fn output_checkpoint_does_not_claim_later_microsteps() {
    let (mut scheduler, reactor, reaction, calls) = fixture(OutputOutcome::Committed);
    let observer = scheduler.output_observer(reactor).unwrap();
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    let later = Tag::new(Duration::ZERO, 1);
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    scheduler
        .events
        .push_event(later, [(Level::from(0), reaction)], false);
    assert!(scheduler.try_next().unwrap());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTag(Tag::ZERO)
    );
    assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO]);
    let wait = observer.request_tag(later).unwrap();
    assert!(scheduler.try_next().unwrap());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTag(later)
    );
    assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO, later]);
}

#[test]
fn output_checkpoint_failed_effect_never_returns_success() {
    let (mut scheduler, reactor, reaction, _) = fixture(OutputOutcome::Failed(OutputFault::Device));
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::ZERO)
        .unwrap();
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    assert!(scheduler.try_next().is_err());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::ExecutionFailed)
    );
}

#[test]
fn output_checkpoint_shutdown_and_early_drop_terminate_queued_waits() {
    let (scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::ZERO)
        .unwrap();
    drop(scheduler);
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::Disconnected)
    );

    let (mut scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::new(Duration::seconds(1), 0))
        .unwrap();
    scheduler.config.timeout = Some(Duration::ZERO);
    scheduler.try_event_loop().unwrap();
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::Shutdown)
    );
}

#[test]
fn output_checkpoint_timeout_and_drop_cannot_supply_a_stale_reply() {
    // This regression intentionally fills a one-request mailbox while cancellation
    // is still queued; the general protocol fixtures allow multiple control frames.
    static SINGLE_REQUEST: EnclaveImage<'static> = EnclaveImage {
        storage_bounds: &StorageBounds::new(1, 1, 1, 0, 0, 0),
        ..EFFECT_IMAGE
    };
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            |_: &mut Context, _, _: ReactionRefs<'_>, _| Ok(OutputOutcome::Committed),
        );
    let storage = OwnedStorage::new(
        EnclaveImageView::new(SINGLE_REQUEST.clone()).unwrap(),
        bindings,
    )
    .unwrap();
    let mut scheduler = CompiledScheduler::new(EnclaveKey::from(4), storage);
    let reactor = ReactorIndex::new(0);
    let reaction = ReactionIndex::new(0);

    let observer = scheduler.output_observer(reactor).unwrap();
    assert_eq!(
        observer
            .request_tag(Tag::ZERO)
            .unwrap()
            .wait_timeout(HostDuration::ZERO),
        Err(OutputWaitError::Timeout)
    );
    let tag = Tag::new(Duration::ZERO, 2);
    assert!(matches!(
        observer.request_tag(tag),
        Err(OutputWaitError::Capacity)
    ));
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    assert!(scheduler.try_next().unwrap());
    let canceled = Tag::new(Duration::ZERO, 1);
    drop(observer.request_tag(canceled).unwrap());
    scheduler
        .events
        .push_event(canceled, [(Level::from(0), reaction)], false);
    assert!(scheduler.try_next().unwrap());
    let wait = observer.request_tag(tag).unwrap();
    assert!(scheduler.try_next().unwrap());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTag(tag)
    );
}

#[test]
fn output_checkpoint_panic_wakes_wait_even_when_scheduler_is_retained() {
    let (mut scheduler, reactor, reaction, calls) = fixture(OutputOutcome::Committed);
    let _ = std::panic::catch_unwind(|| {
        let _guard = calls.lock().unwrap();
        panic!("poison device fixture");
    });
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::ZERO)
        .unwrap();
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scheduler.try_next())).is_err()
    );
    assert_eq!(
        wait.wait_timeout(HostDuration::from_millis(20)),
        Err(OutputWaitError::ExecutionFailed)
    );
}

#[test]
fn output_checkpoint_rejects_late_negative_external_shutdown() {
    let (mut scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let observer = scheduler.output_observer(reactor).unwrap();
    let tag = Tag::new(Duration::nanoseconds(10), 0);
    let wait = observer.request_tag(tag).unwrap();
    scheduler.startup();
    assert!(scheduler.try_next().unwrap());
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    scheduler
        .event_tx
        .send(AsyncEvent::Shutdown {
            delay: Duration::nanoseconds(-1),
        })
        .unwrap();
    assert!(
        scheduler.try_next().is_err(),
        "a past shutdown must not revise completed output"
    );
    assert_eq!(scheduler.current_tag, tag);
}

#[test]
fn output_checkpoint_reaction_rejects_negative_shutdown_after_proof() {
    let reactor = ReactorIndex::new(0);
    let reaction = ReactionIndex::new(0);
    let accepted = Arc::new(Mutex::new(None));
    let recorded = accepted.clone();
    let mut scheduler = callback_fixture(move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
        *recorded.lock().unwrap() = Some(ctx.schedule_shutdown(Some(Duration::nanoseconds(-10))));
        Ok(OutputOutcome::Committed)
    });
    let observer = scheduler.output_observer(reactor).unwrap();
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    scheduler.startup();
    assert!(scheduler.try_next().unwrap());
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    scheduler.events.push_event(
        Tag::new(Duration::nanoseconds(5), 0),
        [(Level::from(0), reaction)],
        false,
    );
    assert!(scheduler.try_next().unwrap());
    assert_eq!(*accepted.lock().unwrap(), Some(false));
    assert!(scheduler.shutdown_tag.is_none());
}

#[test]
fn output_checkpoint_cancel_interrupts_physical_pacing() {
    let (mut scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let observation = Arc::new(crate::ObservationState::new(std::time::Instant::now()));
    scheduler.observation = Some(observation.clone());
    let sender = scheduler.event_tx.clone();
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::new(Duration::days(1), 0))
        .unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        scheduler.startup();
        let result = scheduler.try_next();
        done_tx.send(result).unwrap();
    });
    let deadline = std::time::Instant::now() + HostDuration::from_secs(2);
    while !observation
        .snapshot(std::time::Instant::now())
        .is_some_and(|snapshot| snapshot.current_phase == crate::SchedulerPhase::PhysicalWait)
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    drop(wait);
    let result = done_rx.recv_timeout(HostDuration::from_millis(100));
    let _ = sender.try_send(AsyncEvent::FederateResume);
    worker.join().unwrap();
    assert!(
        result.is_ok(),
        "cancel must wake a scheduler paced at the canceled checkpoint"
    );
}

#[test]
fn output_checkpoint_disconnect_aborts_empty_pacing_without_a_report() {
    let (mut scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let observation = Arc::new(crate::ObservationState::new(std::time::Instant::now()));
    scheduler.observation = Some(observation.clone());
    let sender = scheduler.event_tx.clone();
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::new(Duration::milliseconds(500), 0))
        .unwrap();
    let worker = std::thread::spawn(move || {
        scheduler.startup();
        scheduler.try_next()
    });
    let deadline = std::time::Instant::now() + HostDuration::from_secs(2);
    while !observation
        .snapshot(std::time::Instant::now())
        .is_some_and(|snapshot| snapshot.current_phase == crate::SchedulerPhase::PhysicalWait)
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    sender.close().unwrap();
    let result = wait.wait_timeout(HostDuration::from_millis(100));
    let kept_running = worker.join().unwrap().unwrap();
    assert_eq!(result, Err(OutputWaitError::Disconnected));
    assert!(
        !kept_running,
        "a disconnected proof-only checkpoint must not advance logical time"
    );
}

#[test]
fn output_checkpoint_stale_epoch_cannot_complete_on_another_scheduler() {
    let (old, reactor, _, _) = fixture(OutputOutcome::Committed);
    let observer = old.output_observer(reactor).unwrap();
    let wait = observer.request_tag(Tag::ZERO).unwrap();
    let request = old.event_rx.recv().unwrap();
    let (mut fresh, fresh_reactor, reaction, _) = fixture(OutputOutcome::Committed);
    assert_ne!(
        observer.epoch(),
        fresh.output_observer(fresh_reactor).unwrap().epoch()
    );
    fresh.event_tx.send(request).unwrap();
    fresh.startup();
    fresh
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    assert!(fresh.try_next().unwrap());
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::StaleExecution)
    );
}

#[test]
fn output_checkpoint_drop_wakes_worker_before_reactor_state_joins_it() {
    struct Worker(Option<std::thread::JoinHandle<()>>);
    impl Drop for Worker {
        fn drop(&mut self) {
            self.0.take().unwrap().join().unwrap();
        }
    }
    let (wait_tx, wait_rx) = std::sync::mpsc::channel::<crate::OutputWait>();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        result_tx
            .send(
                wait_rx
                    .recv()
                    .unwrap()
                    .wait_timeout(HostDuration::from_secs(1)),
            )
            .unwrap();
    });
    let reactor = ReactorIndex::new(0);
    let worker = Arc::new(Mutex::new(Some(worker)));
    let captured = worker.clone();
    let bindings = EnclaveBindings::new()
        .bind_state_with_capabilities(
            BindingSlotIndex::new(0),
            crate::initialization::StateCapabilities::default(),
            move |_| Ok(Worker(captured.lock().unwrap().take())),
        )
        .bind_reaction(
            BindingSlotIndex::new(1),
            |_: &mut Context, _, _: ReactionRefs<'_>, _| Ok(OutputOutcome::Committed),
        );
    let mut storage = OwnedStorage::prepare_for_enclave(
        EnclaveImageView::new(EFFECT_IMAGE.clone()).unwrap(),
        bindings,
        EnclaveKey::default(),
    )
    .unwrap();
    storage.configure_output(
        crate::image::EnclaveIndex::new(0),
        crate::clock::ExecutionEpoch(37),
    );
    storage
        .initialize_states(
            Some(crate::image::EnclaveIndex::new(0)),
            #[cfg(feature = "hosted-source-input")]
            None,
            #[cfg(feature = "hosted-manual-clock")]
            None,
        )
        .unwrap();
    wait_tx
        .send(
            storage
                .output_observer(reactor)
                .unwrap()
                .request_tag(Tag::ZERO)
                .unwrap(),
        )
        .unwrap();
    drop(storage);
    assert_eq!(
        result_rx.recv().unwrap(),
        Err(OutputWaitError::Disconnected)
    );
}

#[test]
fn output_checkpoint_mailbox_closed_during_effect_never_returns_success() {
    let reactor = ReactorIndex::new(0);
    let reaction = ReactionIndex::new(0);
    let sender = Arc::new(Mutex::new(None::<crate::Sender<AsyncEvent>>));
    let captured_sender = sender.clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let mut scheduler = callback_fixture(move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
        recorded.lock().unwrap().push(ctx.get_tag());
        captured_sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .close()
            .unwrap();
        Ok(OutputOutcome::Committed)
    });
    *sender.lock().unwrap() = Some(scheduler.event_tx.clone());
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .request_tag(Tag::ZERO)
        .unwrap();
    scheduler.startup();
    scheduler
        .events
        .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
    scheduler.try_next().unwrap();
    assert_eq!(&*calls.lock().unwrap(), &[Tag::ZERO]);
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::Disconnected)
    );
}
