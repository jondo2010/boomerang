use super::{
    ConstructionError, Failure, FaultEffect, FaultScript, FaultScriptError, Lane, Member,
    OrderedFaultScheduler, Outcome, ReferenceCoordinator, Route, RouteTopology, ScheduledOutcome,
    Topology, VectorStep,
};
use crate::WireTag;

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
fn completion_releases_only_the_accounted_frontier() {
    let source = Member::new(0);
    let destination = Member::new(1);
    let route = Route::new(0);
    let earlier = WireTag::finite(0, 5);
    let later = WireTag::finite(0, 10);
    let mut oracle = ReferenceCoordinator::new(
        [source, destination],
        Topology::new([(route, RouteTopology::new(source, destination))]).unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        oracle.apply(VectorStep::publish(source, 1, Some(earlier))),
        vec![Outcome::grant(source, 1, WireTag::FOREVER)]
    );
    assert_eq!(
        oracle.apply(VectorStep::payload(source, route, earlier)),
        vec![Outcome::payload(destination, route, earlier)]
    );
    assert!(oracle
        .apply(VectorStep::complete(source, earlier))
        .is_empty());

    // A later NET does not erase B's delivered, still-accounted earlier tag.
    assert_eq!(
        oracle.apply(VectorStep::publish(source, 2, Some(later))),
        vec![Outcome::grant(source, 2, WireTag::FOREVER)]
    );
    assert!(oracle
        .apply(VectorStep::publish(
            destination,
            1,
            Some(WireTag::finite(0, 9))
        ))
        .is_empty());
    assert!(oracle
        .apply(VectorStep::complete(destination, WireTag::finite(0, 4)))
        .is_empty());

    assert_eq!(
        oracle.apply(VectorStep::complete(destination, earlier)),
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
fn separate_net_and_ltc_lane_ticks_drive_the_oracle_without_coarrival() {
    let source = Member::new(0);
    let destination = Member::new(1);
    let net_lane = Lane::new(0);
    let ltc_lane = Lane::new(1);
    let route = Route::new(0);
    let tag = WireTag::finite(5, 0);
    let mut scheduler =
        OrderedFaultScheduler::new([net_lane, ltc_lane], FaultScript::new([]).unwrap()).unwrap();
    let mut oracle = ReferenceCoordinator::new(
        [source, destination],
        Topology::new([(route, RouteTopology::new(source, destination))]).unwrap(),
        1,
    )
    .unwrap();

    assert_eq!(
        scheduler.submit(0, net_lane, VectorStep::publish(source, 1, Some(tag))),
        vec![ScheduledOutcome::delivered(
            net_lane,
            VectorStep::publish(source, 1, Some(tag)),
        )]
    );
    assert_eq!(
        oracle.apply(VectorStep::publish(source, 1, Some(tag))),
        vec![Outcome::grant(source, 1, WireTag::FOREVER)]
    );
    assert_eq!(
        scheduler.submit(2, ltc_lane, VectorStep::complete(destination, tag)),
        vec![ScheduledOutcome::delivered(
            ltc_lane,
            VectorStep::complete(destination, tag),
        )]
    );
    assert!(oracle
        .apply(VectorStep::complete(destination, tag))
        .is_empty());
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
