//! Time closure through validated compiled images and real scheduler execution.
use super::harness::*;

/// Drive the real scheduler until a public time-close wait can be resolved.
fn drive_time_close(scheduler: &mut CompiledScheduler, time: crate::clock::PhysicalInstant) {
    for _ in 0..128 {
        if scheduler.output.time_closed(time) {
            return;
        }
        assert!(
            scheduler.try_next().unwrap(),
            "closure terminated before proof"
        );
    }
    panic!("closure did not converge");
}

#[test]
fn output_time_close_empty_interval_never_executes_a_projected_tag() {
    let (mut scheduler, reactor, _, calls) = fixture(OutputOutcome::Committed);
    let time = crate::clock::PhysicalInstant(0);
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .close_time(time)
        .unwrap();
    scheduler.startup();
    let before = scheduler.current_tag;
    drive_time_close(&mut scheduler, time);
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTime(time)
    );
    assert_eq!(scheduler.current_tag, before);
    assert_eq!(scheduler.stats.processed_tags(), 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn output_time_close_drains_later_microsteps_and_retains_later_time() {
    let (mut scheduler, reactor, reaction, calls) = fixture(OutputOutcome::Committed);
    let time = crate::clock::PhysicalInstant(0);
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .close_time(time)
        .unwrap();
    scheduler.startup();
    for tag in [
        Tag::ZERO,
        Tag::new(Duration::ZERO, 1),
        Tag::new(Duration::nanoseconds(1), 0),
    ] {
        scheduler
            .events
            .push_event(tag, [(Level::from(0), reaction)], false);
    }
    drive_time_close(&mut scheduler, time);
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1))
            .unwrap()
            .bound(),
        OutputBound::ThroughTime(time)
    );
    assert_eq!(
        &*calls.lock().unwrap(),
        &[Tag::ZERO, Tag::new(Duration::ZERO, 1)]
    );
    assert_eq!(
        scheduler.events.peek_tag(),
        Some(Tag::new(Duration::nanoseconds(1), 0))
    );
    assert_eq!(scheduler.stats.processed_tags(), 2);
}

#[test]
fn output_time_close_moves_later_shutdown_above_durable_floor() {
    let (mut scheduler, reactor, _, _) = fixture(OutputOutcome::Committed);
    let time = crate::clock::PhysicalInstant(0);
    let wait = scheduler
        .output_observer(reactor)
        .unwrap()
        .close_time(time)
        .unwrap();
    scheduler.startup();
    drive_time_close(&mut scheduler, time);
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    scheduler
        .event_tx
        .try_send(AsyncEvent::shutdown(Duration::ZERO))
        .unwrap();
    assert!(!scheduler.try_next().unwrap());
    assert_eq!(scheduler.current_tag, Tag::new(Duration::nanoseconds(1), 0));
}

/// Uses the existing dependency maps and channels, with no second scheduler authority.
fn connect_output_pair(
    a: &mut CompiledScheduler,
    b: &mut CompiledScheduler,
    a_delay: Option<Duration>,
    b_delay: Option<Duration>,
) {
    let a_shutdown = a.shutdown_tx.new_receiver();
    let b_shutdown = b.shutdown_tx.new_receiver();
    let a_context = crate::SendContext {
        clock: a.clock.clone(),
        enclave_key: a.key,
        async_tx: a.event_tx.clone(),
        shutdown_rx: a_shutdown,
    };
    let b_context = crate::SendContext {
        clock: b.clock.clone(),
        enclave_key: b.key,
        async_tx: b.event_tx.clone(),
        shutdown_rx: b_shutdown,
    };
    a.upstream_enclaves
        .insert(b.key, LogicalTimeBarrier::new(b_context.clone(), a_delay));
    b.upstream_enclaves
        .insert(a.key, LogicalTimeBarrier::new(a_context.clone(), b_delay));
    a.downstream_enclaves.insert(b.key, b_context);
    b.downstream_enclaves.insert(a.key, a_context);
    a.config.keep_alive = true;
    b.config.keep_alive = true;
}

