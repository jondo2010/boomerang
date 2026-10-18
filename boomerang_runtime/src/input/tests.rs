//! Admission, exclusive progress, generation fencing, and bounded retention tests.

use super::*;
use crate::{
    clock::{ExecutionEpoch, ManualClock, PhysicalClockDomainId},
    image::ActionIndex,
    input::{InputSource, InputValue},
};
/// Checks that unknown required frontiers block finite tags and no required sources leaves no cap.
#[test]
fn required_without_frontier_blocks_all_finite_tags() {
    assert_eq!(aggregate([None].into_iter()), Tag::NEVER);
    assert_eq!(aggregate([Some(Tag::ZERO), None].into_iter()), Tag::NEVER);
    assert_eq!(aggregate([Some(Tag::ZERO)].into_iter()), Tag::ZERO);
    assert_eq!(aggregate([].into_iter()), Tag::FOREVER);
}
use crate::sched::federate::{
    state::SchedulerMessage, FederateCoordinationParts, LifecyclePolicy,
    LocalFederateCoordinationBackend,
};
/// Admission service, participant receivers, and retained local coordinator fixture.
struct Harness {
    /// Independent manual time control retained only by the test driver.
    clock: ManualClock,
    inputs: InputAdmission,
    rx: Vec<crate::Receiver<AsyncEvent>>,
    _coordination: FederateCoordinationParts<LocalFederateCoordinationBackend>,
}
/// Deliberately sparse original compiled identities, unrelated to fixture vector positions.
fn enclave_key(slot: usize) -> EnclaveIndex {
    EnclaveIndex::new(7 + 3 * u32::try_from(slot).unwrap())
}
/// Constructs bounded participants and resolved sources with initial grant authorization.
fn harness(required: &[bool], enclaves: usize, delay: i64, capacity: usize) -> Harness {
    let channels: Vec<_> = (0..enclaves).map(|_| kanal::bounded(capacity)).collect();
    let coordination = FederateCoordinationParts::new(
        channels
            .iter()
            .enumerate()
            .map(|(i, (tx, rx))| (enclave_key(i), tx.clone(), rx.clone())),
        LifecyclePolicy::KeepAlive,
        LocalFederateCoordinationBackend::default(),
    )
    .unwrap();
    let sources = required
        .iter()
        .enumerate()
        .map(|(i, required)| InputSource {
            id: format!("s{i}"),
            required: *required,
            actions: (0..enclaves)
                .map(|enclave| {
                    CompiledPhysicalAction::new(enclave_key(enclave), ActionIndex::from(i))
                })
                .collect(),
        })
        .collect();
    let clock = ManualClock::new(PhysicalClockDomainId(7)).unwrap();
    let inputs = InputAdmission::new(
        InputConfig {
            max_batch_values: enclaves * 2,
            max_staged_batches: 2,
            sources,
        },
        required
            .iter()
            .map(|_| Duration::nanoseconds(delay))
            .collect(),
        clock.clone(),
        channels
            .iter()
            .enumerate()
            .map(|(i, (tx, _))| (enclave_key(i), tx.clone()))
            .collect(),
        coordination.coordinator.state.shared(),
        coordination.abort_handle.clone(),
    );
    {
        let mut state = inputs.0.coordination.lock().unwrap();
        for slot in 0..enclaves {
            state
                .handle_scheduler(SchedulerMessage::Publish {
                    enclave: enclave_key(slot),
                    next_event: Some(Tag::ZERO),
                })
                .unwrap();
        }
        let revision = state.revision();
        state
            .handle_acquisition(crate::sched::federate::FederateAcquisition::new(
                revision,
                Tag::FOREVER,
            ))
            .unwrap();
    }
    Harness {
        clock,
        inputs,
        rx: channels.into_iter().map(|(_, rx)| rx).collect(),
        _coordination: coordination,
    }
}
impl Harness {
    /// Creates fixture source identity, including malformed keys for rejection tests.
    fn source(&self, source: usize) -> InputSourceHandle {
        InputSourceHandle {
            key: SourceKey::new(source as u32),
            admission: self.inputs.clone(),
        }
    }
    /// Bind a compiled action to its fixture source with the configured minimum delay.
    fn action(&self, target: usize) -> crate::input::SourceAction<u32> {
        let source = target / self.rx.len();
        let enclave = target % self.rx.len();
        let delay = self.inputs.0.state.lock().unwrap().sources[SourceKey::from(source)].delay;
        crate::input::SourceAction {
            reference: CompiledPhysicalAction::new(enclave_key(enclave), ActionIndex::from(source)),
            source: self.source(source),
            delay,
            payload: std::marker::PhantomData,
        }
    }
    /// Crosses the real coordinator's final execution boundary.
    fn reserve(&self, enclave: usize, tag: Tag) -> bool {
        self.inputs
            .0
            .coordination
            .lock()
            .unwrap()
            .begin_tag(enclave_key(enclave), tag)
    }
    /// Reports completion through the existing coordinator state transition.
    fn complete(&self, enclave: usize, tag: Tag) {
        self.inputs
            .0
            .coordination
            .lock()
            .unwrap()
            .handle_scheduler(SchedulerMessage::CompleteTag {
                enclave: enclave_key(enclave),
                tag,
                network_input: false,
            })
            .unwrap();
    }
}
/// Creates a u32 observation for a source's first target using the fixture clock identity.
fn sample(h: &Harness, source: usize, sequence: u64, time: u64) -> InputObservation {
    InputObservation {
        source: h.source(source),
        sequence,
        acquired: PhysicalInstant(time),
        domain: h.inputs.0.clock.domain(),
        epoch: h.inputs.0.clock.epoch(),
        values: vec![InputValue::new(&h.action(source * h.rx.len()), 42u32)],
    }
}
/// Authorizes all participants at the current cached horizon and generation.
fn authorize(h: &Harness) {
    let mut state = h.inputs.0.coordination.lock().unwrap();
    for slot in 0..h.rx.len() {
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave: enclave_key(slot),
                next_event: Some(Tag::ZERO),
            })
            .unwrap();
    }
    let revision = state.revision();
    state
        .handle_acquisition(crate::sched::federate::FederateAcquisition::new(
            revision,
            Tag::FOREVER,
        ))
        .unwrap();
}
/// Drains the existing participant mailboxes; later acquisition uses coordinator publication fences.
fn drain(h: &Harness) {
    for rx in &h.rx {
        while rx.try_recv().unwrap().is_some() {}
    }
}
/// Checks exclusive frontiers, zero-delay mapping, clock independence, and optional sources.
#[test]
fn exclusive_frontiers_zero_delay_independence_and_optional_idle_sources() {
    let h = harness(&[true, true, false], 2, 3, 8);
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
    h.inputs.advance(&h.source(0), PhysicalInstant(0)).unwrap();
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
    h.inputs.advance(&h.source(1), PhysicalInstant(0)).unwrap();
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(2), usize::MAX)
    );
    h.inputs
        .advance(&h.source(0), PhysicalInstant(100))
        .unwrap();
    h.inputs.advance(&h.source(1), PhysicalInstant(10)).unwrap();
    assert_eq!(h.inputs.0.clock.now().unwrap(), PhysicalInstant(0));
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(12), usize::MAX)
    );
    drain(&h);
    h.inputs.advance(&h.source(1), PhysicalInstant(10)).unwrap();
    assert!(h.rx[0].try_recv().unwrap().is_none());
    h.inputs.disconnect(&h.source(2));
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(12), usize::MAX)
    );
    assert_eq!(
        h.inputs
            .advance(&h.source(1), PhysicalInstant(9))
            .unwrap_err()
            .kind,
        InputErrorKind::Protocol
    );
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
}
/// Checks rejected observations preserve sequence state and frontier equality is admissible.
#[test]
fn admission_rejections_preserve_sequence_and_exact_frontier_is_admissible() {
    let h = harness(&[true], 2, 0, 8);
    let start = Instant::now();
    assert_eq!(
        h.inputs.disconnect(&h.source(usize::MAX)).kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        h.inputs
            .advance(&h.source(usize::MAX), PhysicalInstant(1))
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    let mut invalid = sample(&h, 0, 1, 0);
    invalid.epoch = ExecutionEpoch(0);
    assert_eq!(
        h.inputs.submit(vec![invalid], &[]).unwrap_err().kind,
        InputErrorKind::Malformed
    );
    let mut invalid = sample(&h, 0, 1, 0);
    invalid.values = vec![InputValue {
        action: h.action(0).admission(),
        value: Box::new("wrong"),
    }];
    assert_eq!(
        h.inputs.submit(vec![invalid], &[]).unwrap_err().kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 1, 10)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Future
    );
    h.clock.advance_to(PhysicalInstant(20)).unwrap();
    h.inputs.advance(&h.source(0), PhysicalInstant(10)).unwrap();
    let receipt = h.inputs.submit(vec![sample(&h, 0, 2, 10)], &[]).unwrap();
    assert!(receipt.arrival >= start);
    assert_eq!(receipt.tags, [Tag::new(Duration::nanoseconds(10), 0)]);
    for (sequence, time, kind) in [
        (2, 10, InputErrorKind::Duplicate),
        (1, 10, InputErrorKind::OutOfOrder),
        (3, 9, InputErrorKind::Protocol),
    ] {
        let failure = h
            .inputs
            .submit(vec![sample(&h, 0, sequence, time)], &[])
            .unwrap_err();
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.source_id.as_deref(), Some("s0"));
        assert!(failure.arrival >= start);
    }
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
}
/// Checks retained-batch limits, target/tag conflicts, and lateness against reserved execution.
#[test]
fn retained_bounds_same_tag_conflicts_and_inflight_lateness() {
    let h = harness(&[false], 2, 0, 8);
    h.clock.advance_to(PhysicalInstant(30)).unwrap();
    h.inputs.submit(vec![sample(&h, 0, 1, 10)], &[]).unwrap();
    assert!(!h.reserve(0, Tag::new(Duration::nanoseconds(10), 0)));
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 2, 10)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Overflow
    );
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 2, 9)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::OutOfOrder
    );
    h.inputs.submit(vec![sample(&h, 0, 2, 11)], &[]).unwrap();
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 3, 12)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Overflow
    );
    drain(&h);
    authorize(&h);
    assert!(h.reserve(0, Tag::new(Duration::nanoseconds(12), 0)));
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 3, 12)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Late
    );
    h.complete(0, Tag::new(Duration::nanoseconds(12), 0));
    h.inputs.submit(vec![sample(&h, 0, 3, 13)], &[]).unwrap();
    h.inputs.disconnect(&h.source(0));
    drain(&h);
    authorize(&h);
    assert!(h.reserve(0, Tag::new(Duration::nanoseconds(13), 0)));
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 1);
}
/// Checks whole-batch rejection before commit and execution abortion on partial publication.
#[test]
fn entire_batch_rejects_before_commit_and_partial_fanout_aborts() {
    let h = harness(&[false], 2, 0, 1);
    let mut malformed = sample(&h, 0, 1, 0);
    malformed.values.push(InputValue {
        action: h.action(1).admission(),
        value: Box::new(false),
    });
    assert_eq!(
        h.inputs.submit(vec![malformed], &[]).unwrap_err().kind,
        InputErrorKind::Malformed
    );
    assert!(h.rx.iter().all(|rx| rx.try_recv().unwrap().is_none()));
    h.inputs.0.senders[enclave_key(1)]
        .try_send(AsyncEvent::FederateResume)
        .unwrap();
    let mut batch = sample(&h, 0, 1, 0);
    batch.values.push(InputValue::new(&h.action(1), 7u32));
    assert_eq!(
        h.inputs
            .submit(vec![batch], &[(h.source(0), PhysicalInstant(1))])
            .unwrap_err()
            .kind,
        InputErrorKind::Overflow
    );
    assert!(matches!(
        h.rx[0].try_recv().unwrap(),
        Some(AsyncEvent::PhysicalBatch(_))
    ));
    assert!(!h.reserve(0, Tag::ZERO));
    assert!(!h
        .inputs
        .0
        .coordination
        .lock()
        .unwrap()
        .begin_tag(enclave_key(0), Tag::ZERO));
    assert!(h.inputs.0.state.lock().unwrap().sources[SourceKey::new(0)]
        .frontier
        .is_none());
}
/// Checks terminal required-source failures and rejection of invalid time mappings.
#[test]
fn required_overflow_disconnect_and_checked_mapping_are_terminal_or_rejected() {
    use InputErrorKind::{Disconnected, Overflow};
    for (sources, mode) in [[false, false], [false, true], [true, false]]
        .into_iter()
        .flat_map(|sources| [0, 1, 2].map(|mode| (sources, mode)))
    {
        let mut h = harness(&sources, 1 + usize::from(mode == 1), 0, 1);
        if mode == 2 {
            Arc::get_mut(&mut h.inputs.0).unwrap().max_values = 1;
        }
        let required = sources.contains(&true);
        h.inputs.0.senders[enclave_key(0)]
            .try_send(AsyncEvent::FederateResume)
            .unwrap();
        if mode == 1 {
            h.rx[0].close().unwrap();
        }
        let mut first = sample(&h, 0, 1, 0);
        first.values[0].action = h.action(h.rx.len() - 1).admission(); // Last Enclave first.
        let failure = h
            .inputs
            .submit(vec![first, sample(&h, 1, 1, 0)], &[])
            .unwrap_err();
        assert_eq!(
            failure.kind,
            if mode == 1 { Disconnected } else { Overflow }
        );
        let state = h.inputs.0.state.lock().unwrap();
        assert_eq!(state.failure.is_some(), required);
        if !required && mode == 1 {
            assert_eq!(failure.source_id.as_deref(), Some("s1"));
            assert!(
                state.sources[SourceKey::new(0)].enabled
                    && !state.sources[SourceKey::new(1)].enabled
            );
        }
        assert!(
            state.retained.len() == 0 && state.sources.values().all(|source| source.last.is_none())
        );
        drop(state);
        assert_eq!(h.reserve(0, Tag::ZERO), !required);
        h.inputs.disconnect(&h.source(0));
        assert_eq!(h.inputs.0.state.lock().unwrap().failure.is_some(), required);
    }
    let h = harness(&[true], 2, 1, 8);
    h.clock.advance_to(PhysicalInstant(u64::MAX)).unwrap();
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 1, u64::MAX)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        h.inputs
            .advance(&h.source(0), PhysicalInstant(u64::MAX))
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    assert!(h.inputs.0.state.lock().unwrap().sources[SourceKey::new(0)]
        .last
        .is_none());
}
/// Checks admission and final execution reservation cannot both win for conflicting work.
#[test]
fn admission_and_final_reservation_race_cannot_both_win() {
    for _ in 0..64 {
        let h = harness(&[false], 2, 0, 8);
        let barrier = std::sync::Barrier::new(2);
        let observation = sample(&h, 0, 1, 0);
        let inputs = &h.inputs;
        std::thread::scope(|scope| {
            let admitted = scope.spawn(|| {
                barrier.wait();
                inputs.submit(vec![observation], &[])
            });
            barrier.wait();
            let reserved = h.reserve(0, Tag::ZERO);
            match admitted.join().unwrap() {
                Ok(_) => assert!(!reserved),
                Err(error) => {
                    assert!(reserved);
                    assert_eq!(error.kind, InputErrorKind::Late);
                }
            }
        });
    }
}

