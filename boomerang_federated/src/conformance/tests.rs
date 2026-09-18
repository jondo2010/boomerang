use super::{
    ConstructionError, Failure, FaultEffect, FaultScript, FaultScriptError, Lane, Member,
    OrderedFaultScheduler, Outcome, ReferenceCoordinator, Route, RouteTopology, ScheduledOutcome,
    Topology, VectorStep,
};
use crate::WireTag;

#[test]
fn stop_requires_global_idle_authority() {
    let exchange = super::tagged_payload_exchange();
    let mut oracle = ReferenceCoordinator::new(exchange.members, exchange.topology, 1).unwrap();
    assert!(matches!(
        oracle
            .apply(VectorStep::Stop {
                member: exchange.source
            })
            .as_slice(),
        [Outcome::Failed { .. }]
    ));
}

#[test]
fn idle_confirmation_tracks_revisions_and_pending_input() {
    let exchange = super::tagged_payload_exchange();
    let [source, destination] = exchange.members;
    for (pending, revise, expected) in [
        (false, false, Outcome::Stopped { member: source }),
        (
            true,
            false,
            Outcome::Failed {
                member: source,
                failure: Failure::IdleAuthority,
            },
        ),
        (
            false,
            true,
            Outcome::Failed {
                member: source,
                failure: Failure::IdleAuthority,
            },
        ),
    ] {
        let mut oracle =
            ReferenceCoordinator::new(exchange.members, exchange.topology.clone(), 1).unwrap();
        if pending {
            oracle.apply(VectorStep::payload(source, exchange.route, exchange.tag));
        }
        for member in [source, destination] {
            oracle.apply(VectorStep::publish(member, 0, None));
        }
        oracle.apply(VectorStep::ConfirmIdle {
            member: source,
            revision: 0,
        });
        if revise {
            oracle.apply(VectorStep::publish(source, 1, None));
        }
        oracle.apply(VectorStep::ConfirmIdle {
            member: destination,
            revision: 0,
        });
        assert_eq!(
            oracle.apply(VectorStep::Stop { member: source }),
            [expected]
        );
    }
}

#[test]
fn stale_idle_confirmation_fails_instead_of_authorizing_stop() {
    let member = Member::new(0);
    let mut oracle = ReferenceCoordinator::new([member], Topology::new([]).unwrap(), 1).unwrap();
    oracle.apply(VectorStep::publish(member, 1, None));
    assert_eq!(
        oracle.apply(VectorStep::ConfirmIdle {
            member,
            revision: 0
        }),
        [Outcome::Failed {
            member,
            failure: Failure::IdleConfirmation
        }]
    );
}

#[test]
fn injected_failure_wins_over_later_semantic_and_transport_failures() {
    let member = Member::new(0);
    let mut oracle = ReferenceCoordinator::new([member], Topology::new([]).unwrap(), 1).unwrap();
    for step in [
        VectorStep::Fail {
            member,
            failure: Failure::Lost,
        },
        VectorStep::complete(member, WireTag::NEVER),
        VectorStep::Fail {
            member,
            failure: Failure::Corrupted,
        },
        VectorStep::publish(member, 0, Some(WireTag::ZERO)),
    ] {
        assert_eq!(
            oracle.apply(step),
            [Outcome::Failed {
                member,
                failure: Failure::Lost
            }]
        );
    }
}

#[test]
fn tagged_payload_exchange_is_a_reusable_oracle_vector() {
    let exchange = super::tagged_payload_exchange();
    let mut oracle = ReferenceCoordinator::new(exchange.members, exchange.topology, 1).unwrap();
    let outcomes = exchange
        .steps
        .into_iter()
        .flat_map(|step| oracle.apply(step))
        .collect::<Vec<_>>();
    assert_eq!(outcomes, exchange.expected_outcomes);
}

#[test]
fn delivered_input_does_not_block_its_own_execution_grant() {
    let exchange = super::tagged_payload_exchange();
    let mut oracle = ReferenceCoordinator::new(exchange.members, exchange.topology, 1).unwrap();
    for step in &exchange.steps[..4] {
        oracle.apply(*step);
    }
    assert_eq!(
        oracle.apply(exchange.steps[4]),
        [Outcome::grant(exchange.destination, 0, WireTag::FOREVER)]
    );
    assert!(oracle.apply(exchange.steps[5]).is_empty());
}