#[test]
fn output_time_close_empty_cycle_and_positive_delay_subcertificate() {
    for positive in [false, true] {
        let (mut a, owner, reaction, calls) =
            fixture_at(EnclaveKey::from(1), OutputOutcome::Committed);
        let (mut b, _, _, _) = fixture_at(EnclaveKey::from(2), OutputOutcome::Committed);
        connect_output_pair(
            &mut a,
            &mut b,
            positive.then(|| Duration::nanoseconds(10)),
            None,
        );
        let time = crate::clock::PhysicalInstant(if positive { 100 } else { 0 });
        let wait = a.output_observer(owner).unwrap().close_time(time).unwrap();
        a.startup();
        b.startup();
        if positive {
            a.events.push_event(
                Tag::new(Duration::nanoseconds(100), 0),
                [(Level::from(0), reaction)],
                false,
            );
        }
        let a_tx = a.event_tx.clone();
        let b_tx = b.event_tx.clone();
        let wa = std::thread::spawn(move || {
            let result = a.try_event_loop();
            (a, result)
        });
        let wb = std::thread::spawn(move || {
            let result = b.try_event_loop();
            (b, result)
        });
        let result = wait.wait_timeout(HostDuration::from_secs(2));
        a_tx.close().unwrap();
        b_tx.close().unwrap();
        let (a, ar) = wa.join().unwrap();
        let (b, br) = wb.join().unwrap();
        assert!(
            result.is_ok(),
            "positive={positive}, result={result:?}, a={ar:?}, b={br:?}, a_cut={:?}, b_cut={:?}",
            a.output.progress,
            b.output.progress
        );
        assert_eq!(result.unwrap().bound(), OutputBound::ThroughTime(time));
        if positive {
            assert_eq!(
                &*calls.lock().unwrap(),
                &[Tag::new(Duration::nanoseconds(100), 0)]
            );
            assert!(b.output.progress.sealed.is_some_and(
                |cut| cut >= OutputBound::ThroughTime(crate::clock::PhysicalInstant(90))
            ));
        } else {
            assert!(calls.lock().unwrap().is_empty());
        }
        assert!(a.current_tag < Tag::ZERO || a.current_tag.microstep() != usize::MAX);
        assert!(b.current_tag < Tag::ZERO || b.current_tag.microstep() != usize::MAX);
    }
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_time_close_held_prepared_action_waits_above_floor_at_manual_clock() {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId, PhysicalInstant};
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    use crate::{Action, ActionRef, AsyncActionRef, DynActionRef};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let owner = ReactorIndex::new(0);
    let mut scheduler = action_fixture(
        true,
        move |ctx: &mut Context, _, refs: ReactionRefs<'_>, _| {
            let mut input: ActionRef<u32> = refs.actions.partition_mut().unwrap();
            recorded
                .lock()
                .unwrap()
                .push((ctx.get_tag(), *ctx.get_action_value(&mut input).unwrap()));
            Ok(OutputOutcome::Committed)
        },
    );
    let action = scheduler
        .storage
        .scheduler_action_ref(ActionIndex::new(0))
        .key();
    scheduler.config.keep_alive = true;
    let clock = ManualClock::new(PhysicalClockDomainId(9)).unwrap();
    let (abort_tx, abort_rx) = kanal::bounded(1);
    let parts = FederateCoordinationParts::new(
        [(crate::image::EnclaveIndex::new(0), abort_tx, abort_rx)],
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    clock
        .attach(&[scheduler.event_tx.clone()], parts.abort_handle.clone())
        .unwrap();
    scheduler.set_clock(ExecutionClock::manual(clock.clone(), 0));
    let shutdown_rx = scheduler.shutdown_tx.new_receiver();
    let sender = crate::SendContext {
        clock: scheduler.clock.clone(),
        enclave_key: scheduler.key,
        async_tx: scheduler.event_tx.clone(),
        shutdown_rx,
    };
    let handle = Action::<u32>::new("held", action, None, false);
    let handle = AsyncActionRef::<u32>::try_from(DynActionRef(&handle)).unwrap();
    let held = sender.prepare_action(&handle, 42, None).unwrap();
    assert_eq!(held.time(), PhysicalInstant(0));
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .close_time(PhysicalInstant(0))
        .unwrap();
    scheduler.startup();
    drive_time_close(&mut scheduler, PhysicalInstant(0));
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    held.send().unwrap();
    let observation = Arc::new(crate::ObservationState::new(std::time::Instant::now()));
    scheduler.observation = Some(observation.clone());
    let worker = std::thread::spawn(move || {
        let result = scheduler.try_next();
        (scheduler, result)
    });
    let deadline = std::time::Instant::now() + HostDuration::from_secs(2);
    while !observation
        .snapshot(std::time::Instant::now())
        .is_some_and(|s| s.current_phase == crate::SchedulerPhase::PhysicalWait)
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(calls.lock().unwrap().is_empty());
    clock.advance_to(PhysicalInstant(1)).unwrap();
    let (mut scheduler, result) = worker.join().unwrap();
    assert!(result.unwrap());
    // A clock wake can interrupt pacing before the actual event is processed.
    if calls.lock().unwrap().is_empty() {
        assert!(scheduler.try_next().unwrap());
    }
    assert_eq!(
        &*calls.lock().unwrap(),
        &[(Tag::new(Duration::nanoseconds(1), 0), 42)]
    );
}

