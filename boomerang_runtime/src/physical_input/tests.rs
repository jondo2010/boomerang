//! Admission, exclusive progress, generation fencing, and bounded retention tests.

use super::*;
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
    inputs: InputAdmission,
    rx: Vec<crate::Receiver<AsyncEvent>>,
    _coordination: FederateCoordinationParts<LocalFederateCoordinationBackend>,
}
/// Constructs bounded participants and resolved sources with initial grant authorization.
fn harness(required: &[bool], enclaves: usize, delay: i64, capacity: usize) -> Harness {
    let channels: Vec<_> = (0..enclaves).map(|_| kanal::bounded(capacity)).collect();
    let coordination = FederateCoordinationParts::new(
        channels
            .iter()
            .enumerate()
            .map(|(i, (tx, rx))| (EnclaveIndex::new(i as u32), tx.clone(), rx.clone())),
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
            targets: vec![],
        })
        .collect();
    let targets = required
        .iter()
        .enumerate()
        .flat_map(|(i, _)| {
            (0..enclaves).map(move |enclave| Target {
                id: format!("t{enclave}"),
                source: SourceKey(i),
                enclave,
                action: crate::ActionKey::from(i),
                delay: Duration::nanoseconds(delay),
                payload: TypeId::of::<u32>(),
            })
        })
        .collect();
    let inputs = InputAdmission::new(
        InputConfig {
            max_batch_values: enclaves * 2,
            max_staged_batches: 2,
            sources,
        },
        targets,
        ManualClock::new(PhysicalClockDomainId(7)).unwrap(),
        &channels
            .iter()
            .map(|(tx, _)| tx.clone())
            .collect::<Vec<_>>(),
        coordination.abort_handle.clone(),
    );
    for slot in 0..enclaves {
        inputs.authorize(slot, Tag::FOREVER, 0);
    }
    Harness {
        inputs,
        rx: channels.into_iter().map(|(_, rx)| rx).collect(),
        _coordination: coordination,
    }
}
/// Creates a u32 observation for a source's first target using the fixture clock identity.
fn sample(h: &Harness, source: usize, sequence: u64, time: u64) -> InputObservation {
    InputObservation {
        source: SourceKey(source),
        sequence,
        acquired: PhysicalInstant(time),
        domain: h.inputs.0.clock.domain(),
        epoch: h.inputs.0.clock.epoch(),
        values: vec![InputValue::new(TargetKey(source * h.rx.len()), 42u32)],
    }
}
/// Authorizes all participants at the current cached horizon and generation.
fn authorize(h: &Harness) {
    let (cap, generation) = h.inputs.grant_constraint();
    for slot in 0..h.rx.len() {
        h.inputs.authorize(slot, cap, generation);
    }
}
/// Drains wake and batch events and acknowledges consumed batch envelopes.
fn drain(h: &Harness) {
    for (slot, rx) in h.rx.iter().enumerate() {
        while let Ok(Some(event)) = rx.try_recv() {
            if matches!(event, AsyncEvent::PhysicalBatch(_)) {
                h.inputs.received(slot);
            }
        }
    }
}
/// Checks exclusive frontiers, zero-delay mapping, clock independence, and optional sources.
#[test]
fn exclusive_frontiers_zero_delay_independence_and_optional_idle_sources() {
    let h = harness(&[true, true, false], 2, 3, 8);
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
    h.inputs.advance(SourceKey(0), PhysicalInstant(0)).unwrap();
    assert_eq!(h.inputs.horizon(), Tag::NEVER);
    h.inputs.advance(SourceKey(1), PhysicalInstant(0)).unwrap();
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(2), usize::MAX)
    );
    h.inputs
        .advance(SourceKey(0), PhysicalInstant(100))
        .unwrap();
    h.inputs.advance(SourceKey(1), PhysicalInstant(10)).unwrap();
    assert_eq!(h.inputs.0.clock.now().unwrap(), PhysicalInstant(0));
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(12), usize::MAX)
    );
    drain(&h);
    h.inputs.advance(SourceKey(1), PhysicalInstant(10)).unwrap();
    assert!(h.rx[0].try_recv().unwrap().is_none());
    h.inputs.disconnect(SourceKey(2));
    assert_eq!(
        h.inputs.horizon(),
        Tag::new(Duration::nanoseconds(12), usize::MAX)
    );
    assert_eq!(
        h.inputs
            .advance(SourceKey(1), PhysicalInstant(9))
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
        h.inputs.disconnect(SourceKey(usize::MAX)).kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        h.inputs
            .advance(SourceKey(usize::MAX), PhysicalInstant(1))
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
    invalid.values = vec![InputValue::new(TargetKey(0), "wrong")];
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
    h.inputs.0.clock.advance_to(PhysicalInstant(20)).unwrap();
    h.inputs.advance(SourceKey(0), PhysicalInstant(10)).unwrap();
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
    h.inputs.0.clock.advance_to(PhysicalInstant(30)).unwrap();
    h.inputs.submit(vec![sample(&h, 0, 1, 10)], &[]).unwrap();
    assert!(!h.inputs.reserve(0, Tag::new(Duration::nanoseconds(10), 0)));
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
    assert!(h.inputs.reserve(0, Tag::new(Duration::nanoseconds(12), 0)));
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 3, 12)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Late
    );
    h.inputs.complete(0, Tag::new(Duration::nanoseconds(12), 0));
    h.inputs.submit(vec![sample(&h, 0, 3, 13)], &[]).unwrap();
    h.inputs.disconnect(SourceKey(0));
    drain(&h);
    authorize(&h);
    assert!(h.inputs.reserve(0, Tag::new(Duration::nanoseconds(13), 0)));
    assert_eq!(h.inputs.0.state.lock().unwrap().retained.len(), 1);
}
/// Checks whole-batch rejection before commit and execution abortion on partial publication.
#[test]
fn entire_batch_rejects_before_commit_and_partial_fanout_aborts() {
    let h = harness(&[false], 2, 0, 1);
    let mut malformed = sample(&h, 0, 1, 0);
    malformed.values.push(InputValue::new(TargetKey(1), false));
    assert_eq!(
        h.inputs.submit(vec![malformed], &[]).unwrap_err().kind,
        InputErrorKind::Malformed
    );
    assert!(h.rx.iter().all(|rx| rx.try_recv().unwrap().is_none()));
    h.inputs.0.state.lock().unwrap().participants[1]
        .tx
        .try_send(AsyncEvent::FederateResume)
        .unwrap();
    let mut batch = sample(&h, 0, 1, 0);
    batch.values.push(InputValue::new(TargetKey(1), 7u32));
    assert_eq!(
        h.inputs
            .submit(vec![batch], &[(SourceKey(0), PhysicalInstant(1))])
            .unwrap_err()
            .kind,
        InputErrorKind::Overflow
    );
    assert!(matches!(
        h.rx[0].try_recv().unwrap(),
        Some(AsyncEvent::PhysicalBatch(_))
    ));
    assert!(!h.inputs.reserve(0, Tag::ZERO));
    assert_eq!(h.inputs.grant_constraint().1, 1);
    assert!(h.inputs.0.state.lock().unwrap().sources[0]
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
        h.inputs.0.state.lock().unwrap().participants[0]
            .tx
            .try_send(AsyncEvent::FederateResume)
            .unwrap();
        if mode == 1 {
            h.rx[0].close().unwrap();
        }
        let mut first = sample(&h, 0, 1, 0);
        first.values[0].target = TargetKey(h.rx.len() - 1); // Last Enclave first.
        let generation = h.inputs.grant_constraint().1;
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
            assert!(state.sources[0].enabled && !state.sources[1].enabled);
        }
        assert!(
            state.retained.is_empty() && state.sources.iter().all(|source| source.last.is_none())
        );
        assert_eq!(state.generation, generation);
        drop(state);
        assert_eq!(h.inputs.reserve(0, Tag::ZERO), !required);
        h.inputs.disconnect(SourceKey(0));
        assert_eq!(h.inputs.0.state.lock().unwrap().failure.is_some(), required);
    }
    let h = harness(&[true], 2, 1, 8);
    h.inputs
        .0
        .clock
        .advance_to(PhysicalInstant(u64::MAX))
        .unwrap();
    assert_eq!(
        h.inputs
            .submit(vec![sample(&h, 0, 1, u64::MAX)], &[])
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    assert_eq!(
        h.inputs
            .advance(SourceKey(0), PhysicalInstant(u64::MAX))
            .unwrap_err()
            .kind,
        InputErrorKind::Malformed
    );
    assert!(h.inputs.0.state.lock().unwrap().sources[0].last.is_none());
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
            let reserved = h.inputs.reserve(0, Tag::ZERO);
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
            h.inputs.0.clock.advance_to(PhysicalInstant(4)).unwrap();
            for start in [0, 2] {
                let batch = (start..start + 2)
                    .map(|time| {
                        let mut observation = sample(&h, 0, time, time);
                        observation.values = (0..enclaves)
                            .map(|slot| InputValue::new(TargetKey(slot), 1u32))
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
                    .advance(SourceKey(source), PhysicalInstant(5))
                    .unwrap();
            }
            drain(&h);
            authorize(&h);
            let start = Instant::now();
            ALLOCATIONS.with(|n| n.set(Some(0)));
            for _ in 0..100_000 {
                std::hint::black_box(h.inputs.horizon());
                assert!(h.inputs.reserve(0, Tag::ZERO));
            }
            let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
            assert_eq!(allocations, 0);
            eprintln!("input measurement: enclaves={enclaves} sources={count} max_values={} retained=2 checks=100000 allocations={allocations} elapsed={:?}", enclaves * 2, start.elapsed());
            for slot in 0..enclaves {
                h.inputs
                    .complete(slot, Tag::new(Duration::nanoseconds(3), 0));
            }
            assert!(h.inputs.0.state.lock().unwrap().retained.is_empty());
        }
    }
}