#[test]
fn direct_route_delay_and_completion_bound_the_grant() {
    let [source, destination] = [Member::new(0), Member::new(1)];
    for (delay, expected) in [
        (0, WireTag::finite(5, 0)),
        (10, WireTag::finite(14, u64::MAX)),
    ] {
        let mut oracle = ReferenceCoordinator::new(
            [source, destination],
            Topology::new([(
                Route::new(0),
                RouteTopology::with_delay(source, destination, delay),
            )])
            .unwrap(),
            1,
        )
        .unwrap();
        oracle.apply(VectorStep::publish(source, 0, Some(WireTag::ZERO)));
        oracle.apply(VectorStep::complete(source, WireTag::finite(5, 0)));
        assert_eq!(
            oracle.apply(VectorStep::publish(
                destination,
                0,
                Some(WireTag::finite(5, 0))
            )),
            [Outcome::grant(destination, 0, expected)]
        );
    }
}

#[test]
fn destination_waits_for_upstream_publication_before_idle_releases_it() {
    let source = Member::new(0);
    let destination = Member::new(1);
    let route = Route::new(0);
    let requested = WireTag::finite(0, 10);
    let mut oracle = ReferenceCoordinator::new(
        [source, destination],
        Topology::new([(route, RouteTopology::new(source, destination))]).unwrap(),
        1,
    )
    .unwrap();

    assert!(oracle
        .apply(VectorStep::publish(destination, 1, Some(requested)))
        .is_empty());
    assert_eq!(
        oracle.apply(VectorStep::publish(source, 1, None)),
        vec![Outcome::grant(destination, 1, WireTag::FOREVER)]
    );
}

#[test]
fn completion_releases_only_the_accounted_frontier() {
    let source = Member::new(0);
    let middle = Member::new(1);
    let destination = Member::new(2);
    let route = Route::new(0);
    let earlier = WireTag::finite(0, 5);
    let later = WireTag::finite(0, 10);
    let mut oracle = ReferenceCoordinator::new(
        [source, middle, destination],
        Topology::new([
            (route, RouteTopology::new(source, middle)),
            (Route::new(1), RouteTopology::new(middle, destination)),
        ])
        .unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        oracle.apply(VectorStep::publish(source, 1, Some(earlier))),
        vec![Outcome::grant(source, 1, WireTag::FOREVER)]
    );
    assert_eq!(
        oracle.apply(VectorStep::payload(source, route, earlier)),
        vec![Outcome::payload(middle, route, earlier)]
    );
    assert!(oracle
        .apply(VectorStep::publish(source, 2, None))
        .is_empty());

    // B can execute its input, but that queued input still constrains C until B's LTC.
    assert_eq!(
        oracle.apply(VectorStep::publish(middle, 1, Some(later))),
        vec![Outcome::grant(middle, 1, WireTag::FOREVER)]
    );
    assert!(oracle
        .apply(VectorStep::publish(
            destination,
            1,
            Some(WireTag::finite(0, 9))
        ))
        .is_empty());
    assert!(oracle
        .apply(VectorStep::complete(middle, WireTag::finite(0, 4)))
        .is_empty());

    assert_eq!(
        oracle.apply(VectorStep::complete(middle, earlier)),
        vec![Outcome::grant(destination, 1, WireTag::finite(0, 9))]
    );
}

#[test]
fn construction_rejects_route_endpoints_outside_the_member_table() {
    let member = Member::new(0);
    let missing = Member::new(1);
    let route = Route::new(0);
    let topology = Topology::new([(route, RouteTopology::new(member, missing))]).unwrap();

    assert!(matches!(
        ReferenceCoordinator::new([member], topology, 1),
        Err(ConstructionError::RouteEndpoint {
            route: actual_route,
            member: actual_member,
        })
        if actual_route == route && actual_member == missing
    ));
}

#[test]
fn terminal_failure_retains_the_first_distinct_invalid_step() {
    let member = Member::new(0);
    let mut oracle = ReferenceCoordinator::new(
        [member],
        Topology::new([] as [(Route, RouteTopology); 0]).unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        oracle.apply(VectorStep::payload(member, Route::new(0), WireTag::ZERO)),
        vec![Outcome::Failed {
            member,
            failure: Failure::UnknownRoute,
        }]
    );
    assert_eq!(
        oracle.apply(VectorStep::complete(member, WireTag::NEVER)),
        vec![Outcome::Failed {
            member,
            failure: Failure::UnknownRoute,
        }]
    );
}