/// Action feedback gives the fresh mailbox check real work to detect and drain.
fn time_feedback_fixture() -> (
    CompiledScheduler,
    ReactorIndex,
    ActionKey,
    Arc<Mutex<Vec<Tag>>>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let scheduler = action_fixture(
        false,
        move |ctx: &mut Context, _, refs: ReactionRefs<'_>, _| {
            let mut input: crate::ActionRef<u32> = refs.actions.partition_mut().unwrap();
            let remaining = *ctx.get_action_value(&mut input).unwrap();
            recorded.lock().unwrap().push(ctx.get_tag());
            if remaining > 0 {
                ctx.schedule_action(&mut input, remaining - 1, None);
            }
            Ok(OutputOutcome::Committed)
        },
    );
    let action = scheduler
        .storage
        .scheduler_action_ref(ActionIndex::new(0))
        .key();
    (scheduler, ReactorIndex::new(0), action, calls)
}

/// Stop at a real freeze safe point while the root still has the Frozen ack queued.
fn drive_until_frozen(scheduler: &mut CompiledScheduler) {
    use crate::sched::output::closure::protocol::ParticipationPhase;
    for _ in 0..32 {
        if scheduler
            .output
            .proof
            .participant
            .as_ref()
            .is_some_and(|p| p.phase == ParticipationPhase::Frozen)
        {
            return;
        }
        assert!(scheduler.try_next().unwrap());
    }
    panic!("proof did not freeze");
}

#[test]
fn output_time_close_fresh_check_drains_inflight_internal_feedback() {
    let (mut scheduler, owner, action, calls) = time_feedback_fixture();
    let time = crate::clock::PhysicalInstant(0);
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .close_time(time)
        .unwrap();
    scheduler.startup();
    drive_until_frozen(&mut scheduler);
    let first_round = scheduler.output.proof.root.as_ref().unwrap().round;
    // This accepted route payload precedes Check, but follows the earlier Ready.
    assert!(scheduler
        .event_tx
        .try_send(AsyncEvent::routed_logical(
            Tag::ZERO,
            crate::event::AsyncEventTarget::Action(action),
            Box::new(2u32),
        ))
        .unwrap());
    assert!(scheduler.try_next().unwrap());
    assert!(!scheduler.output.time_closed(time));
    assert!(
        calls.lock().unwrap().is_empty(),
        "frozen scheduler executed feedback"
    );
    while scheduler.output.proof.root.as_ref().unwrap().round == first_round {
        assert!(scheduler.try_next().unwrap());
    }
    let lease = scheduler.output.proof.root.as_ref().unwrap().lease.clone();
    scheduler
        .core()
        .handle_output_proof(crate::sched::output::closure::protocol::ProofFrame {
            lease,
            round: first_round,
            kind: crate::sched::output::closure::protocol::ProofKind::Commit,
        });
    assert!(
        scheduler.output.progress.sealed.is_none(),
        "stale round committed the new round"
    );
    drive_time_close(&mut scheduler, time);
    assert!(wait.wait_timeout(HostDuration::from_secs(1)).is_ok());
    assert_eq!(first_round, 1);
    assert_eq!(
        &*calls.lock().unwrap(),
        &[
            Tag::ZERO,
            Tag::new(Duration::ZERO, 1),
            Tag::new(Duration::ZERO, 2)
        ]
    );
}