/// Measures allocation-free horizon reads and reservations with maximum retained batches.
#[test]
fn cached_horizon_and_reservations_allocate_nothing_with_maximum_batches() {
    use crate::clock::ALLOCATIONS;
    for enclaves in [2, 16] {
        for count in [1, 16, 64] {
            let h = harness(&vec![true; count], enclaves, 0, 8);
            h.clock.advance_to(PhysicalInstant(4)).unwrap();
            for start in [0, 2] {
                let batch = (start..start + 2)
                    .map(|time| {
                        let mut observation = sample(&h, 0, time, time);
                        observation.values = (0..enclaves)
                            .map(|slot| InputValue::new(&h.action(slot), 1u32))
                            .collect();
                        observation
                    })
                    .collect();
                assert_eq!(
                    h.inputs.submit(batch, &[]).unwrap().tags.len(),
                    enclaves * 2
                );
            }
            for source in 0..count {
                h.inputs
                    .advance(&h.source(source), PhysicalInstant(5))
                    .unwrap();
            }
            drain(&h);
            authorize(&h);
            let start = Instant::now();
            ALLOCATIONS.with(|n| n.set(Some(0)));
            for _ in 0..100_000 {
                std::hint::black_box(h.inputs.horizon());
                assert!(h.reserve(0, Tag::ZERO));
            }
            let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
            assert_eq!(allocations, 0);
            eprintln!("input measurement: enclaves={enclaves} sources={count} max_values={} retained=2 checks=100000 allocations={allocations} elapsed={:?}", enclaves * 2, start.elapsed());
            for slot in 0..enclaves {
                h.complete(slot, Tag::new(Duration::nanoseconds(3), 0));
            }
            h.inputs.submit(vec![], &[]).unwrap();
            assert!(h.inputs.0.state.lock().unwrap().retained.len() == 0);
        }
    }
}