/// Marks fixture participants active and publishes the next tag.
fn republish(h: &mut Harness, tag: Tag) {
    let state = &mut h._coordination.coordinator.state;
    for slot in 0..h.rx.len() {
        let enclave = EnclaveIndex::new(slot as u32);
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
    h.inputs.0.clock.advance_to(PhysicalInstant(10)).unwrap();
    h._coordination.coordinator.state.physical_inputs = Some(h.inputs.clone());
    let tag = Tag::new(Duration::nanoseconds(10), 0);
    republish(&mut h, tag);
    let revision = h._coordination.coordinator.state.revision();
    h._coordination
        .coordinator
        .state
        .handle_acquisition(FederateAcquisition::new(revision, tag))
        .unwrap();
    h.inputs.submit(vec![sample(&h, 0, 1, 10)], &[]).unwrap();
    drain(&h);
    // A backend extension computed for the old publication must not bless the new generation.
    h._coordination
        .coordinator
        .state
        .handle_acquisition(FederateAcquisition::new(revision, Tag::FOREVER))
        .unwrap();
    h._coordination.coordinator.state.input_progress().unwrap();
    assert!(!h.inputs.reserve(0, tag), "a queued grant from before batch admission must be stale even after its envelope was consumed");
    republish(&mut h, tag);
    let revision = h._coordination.coordinator.state.revision();
    h._coordination
        .coordinator
        .state
        .handle_acquisition(FederateAcquisition::new(revision, tag))
        .unwrap();
    assert!(h.inputs.reserve(0, tag));
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
    h._coordination.coordinator.state.physical_inputs = Some(inputs.clone());
    republish(&mut h, tag);
    let state = &mut h._coordination.coordinator.state;
    // The backend's acquisition has been consumed and capped before the workers wait.
    state
        .handle_acquisition(FederateAcquisition::new(state.revision(), tag))
        .unwrap();
    assert_eq!(state.grant_horizon(), Some(Tag::NEVER));
    for frontier in [1, 2] {
        inputs
            .advance(SourceKey(0), PhysicalInstant(frontier))
            .unwrap();
        state.input_progress().unwrap();
        assert_eq!(state.grant_horizon(), Some(inputs.horizon()));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let parts = h._coordination;
    let results = std::thread::scope(|scope| {
        scope.spawn(move || parts.coordinator.run());
        for (_, mut port) in parts.participants {
            let tx = tx.clone();
            scope.spawn(move || tx.send(port.acquire_tag(tag)).unwrap());
        }
        inputs.advance(SourceKey(0), PhysicalInstant(3)).unwrap();
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