#[test]
fn output_time_close_cancel_retains_floor_and_rejects_stale_commit() {
    use crate::sched::output::closure::protocol::{ProofFrame, ProofKind};
    for timeout in [false, true] {
        let (mut scheduler, owner, _, _) = fixture(OutputOutcome::Committed);
        let observer = scheduler.output_observer(owner).unwrap();
        let time = crate::clock::PhysicalInstant(0);
        let wait = observer.close_time(time).unwrap();
        scheduler.startup();
        drive_until_frozen(&mut scheduler);
        let root = scheduler.output.proof.root.as_ref().unwrap();
        let lease = root.lease.clone();
        let round = root.round;
        if timeout {
            assert_eq!(
                wait.wait_timeout(HostDuration::ZERO),
                Err(OutputWaitError::Timeout)
            );
        } else {
            drop(wait);
        }
        // Cancellation is examined at a safe point without relying on an Abort frame.
        scheduler.core().service_output_proof().unwrap();
        assert!(lease.ended());
        assert_eq!(
            scheduler.output.progress.closing.unwrap().cut,
            OutputBound::ThroughTime(time)
        );
        scheduler.core().handle_output_proof(ProofFrame {
            lease,
            round,
            kind: ProofKind::Commit,
        });
        assert!(scheduler.output.progress.sealed.is_none());
        assert!(!scheduler.output.time_closed(time));
        while let Some(event) = scheduler.event_rx.try_recv().unwrap() {
            scheduler.core().handle_async_event(event).unwrap();
        }
        let retry = observer.close_time(time).unwrap();
        drive_time_close(&mut scheduler, time);
        retry.wait_timeout(HostDuration::from_secs(1)).unwrap();
    }
}

#[test]
fn output_time_close_external_strict_ingress_rejected_and_prior_proof_retained() {
    let (mut scheduler, owner, action, calls) = time_feedback_fixture();
    let observer = scheduler.output_observer(owner).unwrap();
    let time = crate::clock::PhysicalInstant(0);
    let wait = observer.close_time(time).unwrap();
    scheduler.startup();
    drive_time_close(&mut scheduler, time);
    let proof = wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    scheduler
        .event_tx
        .try_send(AsyncEvent::Logical {
            tag: Tag::ZERO,
            target: crate::event::AsyncEventTarget::Action(action),
            value: Box::new(0u32),
        })
        .unwrap();
    assert!(scheduler.try_next().is_err());
    assert_eq!(proof.bound(), OutputBound::ThroughTime(time));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn output_time_close_overlap_fails_without_stealing_reservation() {
    use crate::sched::output::closure::protocol::{ProofFrame, ProofKind, ProofRoot};
    let (mut scheduler, owner, _, _) = fixture(OutputOutcome::Committed);
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .close_time(crate::clock::PhysicalInstant(0))
        .unwrap();
    scheduler.startup();
    drive_until_frozen(&mut scheduler);
    let current = scheduler
        .output
        .proof
        .participant
        .as_ref()
        .unwrap()
        .lease
        .clone();
    let (tx, _rx) = kanal::bounded(2);
    let contender = ProofRoot::new(
        EnclaveKey::from(99),
        current.epoch,
        1,
        crate::clock::PhysicalInstant(0),
        tx.clone(),
        2,
    );
    scheduler.core().handle_output_proof(ProofFrame {
        lease: contender.lease.clone(),
        round: 1,
        kind: ProofKind::Reserve {
            cut: crate::clock::PhysicalInstant(0),
            reply: Some(tx),
        },
    });
    assert_eq!(
        contender.lease.error.get(),
        Some(&OutputWaitError::Capacity)
    );
    assert!(scheduler
        .output
        .proof
        .participant
        .as_ref()
        .unwrap()
        .lease
        .matches(&current));
    drop(contender);
    drive_time_close(&mut scheduler, crate::clock::PhysicalInstant(0));
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
}

#[test]
fn output_time_close_dormant_history_and_reset_reactivate_above_cut() {
    use crate::TransitionKind;
    for transition in [TransitionKind::History, TransitionKind::Reset] {
        let super::modal::ModalFixture {
            mut scheduler,
            deactivate,
            activate,
            pending,
            calls,
        } = super::modal::fixture(transition);
        let owner = ReactorIndex::new(0);
        scheduler.startup();
        for (tag, action, value) in [
            (
                Tag::ZERO,
                deactivate,
                Box::new(()) as Box<dyn crate::ReactorData>,
            ),
            (Tag::new(Duration::ZERO, 2), pending, Box::new(())),
        ] {
            scheduler
                .core()
                .handle_async_event(AsyncEvent::Logical {
                    tag,
                    target: crate::event::AsyncEventTarget::Action(action),
                    value,
                })
                .unwrap();
        }
        let observer = scheduler.output_observer(owner).unwrap();
        let wait = observer
            .close_time(crate::clock::PhysicalInstant(0))
            .unwrap();
        drive_time_close(&mut scheduler, crate::clock::PhysicalInstant(0));
        wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
        assert!(calls.lock().unwrap().is_empty());
        assert!(
            scheduler.events.has_nonterminal_work(),
            "dormant history was discarded"
        );
        scheduler
            .core()
            .handle_async_event(AsyncEvent::Physical {
                time: crate::clock::PhysicalInstant(1),
                target: crate::event::AsyncEventTarget::Action(activate),
                value: Box::new(()),
            })
            .unwrap();
        let next = observer
            .close_time(crate::clock::PhysicalInstant(1))
            .unwrap();
        drive_time_close(&mut scheduler, crate::clock::PhysicalInstant(1));
        next.wait_timeout(HostDuration::from_secs(1)).unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].offset(), Duration::nanoseconds(1));
        assert_eq!(
            calls[0].microstep(),
            if transition == TransitionKind::Reset {
                1
            } else {
                2
            }
        );
    }
}