/// Marks fixture participants active and publishes the next tag.
fn republish(h: &mut Harness, tag: Tag) {
    let mut state = h._coordination.coordinator.state.lock();
    for slot in 0..h.rx.len() {
        let enclave = enclave_key(slot);
        state
            .handle_scheduler(SchedulerMessage::Active { enclave })
            .unwrap();
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: Some(tag),
            })
            .unwrap();
    }
}
/// Checks consumed batches invalidate queued grants until coordinator reauthorization.
#[test]
fn consumed_batch_invalidates_queued_grant_until_coordinator_reauthorizes() {
    use crate::sched::federate::FederateAcquisition;
    let mut h = harness(&[false], 2, 0, 8);
    h.clock.advance_to(PhysicalInstant(10)).unwrap();
    let tag = Tag::new(Duration::nanoseconds(10), 0);
    republish(&mut h, tag);
    let revision = h._coordination.coordinator.state.lock().revision();
    h._coordination
        .coordinator
        .state
        .lock()
        .handle_acquisition(FederateAcquisition::new(revision, tag))
        .unwrap();
    h.inputs.submit(vec![sample(&h, 0, 1, 10)], &[]).unwrap();
    drain(&h);
    // A backend extension computed for the old publication must not bless the new generation.
    h._coordination
        .coordinator
        .state
        .lock()
        .handle_acquisition(FederateAcquisition::new(revision, Tag::FOREVER))
        .unwrap();
    h._coordination
        .coordinator
        .state
        .lock()
        .input_progress()
        .unwrap();
    assert!(!h.reserve(0, tag), "a queued grant from before batch admission must be stale even after its envelope was consumed");
    republish(&mut h, tag);
    let revision = h._coordination.coordinator.state.lock().revision();
    h._coordination
        .coordinator
        .state
        .lock()
        .handle_acquisition(FederateAcquisition::new(revision, tag))
        .unwrap();
    assert!(h.reserve(0, tag));
}