#[test]
fn delayed_first_frame_prevents_later_frame_overtaking() {
    let lane = Lane::new(0);
    let source = Member::new(0);
    let route = Route::new(0);
    let first = VectorStep::payload(source, route, WireTag::ZERO);
    let later = VectorStep::payload(source, route, WireTag::finite(0, 1));
    let script = FaultScript::new([FaultEffect::delay(0, 0, 2)]).unwrap();
    let mut scheduler = OrderedFaultScheduler::new([lane], script).unwrap();

    assert!(scheduler.submit(0, lane, first).is_empty());
    assert!(scheduler.submit(0, lane, later).is_empty());
    assert!(scheduler.advance(1).is_empty());
    assert_eq!(
        scheduler.advance(2),
        vec![
            ScheduledOutcome::delivered(lane, first),
            ScheduledOutcome::delivered(lane, later),
        ]
    );
}

#[test]
fn corruption_retains_exactly_one_terminal_failure() {
    let lane = Lane::new(0);
    let source = Member::new(0);
    let route = Route::new(0);
    let first = VectorStep::payload(source, route, WireTag::ZERO);
    let later = VectorStep::payload(source, route, WireTag::finite(0, 1));
    let script = FaultScript::new([FaultEffect::corrupt(0, 0)]).unwrap();
    let mut scheduler = OrderedFaultScheduler::new([lane], script).unwrap();

    assert_eq!(
        scheduler.submit(0, lane, first),
        vec![ScheduledOutcome::failed(Failure::Corrupted)]
    );
    assert_eq!(
        scheduler.submit(1, lane, later),
        vec![ScheduledOutcome::failed(Failure::Corrupted)]
    );
}

#[test]
fn zero_delay_fault_is_rejected_as_excluded_configuration() {
    assert_eq!(
        FaultScript::new([FaultEffect::delay(0, 0, 0)]),
        Err(FaultScriptError::ZeroDelay {
            ordinal: 0,
            tick: 0,
        })
    );
}

#[test]
fn fault_actions_do_not_duplicate_delivery_and_retain_declared_failures() {
    let lane = Lane::new(0);
    let source = Member::new(0);
    let step = VectorStep::complete(source, WireTag::ZERO);

    for (effect, expected) in [
        (
            FaultEffect::drop(0, 0),
            ScheduledOutcome::failed(Failure::Lost),
        ),
        (
            FaultEffect::duplicate(0, 0),
            ScheduledOutcome::delivered(lane, step),
        ),
        (
            FaultEffect::fail(0, 0, Failure::Publication),
            ScheduledOutcome::failed(Failure::Publication),
        ),
    ] {
        let mut scheduler =
            OrderedFaultScheduler::new([lane], FaultScript::new([effect]).unwrap()).unwrap();
        assert_eq!(scheduler.submit(0, lane, step), vec![expected]);
        match expected {
            ScheduledOutcome::Delivered { .. } => assert!(scheduler.advance(1).is_empty()),
            ScheduledOutcome::Failed { .. } => assert_eq!(scheduler.advance(1), vec![expected]),
        }
    }
}

#[test]
fn invalid_lane_and_regressing_tick_become_retained_failures() {
    let lane = Lane::new(0);
    let unknown_lane = Lane::new(1);
    let source = Member::new(0);
    let step = VectorStep::complete(source, WireTag::ZERO);
    let mut invalid_lane =
        OrderedFaultScheduler::new([lane], FaultScript::new([]).unwrap()).unwrap();
    assert_eq!(
        invalid_lane.submit(0, unknown_lane, step),
        vec![ScheduledOutcome::failed(Failure::UnknownLane)]
    );

    let mut regressing_tick =
        OrderedFaultScheduler::new([lane], FaultScript::new([]).unwrap()).unwrap();
    assert_eq!(
        regressing_tick.submit(1, lane, step),
        vec![ScheduledOutcome::delivered(lane, step)]
    );
    assert_eq!(
        regressing_tick.advance(0),
        vec![ScheduledOutcome::failed(Failure::TickRegression)]
    );
}