/// Observe a real selected-clock wait; the host deadline supervises the test only.
#[cfg(feature = "hosted-manual-clock")]
fn await_output_physical_wait(observation: &crate::ObservationState) {
    let deadline = std::time::Instant::now() + HostDuration::from_secs(2);
    while !observation
        .snapshot(std::time::Instant::now())
        .is_some_and(|s| s.current_phase == crate::SchedulerPhase::PhysicalWait)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "scheduler did not enter physical wait"
        );
        std::thread::yield_now();
    }
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_time_close_held_manual_interval_and_terminal_clock() {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId, PhysicalInstant};
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    for terminal in [None, Some(false), Some(true)] {
        let (mut scheduler, owner, _, calls) = fixture(OutputOutcome::Committed);
        scheduler.config.keep_alive = true;
        let clock = ManualClock::new(PhysicalClockDomainId(19)).unwrap();
        let (abort_tx, abort_rx) = kanal::bounded(1);
        let parts = FederateCoordinationParts::new(
            [(crate::image::EnclaveIndex::new(0), abort_tx, abort_rx)],
            LifecyclePolicy::KeepAlive,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
        clock
            .attach(&[scheduler.event_tx.clone()], parts.abort_handle.clone())
            .unwrap();
        scheduler.set_clock(ExecutionClock::manual(clock.clone(), 0));
        let observation = Arc::new(crate::ObservationState::new(std::time::Instant::now()));
        scheduler.observation = Some(observation.clone());
        let observer = scheduler.output_observer(owner).unwrap();
        let wait = observer.close_time(PhysicalInstant(10)).unwrap();
        let tx = scheduler.event_tx.clone();
        let worker = std::thread::spawn(move || {
            let result = scheduler.try_event_loop();
            (scheduler, result)
        });
        await_output_physical_wait(&observation);
        assert_eq!(clock.now().unwrap(), PhysicalInstant(0));
        assert!(calls.lock().unwrap().is_empty());
        match terminal {
            None => clock.advance_to(PhysicalInstant(10)).unwrap(),
            Some(false) => clock.close(),
            Some(true) => clock.fail(),
        }
        let result = wait.wait_timeout(HostDuration::from_secs(1));
        let _ = tx.close();
        let (scheduler, _) = worker.join().unwrap();
        if terminal.is_none() {
            assert_eq!(
                result.unwrap().bound(),
                OutputBound::ThroughTime(PhysicalInstant(10))
            );
        } else {
            assert_eq!(result, Err(OutputWaitError::ExecutionFailed));
            assert!(!scheduler.output.time_closed(PhysicalInstant(10)));
        }
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_time_close_graceful_shutdown_effect_waits_above_floor() {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId, PhysicalInstant};
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let owner = ReactorIndex::new(0);
    static SHUTDOWN: EnclaveImage<'static> = EnclaveImage {
        shutdown_reactions: &SHUTDOWN_REACTIONS,
        shutdown_actions: &[ActionIndex::new(0)],
        ..EFFECT_IMAGE
    };
    let bindings = EnclaveBindings::new()
        .bind_state(BindingSlotIndex::new(0), initialize_state)
        .bind_reaction(
            BindingSlotIndex::new(1),
            move |ctx: &mut Context, _, _: ReactionRefs<'_>, _| {
                recorded.lock().unwrap().push(ctx.get_tag());
                Ok(OutputOutcome::Committed)
            },
        );
    let storage =
        OwnedStorage::new(EnclaveImageView::new(SHUTDOWN.clone()).unwrap(), bindings).unwrap();
    let mut scheduler = CompiledScheduler::new(EnclaveKey::from(1), storage);
    scheduler.config.keep_alive = true;
    let clock = ManualClock::new(PhysicalClockDomainId(22)).unwrap();
    let (abort_tx, abort_rx) = kanal::bounded(1);
    let parts = FederateCoordinationParts::new(
        [(crate::image::EnclaveIndex::new(0), abort_tx, abort_rx)],
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    clock
        .attach(&[scheduler.event_tx.clone()], parts.abort_handle.clone())
        .unwrap();
    scheduler.set_clock(ExecutionClock::manual(clock.clone(), 0));
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .close_time(PhysicalInstant(0))
        .unwrap();
    scheduler.startup();
    drive_time_close(&mut scheduler, PhysicalInstant(0));
    let proof = wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    scheduler
        .event_tx
        .try_send(AsyncEvent::shutdown(Duration::ZERO))
        .unwrap();
    let observation = Arc::new(crate::ObservationState::new(std::time::Instant::now()));
    scheduler.observation = Some(observation.clone());
    let worker = std::thread::spawn(move || {
        let result = scheduler.try_event_loop();
        (scheduler, result)
    });
    await_output_physical_wait(&observation);
    assert!(calls.lock().unwrap().is_empty());
    clock.advance_to(PhysicalInstant(1)).unwrap();
    let (_, result) = worker.join().unwrap();
    result.unwrap();
    assert_eq!(
        &*calls.lock().unwrap(),
        &[Tag::new(Duration::nanoseconds(1), 0)]
    );
    assert_eq!(proof.bound(), OutputBound::ThroughTime(PhysicalInstant(0)));
}