/// Checks retained backend authority releases successive input frontiers.
#[test]
fn settled_acquisition_releases_successive_frontiers_through_coordinator() {
    use crate::sched::federate::{
        FederateAcquisition, FederateSchedulerCoordination, FederateTagAcquisition,
    };
    let mut h = harness(&[true], 2, 0, 8);
    let inputs = h.inputs.clone();
    let tag = Tag::new(Duration::nanoseconds(2), 0);
    republish(&mut h, tag);
    let mut state = h._coordination.coordinator.state.lock();
    // The backend's acquisition has been consumed and capped before the workers wait.
    let revision = state.revision();
    state
        .handle_acquisition(FederateAcquisition::new(revision, tag))
        .unwrap();
    assert_eq!(state.grant_horizon(), Some(Tag::NEVER));
    drop(state);
    for frontier in [1, 2] {
        inputs
            .advance(&h.source(0), PhysicalInstant(frontier))
            .unwrap();
        h._coordination
            .coordinator
            .state
            .lock()
            .input_progress()
            .unwrap();
        assert_eq!(
            h._coordination.coordinator.state.read().grant_horizon(),
            Some(inputs.horizon())
        );
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let source = h.source(0);
    let parts = h._coordination;
    let results = std::thread::scope(|scope| {
        scope.spawn(move || parts.coordinator.run());
        for (_, mut port) in parts.participants {
            let tx = tx.clone();
            scope.spawn(move || tx.send(port.acquire_tag(tag)).unwrap());
        }
        inputs.advance(&source, PhysicalInstant(3)).unwrap();
        let results: Vec<_> = (0..2)
            .map(|_| rx.recv_timeout(std::time::Duration::from_secs(2)))
            .collect();
        parts.abort_handle.abort();
        results
    });
    assert!(results
        .into_iter()
        .all(|result| matches!(result.unwrap().unwrap(), FederateTagAcquisition::Granted)));
}

/// Run identity fences both source protocol messages and otherwise identical action keys.
#[test]
fn source_and_action_handles_cannot_cross_execution_epochs() {
    let first = harness(&[false], 2, 0, 8);
    let second = harness(&[false], 2, 0, 8);
    let mut foreign_source = sample(&second, 0, 1, 0);
    foreign_source.source = first.source(0);
    assert_eq!(
        second
            .inputs
            .submit(vec![foreign_source], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    let mut foreign_action = sample(&second, 0, 1, 0);
    foreign_action.values = vec![InputValue::new(&first.action(0), 1)];
    assert_eq!(
        second
            .inputs
            .submit(vec![foreign_action], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        second
            .inputs
            .advance(&first.source(0), PhysicalInstant(1))
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    second
        .inputs
        .submit(vec![sample(&second, 0, 1, 0)], &[])
        .unwrap();
}

/// Cloning ingress handles never requires cloning the uniquely owned payload.
#[test]
fn source_action_handle_clones_for_nonclone_payload() {
    #[derive(Debug)]
    struct UniquePayload;
    let h = harness(&[false], 2, 0, 8);
    let action = crate::input::SourceAction::<UniquePayload> {
        reference: CompiledPhysicalAction::new(enclave_key(0), ActionIndex::new(0)),
        source: h.source(0),
        delay: Duration::ZERO,
        payload: std::marker::PhantomData,
    };
    let cloned = action.clone();
    let value = InputValue::new(&cloned, UniquePayload);
    assert_eq!(value.action.reference, action.reference);
}

/// An idle non-target participant keeps its valid idle publication across another Enclave's input.
#[test]
fn coherent_input_does_not_require_idle_nontarget_to_republish() {
    use crate::sched::federate::state::CoordinationAction;
    let h = harness(&[false], 2, 0, 8);
    {
        let mut state = h.inputs.0.coordination.lock().unwrap();
        for slot in 0..2 {
            state
                .handle_scheduler(SchedulerMessage::Publish {
                    enclave: enclave_key(slot),
                    next_event: None,
                })
                .unwrap();
        }
    }
    // Only Enclave 7 receives a batch. Enclave 10 remains in wait(), where a
    // FederateResume wake is consumed without publishing another idle candidate.
    h.inputs.submit(vec![sample(&h, 0, 1, 0)], &[]).unwrap();
    drain(&h);
    let mut state = h.inputs.0.coordination.lock().unwrap();
    state
        .handle_scheduler(SchedulerMessage::Active {
            enclave: enclave_key(0),
        })
        .unwrap();
    let actions = state
        .handle_scheduler(SchedulerMessage::Publish {
            enclave: enclave_key(0),
            next_event: Some(Tag::ZERO),
        })
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, CoordinationAction::Publish(_))),
        "unaffected idle candidate must participate without an unsolicited republish"
    );
}

/// Real idle/acquisition ports need no synthetic republish after another participant receives input.
#[test]
fn coherent_input_releases_target_with_unaffected_waiting_port() {
    use crate::sched::federate::{
        FederateIdleWait, FederateSchedulerCoordination, FederateTagAcquisition,
    };
    for idle in [true, false] {
        let h = harness(&[!idle], 2, 0, 8);
        let observation = sample(&h, 0, 1, 0);
        let source = h.source(0);
        let inputs = h.inputs.clone();
        let state = inputs.0.coordination.clone();
        let parts = h._coordination;
        let mut ports = parts.participants.into_iter();
        let (_, mut target) = ports.next().unwrap();
        let (_, mut unaffected) = ports.next().unwrap();
        let future = Tag::new(Duration::nanoseconds(10), 0);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || parts.coordinator.run());
            let tx = result_tx.clone();
            scope.spawn(move || {
                let success = if idle {
                    matches!(unaffected.wait(), Ok(FederateIdleWait::Aborted))
                } else {
                    matches!(
                        unaffected.acquire_tag(future),
                        Ok(FederateTagAcquisition::Granted)
                    )
                };
                tx.send((false, success)).unwrap();
            });
            // This state change acknowledges that the real port's initial Publish
            // was processed, without any host sleep or fixture republish.
            let bound = Instant::now() + std::time::Duration::from_secs(2);
            let candidate = if idle { None } else { Some(future) };
            let ready = loop {
                if state.lock().unwrap().candidate(enclave_key(1)) == Some(candidate) {
                    break true;
                }
                if Instant::now() >= bound {
                    break false;
                }
                std::thread::yield_now();
            };
            if !ready {
                parts.abort_handle.abort();
                panic!("unaffected port did not publish its initial candidate");
            }
            inputs
                .submit(vec![observation], &[(source, PhysicalInstant(11))])
                .unwrap();
            scope.spawn(move || {
                let batch = matches!(
                    target.acquire_tag(Tag::ZERO),
                    Ok(FederateTagAcquisition::Interrupted(
                        AsyncEvent::PhysicalBatch(_)
                    ))
                );
                let granted = batch
                    && matches!(
                        target.acquire_tag(Tag::ZERO),
                        Ok(FederateTagAcquisition::Granted)
                    )
                    && target.begin_tag(Tag::ZERO);
                if granted {
                    target.logical_tag_complete(Tag::ZERO, false).unwrap();
                }
                result_tx.send((true, granted)).unwrap();
                // A scheduler reports its completed work and then publishes idle;
                // otherwise its consumed candidate correctly blocks a sibling's future tag.
                let _ = target.wait();
            });
            let first = result_rx.recv_timeout(std::time::Duration::from_secs(2));
            let second = if idle {
                parts.abort_handle.abort();
                result_rx.recv_timeout(std::time::Duration::from_secs(2))
            } else {
                let second = result_rx.recv_timeout(std::time::Duration::from_secs(2));
                parts.abort_handle.abort();
                second
            };
            assert!(
                first.is_ok_and(|(_, success)| success),
                "first port failed, idle={idle}"
            );
            assert!(
                second.is_ok_and(|(_, success)| success),
                "second port failed, idle={idle}"
            );
        });
    }
}

