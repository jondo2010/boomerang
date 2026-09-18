//! Fault-driven checks at the pure RTI request/reply boundary, not a simulated network stack.
use super::*;
use boomerang_federated::conformance::{
    Failure, FaultEffect, FaultScript, Lane, OrderedFaultScheduler, ScheduledOutcome,
};

const SOURCE: Member = Member::new(0);
const SINK: Member = Member::new(1);
const ROUTE: Route = Route::new(0);
const TAG: WireTag = WireTag::finite(1_000_000, 0);

/// A participant owns one channel, not separate NET/LTC/payload lanes.
fn lane(step: VectorStep) -> Lane {
    let (member, _) = ReferenceVectorAdapter::request(step);
    if member == MEMBERS[0] {
        Lane::new(0)
    } else if member == MEMBERS[1] {
        Lane::new(1)
    } else {
        panic!("unmapped participant")
    }
}

/// Both implementations consume the exact same delivered steps. Compare after *each* request
/// so an early grant, reordered payload, swallowed failure, or duplicate reply cannot disappear
/// inside an end-of-vector comparison. Administrative DNET/idle replies remain visible separately.
struct FaultRun {
    scheduler: OrderedFaultScheduler,
    oracle: ReferenceCoordinator,
    rti: CompiledRti<'static>,
    execution: VectorExecution,
    delivered: Vec<(Lane, VectorStep)>,
    failure: Option<Failure>,
    failure_message: Option<String>,
}

impl FaultRun {
    fn new(effects: impl IntoIterator<Item = FaultEffect>) -> Self {
        let exchange = tagged_payload_exchange();
        Self {
            scheduler: OrderedFaultScheduler::new(
                [Lane::new(0), Lane::new(1)],
                FaultScript::new(effects).unwrap(),
            )
            .unwrap(),
            // Independent literal matching the compiled fixture's destination budget.
            oracle: ReferenceCoordinator::new(exchange.members, exchange.topology, 32).unwrap(),
            rti: admitted_rti(),
            execution: VectorExecution::default(),
            delivered: Vec::new(),
            failure: None,
            failure_message: None,
        }
    }

    fn submit(&mut self, tick: u64, step: VectorStep) {
        let outcomes = self.scheduler.submit(tick, lane(step), step);
        self.deliver(outcomes);
    }

    fn advance(&mut self, tick: u64) {
        let outcomes = self.scheduler.advance(tick);
        self.deliver(outcomes);
    }

    fn deliver(&mut self, outcomes: Vec<ScheduledOutcome>) {
        for outcome in outcomes {
            let step = match outcome {
                ScheduledOutcome::Delivered { lane: actual, step } => {
                    assert_eq!(actual, lane(step));
                    self.delivered.push((actual, step));
                    step
                }
                // Failure is session-wide. SOURCE is the observer used to report it, not a
                // claim about which peer caused it. No failed frame becomes an application step.
                ScheduledOutcome::Failed { failure } => VectorStep::Fail {
                    member: SOURCE,
                    failure,
                },
            };
            self.apply(step);
        }
    }