#[test]
fn output_time_close_absorbs_queued_virtual_control_without_executing_it() {
    let (mut scheduler, owner, _, calls) = fixture(OutputOutcome::Committed);
    let time = crate::clock::PhysicalInstant(0);
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .close_time(time)
        .unwrap();
    scheduler.startup();
    let before = scheduler.current_tag;
    scheduler
        .events
        .push_control_event(Tag::new(Duration::ZERO, usize::MAX));
    drive_time_close(&mut scheduler, time);
    wait.wait_timeout(HostDuration::from_secs(1)).unwrap();
    assert_eq!(scheduler.current_tag, before);
    assert_eq!(scheduler.stats.processed_tags(), 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn output_time_close_drains_finite_same_time_feedback_across_a_cycle() {
    use crate::ActionRef;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let peers = Arc::new(Mutex::new(
        Vec::<(crate::Sender<AsyncEvent>, ActionKey)>::new(),
    ));
    let mut schedulers = Vec::new();
    for index in 0..2 {
        let recorded = calls.clone();
        let captured_peers = peers.clone();
        let mut scheduler = action_fixture(
            false,
            move |ctx: &mut Context, _, refs: ReactionRefs<'_>, _| {
                let mut action: ActionRef<u32> = refs.actions.partition_mut().unwrap();
                let remaining = *ctx.get_action_value(&mut action).unwrap();
                recorded.lock().unwrap().push((index, ctx.get_tag()));
                if remaining > 0 {
                    let peers = captured_peers.lock().unwrap();
                    let (sender, destination) = &peers[1 - index];
                    sender
                        .send(AsyncEvent::routed_logical(
                            ctx.get_tag().delay(Duration::ZERO),
                            crate::event::AsyncEventTarget::Action(*destination),
                            Box::new(remaining - 1),
                        ))
                        .unwrap();
                }
                Ok(OutputOutcome::Committed)
            },
        );
        scheduler.key = EnclaveKey::from(index + 1);
        let action = scheduler
            .storage
            .scheduler_action_ref(ActionIndex::new(0))
            .key();
        peers
            .lock()
            .unwrap()
            .push((scheduler.event_tx.clone(), action));
        schedulers.push(scheduler);
    }
    let mut b = schedulers.pop().unwrap();
    let mut a = schedulers.pop().unwrap();
    connect_output_pair(&mut a, &mut b, None, None);
    a.startup();
    b.startup();
    let initial_action = a.storage.scheduler_action_ref(ActionIndex::new(0)).key();
    a.core()
        .handle_async_event(AsyncEvent::routed_logical(
            Tag::ZERO,
            crate::event::AsyncEventTarget::Action(initial_action),
            Box::new(2u32),
        ))
        .unwrap();
    let wait = a
        .output_observer(ReactorIndex::new(0))
        .unwrap()
        .close_time(crate::clock::PhysicalInstant(0))
        .unwrap();
    let a_tx = a.event_tx.clone();
    let b_tx = b.event_tx.clone();
    let wa = std::thread::spawn(move || a.try_event_loop());
    let wb = std::thread::spawn(move || b.try_event_loop());
    let result = wait.wait_timeout(HostDuration::from_secs(2));
    let _ = a_tx.close();
    let _ = b_tx.close();
    let _ = wa.join().unwrap();
    let _ = wb.join().unwrap();
    result.unwrap();
    assert_eq!(
        &*calls.lock().unwrap(),
        &[
            (0, Tag::ZERO),
            (1, Tag::new(Duration::ZERO, 1)),
            (0, Tag::new(Duration::ZERO, 2))
        ]
    );
}

#[test]
fn output_time_close_cancel_after_local_commit_keeps_frontier_without_publishing() {
    let (mut scheduler, owner, _, _) = fixture(OutputOutcome::Committed);
    let observer = scheduler.output_observer(owner).unwrap();
    let time = crate::clock::PhysicalInstant(0);
    let wait = observer.close_time(time).unwrap();
    scheduler.startup();
    drive_until_frozen(&mut scheduler);
    for _ in 0..16 {
        if scheduler.output.progress.sealed.is_some() {
            break;
        }
        assert!(scheduler.try_next().unwrap());
    }
    assert_eq!(
        scheduler.output.progress.sealed,
        Some(OutputBound::ThroughTime(time))
    );
    assert!(
        !scheduler.output.time_closed(time),
        "report escaped before root consumed Committed"
    );
    assert_eq!(
        wait.wait_timeout(HostDuration::ZERO),
        Err(OutputWaitError::Timeout)
    );
    scheduler.core().service_output_proof().unwrap();
    assert!(scheduler.output.proof.root.is_none());
    assert!(scheduler.output.proof.participant.is_none());
    assert!(!scheduler.output.time_closed(time));
    assert_eq!(
        scheduler.output.progress.sealed,
        Some(OutputBound::ThroughTime(time))
    );
    while let Some(event) = scheduler.event_rx.try_recv().unwrap() {
        scheduler.core().handle_async_event(event).unwrap();
    }
    let retry = observer.close_time(time).unwrap();
    drive_time_close(&mut scheduler, time);
    retry.wait_timeout(HostDuration::from_secs(1)).unwrap();
}

#[test]
fn output_time_close_caught_panic_fails_two_participants_without_another_turn() {
    for panic_at_root in [false, true] {
        let (mut a, owner, a_reaction, a_calls) =
            fixture_at(EnclaveKey::from(1), OutputOutcome::Committed);
        let (mut b, _, b_reaction, b_calls) =
            fixture_at(EnclaveKey::from(2), OutputOutcome::Committed);
        connect_output_pair(
            &mut a,
            &mut b,
            Some(Duration::nanoseconds(1)),
            Some(Duration::nanoseconds(1)),
        );
        let wait = a
            .output_observer(owner)
            .unwrap()
            .close_time(crate::clock::PhysicalInstant(10))
            .unwrap();
        a.startup();
        b.startup();
        // Drive discovery only. No participant executes a payload during setup.
        for _ in 0..16 {
            a.event_tx.try_send(AsyncEvent::OutputWake).unwrap();
            assert!(a.try_next().unwrap());
            while let Some(event) = b.event_rx.try_recv().unwrap() {
                b.core().handle_async_event(event).unwrap();
            }
            if b.output.proof.participant.is_some() {
                break;
            }
        }
        let lease = b
            .output
            .proof
            .participant
            .as_ref()
            .expect("peer reserved")
            .lease
            .clone();
        assert!(a.output.proof.participant.is_some());
        while let Some(event) = a.event_rx.try_recv().unwrap() {
            a.core().handle_async_event(event).unwrap();
        }
        let (failed, other, reaction, calls) = if panic_at_root {
            (&mut a, &mut b, a_reaction, a_calls)
        } else {
            (&mut b, &mut a, b_reaction, b_calls)
        };
        let _ = std::panic::catch_unwind(|| {
            let _guard = calls.lock().unwrap();
            panic!("poison the device fixture");
        });
        let completed_before_panic = failed.current_tag;
        failed
            .events
            .push_event(Tag::ZERO, [(Level::from(0), reaction)], false);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| failed.try_next())).is_err()
        );
        // Inspect before dropping or stepping the failed scheduler again.
        assert_eq!(lease.error.get(), Some(&OutputWaitError::ExecutionFailed));
        let mut woke = false;
        let mut received = Vec::new();
        while let Some(event) = other.event_rx.try_recv().unwrap() {
            received.push(event.kind_str());
            woke |= matches!(event, AsyncEvent::OutputWake);
            other.core().handle_async_event(event).unwrap();
        }
        // A full mailbox already wakes the root; abort must not block on it.
        assert!(
            woke || received.len() == other.event_rx.capacity(),
            "unwind wake missing: root={panic_at_root}, received={received:?}"
        );
        other.core().service_output_proof().unwrap();
        assert_eq!(
            wait.wait_timeout(HostDuration::from_millis(20)),
            Err(OutputWaitError::ExecutionFailed)
        );
        // An interrupted tag has no valid completed result. Resumption must stop.
        failed.events.push_event(
            Tag::new(Duration::nanoseconds(1), 0),
            [(Level::from(0), reaction)],
            false,
        );
        assert!(!failed.try_next().unwrap());
        assert!(!failed.try_next().unwrap());
        assert_eq!(failed.current_tag, completed_before_panic);
        assert!(failed.output.progress.sealed.is_none());
    }
}