/// A partially completed batch still consumes one staging slot until its last target finishes.
#[test]
fn retained_batches_reclaim_complete_batches_without_releasing_partial_batches() {
    let h = harness(&[false], 2, 0, 16);
    h.clock.advance_to(PhysicalInstant(40)).unwrap();
    for (sequence, time) in [(1, 10), (2, 20)] {
        let mut observation = sample(&h, 0, sequence, time);
        observation.values.push(InputValue::new(&h.action(1), 7));
        h.inputs.submit(vec![observation], &[]).unwrap();
    }
    drain(&h);
    h.complete(0, Tag::new(Duration::nanoseconds(10), 0));
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 3, 30)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Overflow
    );
    h.complete(1, Tag::new(Duration::nanoseconds(10), 0));
    h.inputs.submit(vec![sample(&h, 0, 3, 30)], &[]).unwrap();
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 2);
    h.complete(0, Tag::new(Duration::nanoseconds(20), 0));
    h.inputs.submit(vec![], &[]).unwrap();
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 2);
    h.complete(1, Tag::new(Duration::nanoseconds(20), 0));
    h.inputs.submit(vec![], &[]).unwrap();
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 1);
    h.complete(0, Tag::new(Duration::nanoseconds(30), 0));
    h.inputs.submit(vec![], &[]).unwrap();
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 0);
}