    fn apply(&mut self, step: VectorStep) {
        let expected = self.oracle.apply(step);
        let (member, request) = ReferenceVectorAdapter::request(step);
        let replies = self.rti.handle(member, request);
        let observations = replies
            .iter()
            .cloned()
            .map(ReferenceVectorAdapter::observation)
            .collect::<Vec<_>>();
        if let [Outcome::Failed { failure, .. }] = expected.as_slice() {
            let first = self.failure.is_none();
            assert_eq!(*self.failure.get_or_insert(*failure), *failure);
            let expected_members = if first {
                vec![SOURCE, SINK]
            } else {
                vec![ReferenceVectorAdapter::vector_member(member)]
            };
            let mut failed_members = Vec::new();
            for observation in &observations {
                let VectorObservation::Failed { member, message } = observation else {
                    panic!("nonterminal reply after {failure:?}: {observation:?}");
                };
                assert_eq!(
                    self.failure_message.get_or_insert_with(|| message.clone()),
                    message
                );
                failed_members.push(*member);
            }
            assert_eq!(
                failed_members, expected_members,
                "failure must release all peers once"
            );
            assert!(self.rti.is_finished());
        } else {
            assert!(
                !observations
                    .iter()
                    .any(|o| matches!(o, VectorObservation::Failed { .. })),
                "unexpected failure: {observations:?}"
            );
            let actual = observations
                .iter()
                .filter_map(|o| match o {
                    VectorObservation::Semantic(outcome) => Some(*outcome),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "outcomes for {step:?}");
        }
        self.execution.record(replies);
    }

    fn idle_members(&self) -> Vec<Member> {
        self.execution
            .observations
            .iter()
            .filter_map(|o| match o {
                VectorObservation::Idle { member, .. } => Some(*member),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn delayed_ltc_holds_following_net_but_not_peer_revisions() {
    if !client::run_trace_test() {
        return;
    }
    let (_, events) = client::capture_coordination(|| {
        let mut run = FaultRun::new([FaultEffect::duplicate(2, 2), FaultEffect::delay(3, 3, 5)]);
        let steps = tagged_payload_exchange().steps;
        for (tick, step) in (0..).zip(&steps[..5]) {
            run.submit(tick, *step);
        }
        // B's revised NET crosses the delayed A lane; A's later idle NET must not.
        let revised = VectorStep::publish(SINK, 1, Some(TAG));
        run.submit(5, revised);
        run.advance(7);
        assert_eq!(
            run.delivered,
            [
                (Lane::new(0), steps[0]),
                (Lane::new(1), steps[1]),
                (Lane::new(0), steps[2]),
                (Lane::new(1), revised),
            ]
        );
        assert_eq!(
            run.execution.semantic_outcomes,
            [
                Outcome::grant(SOURCE, 0, WireTag::FOREVER),
                Outcome::payload(SINK, ROUTE, TAG),
            ]
        );
        run.advance(8);
        assert_eq!(
            &run.delivered[4..],
            &[(Lane::new(0), steps[3]), (Lane::new(0), steps[4])]
        );
        assert_eq!(
            run.execution.semantic_outcomes.last(),
            Some(&Outcome::grant(SINK, 1, WireTag::FOREVER))
        );
        // The future input has been forwarded but is still accounted until B executes it.
        run.submit(
            9,
            VectorStep::ConfirmIdle {
                member: SOURCE,
                revision: 1,
            },
        );
        assert!(run.idle_members().is_empty());
        run.submit(10, VectorStep::complete(SINK, TAG));
        run.submit(11, VectorStep::publish(SINK, 2, None));
        run.submit(
            12,
            VectorStep::ConfirmIdle {
                member: SINK,
                revision: 2,
            },
        );
        assert_eq!(run.idle_members(), [SOURCE, SINK]);
        run.submit(13, VectorStep::Stop { member: SOURCE });
        run.submit(14, VectorStep::Stop { member: SINK });
        assert!(run.rti.is_finished());
        assert_eq!(
            run.execution.semantic_outcomes,
            [
                Outcome::grant(SOURCE, 0, WireTag::FOREVER),
                Outcome::payload(SINK, ROUTE, TAG),
                Outcome::grant(SINK, 1, WireTag::FOREVER),
                Outcome::Stopped { member: SOURCE },
                Outcome::Stopped { member: SINK },
            ]
        );
    });
    // Real formatted subscriber bytes, not an intermediate event visitor. The peer revision
    // produces no delivery while A's LTC/NET is delayed; the grant follows A's completion.
    let decisions = events
        .iter()
        .filter(|e| e["fields"]["event"] == "coordination.rti.decision")
        .filter(|e| e["fields"]["request"] == "publication")
        .collect::<Vec<_>>();
    assert_eq!(decisions[2]["fields"]["deliveries"], 0);
    let completed = events
        .iter()
        .position(|e| e["fields"]["event"] == "coordination.rti.accounting.completed")
        .unwrap();
    let sink_grant = events
        .iter()
        .position(|e| {
            e["fields"]["event"] == "coordination.rti.grant.issued" && e["fields"]["revision"] == 1
        })
        .unwrap();
    assert!(completed < sink_grant);
    let forwarded = events
        .iter()
        .filter(|e| e["fields"]["event"] == "coordination.rti.payload.forwarded")
        .collect::<Vec<_>>();
    assert_eq!(
        forwarded.len(),
        1,
        "lower-layer duplication must not duplicate application delivery"
    );
    assert_eq!(forwarded[0]["fields"]["pending_tags"], 1);
    assert!(events
        .iter()
        .filter(|e| e["fields"]["event"] == "coordination.rti.accounting.completed")
        .all(|e| e["fields"]["pending_tags"] == 0));
}

#[test]
fn delayed_source_publication_allows_destination_first_without_early_grant() {
    let mut run = FaultRun::new([FaultEffect::delay(0, 0, 5)]);
    let steps = tagged_payload_exchange().steps;
    for (tick, step) in (0..).zip(&steps[..5]) {
        run.submit(tick, *step);
    }
    assert_eq!(run.delivered, [(Lane::new(1), steps[1])]);
    assert!(run.execution.semantic_outcomes.is_empty());
    run.advance(5);
    run.submit(6, steps[5]);
    run.submit(7, steps[6]);
    assert_eq!(
        run.execution.semantic_outcomes,
        tagged_payload_exchange().expected_outcomes
    );
}

#[test]
fn transport_failure_discards_delayed_frames_and_traces_only_the_first_cause() {
    if !client::run_trace_test() {
        return;
    }
    for (effect, expected) in [
        (FaultEffect::drop(2, 2), Failure::Lost),
        (FaultEffect::corrupt(2, 2), Failure::Corrupted),
        (
            FaultEffect::fail(2, 2, Failure::InTransitCapacity),
            Failure::InTransitCapacity,
        ),
    ] {
        let (_, events) = client::capture_coordination(|| {
            let mut run = FaultRun::new([FaultEffect::delay(0, 0, 5), effect]);
            let steps = tagged_payload_exchange().steps;
            for (tick, step) in (0..).zip(&steps[..3]) {
                run.submit(tick, *step);
            }
            assert_eq!(run.failure, Some(expected));
            assert_eq!(
                run.failure_message,
                Some(format!("conformance transport: {expected:?}"))
            );
            run.advance(10);
            run.submit(11, steps[3]);
            // Also bypass the failed scheduler to prove that the RTI itself is terminal.
            run.apply(VectorStep::Fail {
                member: SINK,
                failure: Failure::Publication,
            });
            run.apply(VectorStep::publish(SOURCE, 2, Some(TAG)));
            assert_eq!(run.delivered, [(Lane::new(1), steps[1])]);
            assert!(run.execution.semantic_outcomes.is_empty());
        });
        assert_eq!(
            events
                .iter()
                .filter(|e| e["fields"]["event"] == "coordination.rti.failure.first")
                .count(),
            1
        );
        assert!(!events.iter().any(|e| matches!(
            e["fields"]["event"].as_str(),
            Some(
                "coordination.rti.grant.issued"
                    | "coordination.rti.payload.forwarded"
                    | "coordination.rti.idle.issued"
            )
        )));
    }
}

#[test]
fn future_net_does_not_retire_input_or_hide_bounded_saturation() {
    let mut run = FaultRun::new([]);
    run.submit(0, VectorStep::publish(SOURCE, 0, Some(WireTag::ZERO)));
    for (tick, nanos) in (1..).zip(1_000_000..1_000_032) {
        run.submit(
            tick,
            VectorStep::payload(SOURCE, ROUTE, WireTag::finite(nanos, 0)),
        );
    }
    // Local future-queue admission (NET) is not execution (LTC), so it frees no slot.
    run.submit(
        33,
        VectorStep::publish(SINK, 0, Some(WireTag::finite(2_000_000, 0))),
    );
    assert_eq!(run.execution.semantic_outcomes.len(), 33); // source grant + 32 payloads
    run.submit(
        34,
        VectorStep::payload(SOURCE, ROUTE, WireTag::finite(1_000_032, 0)),
    );
    assert_eq!(run.failure, Some(Failure::InTransitCapacity));
    assert!(run.failure_message.as_ref().unwrap().contains("in-transit"));
    run.apply(VectorStep::Fail {
        member: SINK,
        failure: Failure::Corrupted,
    });
    run.submit(35, VectorStep::complete(SINK, TAG));
    assert_eq!(run.execution.semantic_outcomes.len(), 33);
}

#[test]
fn only_executed_input_frees_capacity_for_a_later_distinct_tag() {
    let mut run = FaultRun::new([FaultEffect::delay(34, 34, 3)]);
    run.submit(0, VectorStep::publish(SOURCE, 0, Some(WireTag::ZERO)));
    for (tick, nanos) in (1..).zip(1_000_000..1_000_032) {
        run.submit(
            tick,
            VectorStep::payload(SOURCE, ROUTE, WireTag::finite(nanos, 0)),
        );
    }
    run.submit(33, VectorStep::publish(SOURCE, 1, None));
    // The delayed NET holds B's following LTC on the same lane.
    run.submit(34, VectorStep::publish(SINK, 0, Some(TAG)));
    run.submit(35, VectorStep::complete(SINK, TAG));
    assert_eq!(run.execution.semantic_outcomes.len(), 33);
    run.advance(37);
    assert_eq!(
        run.execution.semantic_outcomes.last(),
        Some(&Outcome::grant(SINK, 0, WireTag::FOREVER))
    );
    run.submit(
        38,
        VectorStep::payload(SOURCE, ROUTE, WireTag::finite(1_000_032, 0)),
    );
    assert!(run.failure.is_none());
    assert_eq!(
        run.execution.semantic_outcomes.last(),
        Some(&Outcome::payload(
            SINK,
            ROUTE,
            WireTag::finite(1_000_032, 0)
        ))
    );
    // Completing the first tag left 31 later tags pending. The replacement refilled the
    // budget; a second distinct tag must fail (clearing the whole queue would hide this).
    run.submit(
        39,
        VectorStep::payload(SOURCE, ROUTE, WireTag::finite(1_000_033, 0)),
    );
    assert_eq!(run.failure, Some(Failure::InTransitCapacity));
}

#[test]
fn confirmed_idle_waits_for_delayed_input_completion_before_stop() {
    let mut run = FaultRun::new([FaultEffect::delay(8, 8, 3)]);
    for (tick, step) in (0..).zip(&tagged_payload_exchange().steps[..5]) {
        run.submit(tick, *step);
    }
    run.submit(5, VectorStep::publish(SINK, 1, None));
    run.submit(
        6,
        VectorStep::ConfirmIdle {
            member: SOURCE,
            revision: 1,
        },
    );
    run.submit(
        7,
        VectorStep::ConfirmIdle {
            member: SINK,
            revision: 1,
        },
    );
    assert!(
        run.idle_members().is_empty(),
        "both peers confirmed but the input is still pending"
    );
    run.submit(8, VectorStep::complete(SINK, TAG));
    run.submit(9, VectorStep::Stop { member: SINK });
    run.advance(10);
    assert!(run.idle_members().is_empty());
    assert!(!run.rti.is_finished());
    run.advance(11);
    assert_eq!(run.idle_members(), [SOURCE, SINK]);
    assert_eq!(
        run.execution.semantic_outcomes.last(),
        Some(&Outcome::Stopped { member: SINK })
    );
    run.submit(12, VectorStep::Stop { member: SOURCE });
    assert!(run.rti.is_finished());
}

#[test]
fn premature_stop_is_a_protocol_failure_not_an_adapter_panic() {
    let execution = run_reference_vector([VectorStep::Stop {
        member: Member::new(0),
    }]);
    assert!(execution.semantic_outcomes.is_empty());
    assert!(matches!(execution.observations.as_slice(), [
        VectorObservation::Failed { member: a, message: first },
        VectorObservation::Failed { member: b, message: second },
    ] if *a == Member::new(0) && *b == Member::new(1)
        && first == second && first.contains("idle authority")));
}

#[test]
fn idle_handshake_and_stop_are_explicit_vector_steps() {
    let [a, b] = [Member::new(0), Member::new(1)];
    let execution = run_reference_vector([
        VectorStep::publish(a, 0, None),
        VectorStep::publish(b, 0, None),
        VectorStep::ConfirmIdle {
            member: a,
            revision: 0,
        },
        VectorStep::ConfirmIdle {
            member: b,
            revision: 0,
        },
        VectorStep::Stop { member: a },
        VectorStep::Stop { member: b },
    ]);
    assert_eq!(
        execution.observations,
        [
            VectorObservation::Idle {
                member: a,
                revision: 0
            },
            VectorObservation::Idle {
                member: b,
                revision: 0
            },
            VectorObservation::Semantic(Outcome::Stopped { member: a }),
            VectorObservation::Semantic(Outcome::Stopped { member: b }),
        ]
    );
}

#[test]
fn injected_abort_releases_both_peers_and_retains_first_cause() {
    let [a, b] = [Member::new(0), Member::new(1)];
    let execution = run_reference_vector([
        VectorStep::Fail {
            member: a,
            failure: Failure::Lost,
        },
        VectorStep::Fail {
            member: b,
            failure: Failure::Corrupted,
        },
    ]);
    assert_eq!(
        execution.observations,
        [a, b, b].map(|member| VectorObservation::Failed {
            member,
            message: "conformance transport: Lost".into(),
        })
    );
}