#[cfg(feature = "hosted-manual-clock")]
fn check_output_terminal_clock_before_physical_wait(terminate: fn(&crate::clock::ManualClock)) {
    use crate::clock::{ExecutionClock, ManualClock, PhysicalClockDomainId, PhysicalInstant};
    let mut results = Vec::new();
    for mailbox_closed in [false, true] {
        let (mut scheduler, owner, _, calls) = fixture(OutputOutcome::Committed);
        let clock = ManualClock::new(PhysicalClockDomainId(20)).unwrap();
        scheduler.set_clock(ExecutionClock::manual(clock.clone(), 0));
        let observer = scheduler.output_observer(owner).unwrap();
        let wait = observer.close_time(PhysicalInstant(10)).unwrap();
        scheduler.startup();
        let request = scheduler.event_rx.try_recv().unwrap().unwrap();
        scheduler.core().handle_async_event(request).unwrap();
        let before = scheduler.current_tag;

        // The clock terminates after the step checks it, before wait registration.
        assert!(!scheduler.clock.failed());
        terminate(&clock);
        if mailbox_closed {
            scheduler.event_tx.close().unwrap();
        }
        assert_eq!(
            scheduler
                .core()
                .synchronize_next_tag(Tag::new(Duration::nanoseconds(10), 0))
                .unwrap(),
            Some(false)
        );
        results.push(wait.wait_timeout(HostDuration::from_secs(1)));
        assert!(!scheduler.output.time_closed(PhysicalInstant(10)));
        assert_eq!(scheduler.current_tag, before);
        assert!(calls.lock().unwrap().is_empty());
    }
    assert_eq!(
        results,
        vec![
            Err(OutputWaitError::ExecutionFailed),
            Err(OutputWaitError::ExecutionFailed)
        ]
    );
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_checkpoint_failed_clock_before_physical_wait_is_execution_failed() {
    check_output_terminal_clock_before_physical_wait(crate::clock::ManualClock::fail);
}

#[cfg(feature = "hosted-manual-clock")]
#[test]
fn output_checkpoint_closed_clock_before_physical_wait_is_execution_failed() {
    check_output_terminal_clock_before_physical_wait(crate::clock::ManualClock::close);
}

#[test]
fn output_checkpoint_explicit_abort_with_healthy_clock_is_canceled() {
    use crate::sched::federate::{
        FederateCoordinationParts, LifecyclePolicy, LocalFederateCoordinationBackend,
    };
    let (mut scheduler, owner, _, calls) = fixture(OutputOutcome::Committed);
    let tag = Tag::new(Duration::days(1), 0);
    let wait = scheduler
        .output_observer(owner)
        .unwrap()
        .request_tag(tag)
        .unwrap();
    scheduler.startup();
    let request = scheduler.event_rx.try_recv().unwrap().unwrap();
    scheduler.core().handle_async_event(request).unwrap();
    let before = scheduler.current_tag;
    let parts = FederateCoordinationParts::new(
        [(
            crate::image::EnclaveIndex::new(0),
            scheduler.event_tx.clone(),
            scheduler.event_rx.clone(),
        )],
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    let (_, mut port) = parts.participants.into_iter().next().unwrap();
    parts.abort_handle.abort();
    parts.coordinator.run().coordination_result.unwrap();
    assert!(!scheduler.clock.failed());
    let mut core = scheduler.core();
    core.federate_coordination = Some(&mut port);
    assert_eq!(core.synchronize_next_tag(tag).unwrap(), Some(false));
    drop(core);
    assert_eq!(
        wait.wait_timeout(HostDuration::from_secs(1)),
        Err(OutputWaitError::Canceled)
    );
    assert_eq!(scheduler.current_tag, before);
    assert!(calls.lock().unwrap().is_empty());
}