/// Retention storage reuses previously grown capacity across batch turnover.
#[test]
fn retained_batch_turnover_does_not_allocate() {
    let mut ledger = RetainedBatches::new();
    let reservation = |enclave, nanoseconds| Reservation {
        action: CompiledPhysicalAction::new(enclave_key(enclave), ActionIndex::new(0)),
        tag: Tag::new(Duration::nanoseconds(nanoseconds), 0),
    };
    let first = [reservation(0, 10), reservation(1, 10)];
    let second = [reservation(0, 20), reservation(1, 20)];
    ledger.append(&first);
    ledger.append(&second);
    ledger.reclaim(|_| false);
    crate::clock::ALLOCATIONS.with(|count| count.set(Some(0)));
    ledger.append(&first);
    ledger.append(&second);
    ledger.reclaim(|entry| entry.action.enclave != enclave_key(0));
    ledger.reclaim(|entry| entry.tag != first[0].tag);
    ledger.append(&first);
    ledger.reclaim(|_| false);
    let allocations = crate::clock::ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(ledger.len(), 0);
}

/// Published source batches keep compiled action rows, independent of storage keys.
#[test]
fn source_batch_preserves_compiled_action_identity() {
    let h = harness(&[false, false], 2, 3, 8);
    let mut observation = sample(&h, 1, 1, 0);
    observation.values.push(InputValue::new(&h.action(3), 7u32));
    h.inputs.submit(vec![observation], &[]).unwrap();
    for (rx, expected) in h.rx.iter().zip([42u32, 7]) {
        let Some(AsyncEvent::PhysicalBatch(batch)) = rx.try_recv().unwrap() else {
            panic!("source admission must publish a batch to each destination");
        };
        let mut values = batch.values.into_iter();
        let (tag, action, value) = values.next().unwrap();
        assert_eq!(action, ActionIndex::new(1));
        assert_eq!(tag, Tag::new(Duration::nanoseconds(3), 0));
        assert_eq!(*value.downcast::<u32>().ok().unwrap(), expected);
        assert!(values.next().is_none());
    }
}
