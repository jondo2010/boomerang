use super::*;

#[test]
fn frontier_demands_suppress_duplicates_preserve_partial_progress_and_reset_with_the_proof() {
    let (tx, rx) = kanal::bounded(2);
    let root = ProofRoot::new(
        EnclaveKey::from(1),
        ExecutionEpoch(7),
        1,
        PhysicalInstant(9),
        tx.clone(),
        2,
    );
    let mut outbox = ProofOutbox::default();
    outbox.prepare(&root.lease);
    let mut participant =
        Participation::new(root.lease.clone(), tx.clone(), 1, PhysicalInstant(9), 2);
    let upstream = EnclaveKey::from(1_000_000);
    let demand = PhysicalInstant(4);
    let receive = |round, sequence, expected_cut| {
        let Some(AsyncEvent::OutputProof(frame)) = rx.try_recv().unwrap() else {
            panic!("missing frontier demand");
        };
        assert_eq!(frame.round, round);
        assert_eq!(frame.lease.sequence, sequence);
        assert!(
            matches!(frame.kind, ProofKind::NeedFrontier { key, cut } if key == upstream && cut == expected_cut)
        );
        assert!(
            rx.try_recv().unwrap().is_none(),
            "duplicate frontier demand"
        );
    };
    participant.request_frontier(upstream, demand, &mut outbox);
    participant.request_frontier(upstream, demand, &mut outbox);
    receive(1, 1, demand);
    // A smaller predecessor certificate is distinct from the outstanding goal.
    participant.request_frontier(upstream, PhysicalInstant(3), &mut outbox);
    receive(1, 1, PhysicalInstant(3));
    participant.request_frontier(upstream, demand, &mut outbox);
    receive(1, 1, demand);

    // Neither partial time progress nor the last microstep proves closure at 4.
    for progress in [
        ProgressBound::ThroughTime(PhysicalInstant(3)),
        ProgressBound::ThroughTag(crate::Tag::new(crate::Duration::nanoseconds(4), usize::MAX)),
    ] {
        participant.observe_progress(upstream, progress);
        participant.request_frontier(upstream, demand, &mut outbox);
        assert!(rx.try_recv().unwrap().is_none());
    }
    participant.observe_progress(upstream, ProgressBound::ThroughTime(demand));
    assert!(participant.frontier_demands.is_empty());
    participant.request_frontier(upstream, demand, &mut outbox);
    receive(1, 1, demand);
    participant.observe_progress(
        upstream,
        ProgressBound::ThroughTag(crate::Tag::new(crate::Duration::nanoseconds(5), 0)),
    );
    assert!(participant.frontier_demands.is_empty());

    participant.request_frontier(upstream, demand, &mut outbox);
    receive(1, 1, demand);
    participant.park(2);
    participant.request_frontier(upstream, demand, &mut outbox);
    receive(2, 1, demand);
    assert_eq!(participant.frontier_demands.capacity(), 2);

    let next = ProofRoot::new(
        EnclaveKey::from(1),
        ExecutionEpoch(7),
        2,
        PhysicalInstant(9),
        tx.clone(),
        2,
    );
    let mut participant = Participation::new(next.lease.clone(), tx, 1, PhysicalInstant(9), 1);
    participant.request_frontier(upstream, demand, &mut outbox);
    receive(1, 2, demand);
    participant.request_frontier(EnclaveKey::from(9), demand, &mut outbox);
    assert_eq!(next.lease.error.get(), Some(&OutputWaitError::Capacity));
    assert_eq!(participant.frontier_demands.len(), 1);
    assert_eq!(participant.frontier_demands.capacity(), 1);
}

#[test]
fn sparse_members_do_not_expand_the_fixed_evidence_allocation() {
    let (tx, _rx) = kanal::bounded(2);
    let mut root = ProofRoot::new(
        EnclaveKey::from(1),
        ExecutionEpoch(7),
        1,
        PhysicalInstant(0),
        tx.clone(),
        2,
    );
    root.include(
        EnclaveKey::from(1_000_000),
        PhysicalInstant(0),
        Some(tx.clone()),
    );
    assert_eq!(root.members.len(), 2);
    assert_eq!(root.members.capacity(), 2);
    root.include(EnclaveKey::from(9), PhysicalInstant(0), Some(tx));
    assert_eq!(root.lease.error.get(), Some(&OutputWaitError::Capacity));
    assert_eq!(root.members.len(), 2);
    assert_eq!(root.members.capacity(), 2);
}

#[test]
fn full_control_mailbox_retains_order_without_unbounded_allocation() {
    let (tx, rx) = kanal::bounded(1);
    let root = ProofRoot::new(
        EnclaveKey::from(1),
        ExecutionEpoch(7),
        1,
        PhysicalInstant(0),
        tx.clone(),
        1,
    );
    let mut outbox = ProofOutbox::default();
    outbox.prepare(&root.lease);
    tx.try_send(AsyncEvent::OutputWake).unwrap();
    for round in 1..=4 {
        outbox.send(&tx, &root.lease, round, ProofKind::Park);
    }
    assert_eq!(outbox.pending.len(), 4);
    assert!(root.lease.error.get().is_none());
    rx.try_recv().unwrap().unwrap();
    for expected in 1..=4 {
        outbox.flush();
        let AsyncEvent::OutputProof(frame) = rx.try_recv().unwrap().unwrap() else {
            panic!("lost proof frame")
        };
        assert_eq!(frame.round, expected);
    }
    assert!(!outbox.pending());
    assert_eq!(outbox.pending.capacity(), 4);
    tx.try_send(AsyncEvent::OutputWake).unwrap();
    for round in 1..=5 {
        outbox.send(&tx, &root.lease, round, ProofKind::Park);
    }
    assert_eq!(root.lease.error.get(), Some(&OutputWaitError::Capacity));
    assert_eq!(outbox.pending.len(), 4);
    outbox.flush();
    assert!(
        !outbox.pending(),
        "failed proof frames survived lease cancellation"
    );
}
