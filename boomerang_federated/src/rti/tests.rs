use super::*;
use crate::protocol::EndpointId;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSnapshot {
    federates: tinymap::TinySecondaryMap<FederateKey, FederateCoordination>,
}

fn snapshot(rti: &RtiState) -> StateSnapshot {
    StateSnapshot {
        federates: rti.runtime.federates.clone(),
    }
}

fn fed(id: &str) -> FederateId {
    FederateId::new(id)
}

fn endpoint(id: &str) -> EndpointId {
    EndpointId::new(id)
}

fn federate_part(id: &str, incoming: &[(&str, WireDelay)], affected: &[&str]) -> RtiFederateParts {
    RtiFederateParts {
        id: fed(id),
        transitive_incoming: incoming
            .iter()
            .map(|(source, delay)| (fed(source), *delay))
            .collect(),
        affected_downstream: affected.iter().map(|target| fed(target)).collect(),
    }
}

fn endpoint_part(source: &str, target: &str, id: &str, delay: WireDelay) -> RtiEndpointParts {
    RtiEndpointParts {
        id: endpoint(id),
        source: fed(source),
        target: fed(target),
        delay,
    }
}

fn new_rti(
    federates: impl IntoIterator<Item = RtiFederateParts>,
    endpoints: impl IntoIterator<Item = RtiEndpointParts>,
) -> RtiState {
    RtiState::from_graph(super::test_graph(federates, endpoints))
}

fn single_federate(id: &str) -> RtiState {
    new_rti([federate_part(id, &[], &[])], [])
}

fn coordination<'a>(rti: &'a RtiState, federate_id: &FederateId) -> &'a FederateCoordination {
    let key = rti
        .graph
        .federate_key(federate_id)
        .expect("test federate must exist");
    rti.runtime
        .federates
        .get(key)
        .expect("test federate must exist")
}

fn coordination_mut<'a>(
    rti: &'a mut RtiState,
    federate_id: &FederateId,
) -> &'a mut FederateCoordination {
    let key = rti
        .graph
        .federate_key(federate_id)
        .expect("test federate must exist");
    rti.runtime
        .federates
        .get_mut(key)
        .expect("test federate must exist")
}

fn rti_with_edge(delay: WireDelay) -> RtiState {
    new_rti(
        [
            federate_part("source", &[], &["target"]),
            federate_part("target", &[("source", delay)], &[]),
        ],
        [endpoint_part(
            "source",
            "target",
            "source.out->target.in",
            delay,
        )],
    )
}

#[test]
fn rti_coordination_is_indexed_by_compiled_federate_key() {
    let rti = new_rti(
        [federate_part("b", &[], &[]), federate_part("a", &[], &[])],
        [],
    );
    let a = rti.graph.federate_key(&fed("a")).unwrap();
    let b = rti.graph.federate_key(&fed("b")).unwrap();

    assert_ne!(a, b);
    assert_eq!(
        rti.runtime.federates.get(a),
        Some(&FederateCoordination::default())
    );
    assert_eq!(
        rti.runtime.federates.get(b),
        Some(&FederateCoordination::default())
    );
}

#[test]
fn handle_characterizes_legal_transition_sequence() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let source = fed("source");
    let target = fed("target");
    let endpoint = endpoint("source.out->target.in");
    let source_next = WireTag::finite(0, 1);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: source.clone(),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        vec![RtiDelivery::new(
            source.clone(),
            RtiToFederate::Tag { tag: WireTag::ZERO },
        )]
    );
    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: source.clone(),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        Vec::new()
    );
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: source.clone(),
            tag: source_next,
        })
        .unwrap(),
        vec![RtiDelivery::new(
            source.clone(),
            RtiToFederate::Tag { tag: source_next },
        )]
    );
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: target.clone(),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        vec![RtiDelivery::new(
            target.clone(),
            RtiToFederate::Tag { tag: WireTag::ZERO },
        )]
    );

    let payload = vec![1, 2, 3];
    assert_eq!(
        rti.handle(FederateToRti::Msg {
            source: source.clone(),
            target: target.clone(),
            endpoint: endpoint.clone(),
            tag: WireTag::ZERO,
            payload: payload.clone(),
        })
        .unwrap(),
        vec![RtiDelivery::new(
            target.clone(),
            RtiToFederate::Msg {
                source: source.clone(),
                endpoint,
                tag: WireTag::ZERO,
                payload,
            },
        )]
    );
    assert!(coordination(&rti, &target)
        .in_transit
        .contains(&WireTag::ZERO));
    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: target.clone(),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        Vec::new()
    );

    for federate_id in [&source, &target] {
        assert_eq!(
            rti.handle(FederateToRti::Net {
                federate_id: federate_id.clone(),
                tag: WireTag::FOREVER,
            })
            .unwrap(),
            Vec::new()
        );
        assert_eq!(
            rti.handle(FederateToRti::Stop {
                federate_id: federate_id.clone(),
            })
            .unwrap(),
            Vec::new()
        );
    }

    assert_eq!(
        coordination(&rti, &source),
        &FederateCoordination {
            lifecycle: FederateLifecycle::Stopped,
            last_completed: WireTag::ZERO,
            last_granted: Some(source_next),
            in_transit: BTreeSet::new(),
        }
    );
    assert_eq!(
        coordination(&rti, &target),
        &FederateCoordination {
            lifecycle: FederateLifecycle::Stopped,
            last_completed: WireTag::ZERO,
            last_granted: Some(WireTag::ZERO),
            in_transit: BTreeSet::new(),
        }
    );
}

#[test]
fn repeated_net_is_idempotent_and_regression_is_failure_atomic() {
    let mut rti = single_federate("solo");
    let requested = WireTag::finite(10, 0);
    let regressing = WireTag::finite(5, 0);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: requested,
        })
        .unwrap(),
        vec![RtiDelivery::new(
            fed("solo"),
            RtiToFederate::Tag { tag: requested },
        )]
    );
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: requested,
        })
        .unwrap(),
        Vec::new()
    );
    rti.handle(FederateToRti::Ltc {
        federate_id: fed("solo"),
        tag: requested,
    })
    .unwrap();
    let before_regression = snapshot(&rti);
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: regressing,
        }),
        Err(RtiError::RegressingNet {
            federate_id: fed("solo"),
            previous: requested,
            requested: regressing,
        })
    );
    assert_eq!(snapshot(&rti), before_regression);

    let state = coordination(&rti, &fed("solo"));
    assert_eq!(state.last_granted, Some(requested));
    assert_eq!(
        state.lifecycle,
        FederateLifecycle::Running {
            next_event: NextEvent::Finite(requested),
        }
    );
}

#[test]
fn pending_net_can_be_revised_to_an_earlier_ungranted_tag() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let target = fed("target");

    assert!(rti
        .handle(FederateToRti::Net {
            federate_id: target.clone(),
            tag: WireTag::finite(100, 0),
        })
        .unwrap()
        .is_empty());
    assert!(rti
        .handle(FederateToRti::Net {
            federate_id: target.clone(),
            tag: WireTag::finite(10, 0),
        })
        .unwrap()
        .is_empty());

    assert_eq!(
        coordination(&rti, &target).lifecycle,
        FederateLifecycle::Running {
            next_event: NextEvent::Finite(WireTag::finite(10, 0)),
        }
    );
    assert_eq!(coordination(&rti, &target).last_granted, None);
}

#[test]
fn repeated_ltc_is_idempotent_and_regression_is_failure_atomic() {
    let mut rti = single_federate("solo");
    let completed = WireTag::finite(10, 0);

    for tag in [completed, completed] {
        assert_eq!(
            rti.handle(FederateToRti::Ltc {
                federate_id: fed("solo"),
                tag,
            })
            .unwrap(),
            Vec::new()
        );
    }

    let before_regression = snapshot(&rti);
    let regressing = WireTag::finite(5, 0);
    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("solo"),
            tag: regressing,
        }),
        Err(RtiError::RegressingLtc {
            federate_id: fed("solo"),
            previous: completed,
            completed: regressing,
        })
    );
    assert_eq!(snapshot(&rti), before_regression);
    assert_eq!(coordination(&rti, &fed("solo")).last_completed, completed);
}

#[test]
fn net_never_is_rejected_without_mutation() {
    let mut rti = single_federate("solo");
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: WireTag::NEVER,
        }),
        Err(RtiError::InvalidTag {
            event: "NET",
            federate_id: fed("solo"),
            tag: WireTag::NEVER,
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn unknown_federate_errors_are_failure_atomic() {
    let unknown = fed("unknown");
    let cases = [
        FederateToRti::Hello {
            federate_id: unknown.clone(),
        },
        FederateToRti::Net {
            federate_id: unknown.clone(),
            tag: WireTag::ZERO,
        },
        FederateToRti::Ltc {
            federate_id: unknown.clone(),
            tag: WireTag::ZERO,
        },
        FederateToRti::Msg {
            source: unknown.clone(),
            target: fed("target"),
            endpoint: endpoint("unknown.out->target.in"),
            tag: WireTag::ZERO,
            payload: vec![1],
        },
        FederateToRti::Msg {
            source: fed("source"),
            target: unknown.clone(),
            endpoint: endpoint("source.out->unknown.in"),
            tag: WireTag::ZERO,
            payload: vec![2],
        },
        FederateToRti::Stop {
            federate_id: unknown.clone(),
        },
    ];

    for message in cases {
        let mut rti = new_rti(
            [
                federate_part("source", &[], &[]),
                federate_part("target", &[], &[]),
            ],
            [],
        );
        rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::ZERO)
            .unwrap();
        let before = snapshot(&rti);
        assert_eq!(
            rti.handle(message),
            Err(RtiError::UnknownFederate(unknown.clone()))
        );
        assert_eq!(snapshot(&rti), before);
    }
}

#[test]
fn authenticated_origin_mismatches_are_failure_atomic() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle_from(
            &fed("source"),
            FederateToRti::Net {
                federate_id: fed("target"),
                tag: WireTag::ZERO,
            },
        ),
        Err(RtiError::FederateIdentityMismatch {
            event: "NET",
            authenticated_federate: fed("source"),
            claimed_federate: fed("target"),
        })
    );
    assert_eq!(snapshot(&rti), before);

    assert_eq!(
        rti.handle_from(
            &fed("target"),
            FederateToRti::Msg {
                source: fed("source"),
                target: fed("target"),
                endpoint: endpoint("source.out->target.in"),
                tag: WireTag::ZERO,
                payload: vec![1],
            },
        ),
        Err(RtiError::FederateIdentityMismatch {
            event: "MSG",
            authenticated_federate: fed("target"),
            claimed_federate: fed("source"),
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn invalid_tags_and_lifecycle_transitions_are_failure_atomic() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    for (message, event, federate_id, tag) in [
        (
            FederateToRti::Net {
                federate_id: fed("source"),
                tag: WireTag::finite(-1, 0),
            },
            "NET",
            fed("source"),
            WireTag::finite(-1, 0),
        ),
        (
            FederateToRti::Ltc {
                federate_id: fed("source"),
                tag: WireTag::FOREVER,
            },
            "LTC",
            fed("source"),
            WireTag::FOREVER,
        ),
        (
            FederateToRti::Msg {
                source: fed("source"),
                target: fed("target"),
                endpoint: endpoint("source.out->target.in"),
                tag: WireTag::finite(-1, 0),
                payload: vec![],
            },
            "MSG",
            fed("source"),
            WireTag::finite(-1, 0),
        ),
    ] {
        let before = snapshot(&rti);
        assert_eq!(
            rti.handle(message),
            Err(RtiError::InvalidTag {
                event,
                federate_id,
                tag,
            })
        );
        assert_eq!(snapshot(&rti), before);
    }

    let before_stop = snapshot(&rti);
    assert_eq!(
        rti.handle(FederateToRti::Stop {
            federate_id: fed("source"),
        }),
        Err(RtiError::InvalidLifecycleTransition {
            federate_id: fed("source"),
            event: "Stop",
            lifecycle: "running with future events",
        })
    );
    assert_eq!(snapshot(&rti), before_stop);

    rti.handle(FederateToRti::Net {
        federate_id: fed("source"),
        tag: WireTag::FOREVER,
    })
    .unwrap();
    let no_future = snapshot(&rti);
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("source"),
            tag: WireTag::finite(1, 0),
        }),
        Err(RtiError::InvalidLifecycleTransition {
            federate_id: fed("source"),
            event: "NET",
            lifecycle: "no-future",
        })
    );
    assert_eq!(snapshot(&rti), no_future);
}

#[test]
fn state_handler_rejects_route_absent_from_topology_without_mutation() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let invalid_endpoint = endpoint("source.out->target.other");
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Msg {
            source: fed("source"),
            target: fed("target"),
            endpoint: invalid_endpoint.clone(),
            tag: WireTag::ZERO,
            payload: vec![9],
        }),
        Err(RtiError::InvalidRoute {
            source_federate: fed("source"),
            target_federate: fed("target"),
            endpoint: invalid_endpoint,
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn grants_tag_when_federate_has_no_upstream_or_in_transit_messages() {
    let mut rti = single_federate("solo");

    let decision = rti.request_tag(&fed("solo"), WireTag::ZERO).unwrap();

    assert_eq!(decision, GrantDecision::Granted { tag: WireTag::ZERO });
}

#[test]
fn upstream_net_at_requested_tag_blocks_tag_grant() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    assert!(matches!(
        rti.request_tag(&fed("source"), WireTag::ZERO).unwrap(),
        GrantDecision::Granted { .. }
    ));

    let blocked = rti.request_tag(&fed("target"), WireTag::ZERO).unwrap();
    assert_eq!(
        blocked,
        GrantDecision::Blocked {
            requested: WireTag::ZERO,
            earliest_incoming: Some(WireTag::ZERO),
        }
    );

    assert!(matches!(
        rti.request_tag(&fed("source"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Granted { .. }
    ));
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::ZERO).unwrap(),
        GrantDecision::Granted { tag: WireTag::ZERO }
    );
}

#[test]
fn multiple_same_tag_messages_share_one_in_transit_entry() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let message = || FederateToRti::Msg {
        source: fed("source"),
        target: fed("target"),
        endpoint: endpoint("source.out->target.in"),
        tag: WireTag::ZERO,
        payload: vec![1],
    };

    rti.handle(message()).unwrap();
    rti.handle(message()).unwrap();

    assert_eq!(
        coordination(&rti, &fed("target")).in_transit,
        BTreeSet::from([WireTag::ZERO])
    );
}

#[test]
fn ltc_clears_in_transit_tags_through_completion() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    for tag in [
        WireTag::finite(3, 0),
        WireTag::finite(5, 0),
        WireTag::finite(7, 0),
    ] {
        rti.record_in_transit_message(&fed("source"), &fed("target"), tag)
            .unwrap();
    }

    rti.handle(FederateToRti::Ltc {
        federate_id: fed("target"),
        tag: WireTag::finite(5, 0),
    })
    .unwrap();

    assert_eq!(
        coordination(&rti, &fed("target")).in_transit,
        BTreeSet::from([WireTag::finite(7, 0)])
    );
}

#[test]
fn target_ltc_reconsiders_downstream_request_after_clearing_in_transit_tag() {
    let mut rti = new_rti(
        [
            federate_part("source", &[], &["downstream", "target"]),
            federate_part("target", &[("source", WireDelay::ZERO)], &["downstream"]),
            federate_part(
                "downstream",
                &[("source", WireDelay::ZERO), ("target", WireDelay::ZERO)],
                &[],
            ),
        ],
        [
            endpoint_part("source", "target", "source.out->target.in", WireDelay::ZERO),
            endpoint_part(
                "target",
                "downstream",
                "target.out->downstream.in",
                WireDelay::ZERO,
            ),
        ],
    );
    rti.handle(FederateToRti::Net {
        federate_id: fed("source"),
        tag: WireTag::FOREVER,
    })
    .unwrap();
    rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::finite(0, 5))
        .unwrap();
    rti.handle(FederateToRti::Net {
        federate_id: fed("target"),
        tag: WireTag::finite(0, 10),
    })
    .unwrap();

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("downstream"),
            tag: WireTag::finite(0, 9),
        })
        .unwrap(),
        Vec::new()
    );

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("target"),
            tag: WireTag::finite(0, 5),
        })
        .unwrap(),
        vec![RtiDelivery::new(
            fed("downstream"),
            RtiToFederate::Tag {
                tag: WireTag::finite(0, 9),
            },
        )]
    );
}

#[test]
fn message_at_completed_tag_is_forwarded_without_being_recorded() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let completed = WireTag::finite(5, 0);
    rti.handle(FederateToRti::Ltc {
        federate_id: fed("target"),
        tag: completed,
    })
    .unwrap();

    let deliveries = rti
        .handle(FederateToRti::Msg {
            source: fed("source"),
            target: fed("target"),
            endpoint: endpoint("source.out->target.in"),
            tag: completed,
            payload: vec![1],
        })
        .unwrap();

    assert!(coordination(&rti, &fed("target")).in_transit.is_empty());
    assert_eq!(deliveries.len(), 1);
    assert!(matches!(deliveries[0].message, RtiToFederate::Msg { tag, .. } if tag == completed));
}

#[test]
fn net_overflow_is_failure_atomic() {
    let delay = WireDelay::from_nanos(1);
    let overflowing = WireTag::finite(i128::MAX, 0);
    let mut rti = rti_with_edge(delay);
    assert_eq!(
        rti.request_tag(&fed("source"), overflowing).unwrap(),
        GrantDecision::Granted { tag: overflowing }
    );
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("target"),
            tag: WireTag::ZERO,
        }),
        Err(RtiError::TagDelayOverflow {
            tag: overflowing,
            delay_ns: 1,
        })
    );

    assert_eq!(snapshot(&rti), before);
}

#[test]
fn ltc_does_not_reevaluate_unaffected_pending_grants() {
    let delay = WireDelay::from_nanos(1);
    let overflowing = WireTag::finite(i128::MAX, 0);
    let mut rti = new_rti(
        [
            federate_part("a", &[("x", WireDelay::ZERO)], &[]),
            federate_part("source", &[], &["z"]),
            federate_part("x", &[], &["a"]),
            federate_part("z", &[("source", delay)], &[]),
        ],
        [
            endpoint_part("source", "z", "source.out->z.in", delay),
            endpoint_part("x", "a", "x.out->a.in", WireDelay::ZERO),
        ],
    );

    rti.request_tag(&fed("x"), WireTag::ZERO).unwrap();
    assert!(matches!(
        rti.request_tag(&fed("a"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked { .. }
    ));
    assert_eq!(
        rti.request_tag(&fed("source"), overflowing).unwrap(),
        GrantDecision::Granted { tag: overflowing }
    );
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("source"),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        Vec::new()
    );

    assert_eq!(coordination(&rti, &fed("a")).last_granted, None);
    assert_eq!(
        coordination(&rti, &fed("source")).last_completed,
        WireTag::ZERO
    );
    assert_eq!(
        coordination(&rti, &fed("source")).last_granted,
        before
            .federates
            .get(rti.graph.federate_key(&fed("source")).unwrap())
            .unwrap()
            .last_granted
    );
}

#[test]
fn ltc_grants_the_minimum_delay_adjusted_upstream_completion() {
    let mut rti = rti_with_edge(WireDelay::from_nanos(5));

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("source"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap(),
        vec![RtiDelivery::new(
            fed("source"),
            RtiToFederate::Tag {
                tag: WireTag::finite(10, 0),
            },
        )]
    );
    assert!(rti
        .handle(FederateToRti::Net {
            federate_id: fed("target"),
            tag: WireTag::finite(15, 0),
        })
        .unwrap()
        .is_empty());

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("source"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap(),
        vec![RtiDelivery::new(
            fed("target"),
            RtiToFederate::Tag {
                tag: WireTag::finite(15, 0),
            },
        )]
    );
}

#[test]
fn ltc_reevaluates_sorted_transitive_downstream_work_set() {
    let mut rti = new_rti(
        [
            federate_part("z", &[("source", WireDelay::ZERO)], &[]),
            federate_part("source", &[], &["a", "z"]),
            federate_part("a", &[("source", WireDelay::ZERO)], &[]),
        ],
        [
            endpoint_part("source", "z", "source.out->z.in", WireDelay::ZERO),
            endpoint_part("source", "a", "source.out->a.in", WireDelay::ZERO),
        ],
    );

    rti.handle(FederateToRti::Net {
        federate_id: fed("source"),
        tag: WireTag::finite(10, 0),
    })
    .unwrap();
    for target in [fed("z"), fed("a")] {
        assert!(rti
            .handle(FederateToRti::Net {
                federate_id: target,
                tag: WireTag::finite(10, 0),
            })
            .unwrap()
            .is_empty());
    }

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("source"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap(),
        ["a", "z"]
            .into_iter()
            .map(|id| {
                RtiDelivery::new(
                    fed(id),
                    RtiToFederate::Tag {
                        tag: WireTag::finite(10, 0),
                    },
                )
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn ltc_transitive_grant_overflow_is_failure_atomic() {
    let mut rti = rti_with_edge(WireDelay::from_nanos(1));
    assert!(rti
        .handle(FederateToRti::Net {
            federate_id: fed("target"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap()
        .is_empty());
    coordination_mut(&mut rti, &fed("source")).request(WireTag::finite(i128::MAX, 0));
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("source"),
            tag: WireTag::ZERO,
        }),
        Err(RtiError::TagDelayOverflow {
            tag: WireTag::finite(i128::MAX, 0),
            delay_ns: 1,
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn lf_tag_predecessor_preserves_sentinels_and_tag_boundaries() {
    assert_eq!(
        latest_tag_strictly_before(WireTag::NEVER),
        Some(WireTag::NEVER)
    );
    assert_eq!(
        latest_tag_strictly_before(WireTag::FOREVER),
        Some(WireTag::FOREVER)
    );
    assert_eq!(
        latest_tag_strictly_before(WireTag::finite(5, 2)),
        Some(WireTag::finite(5, 1))
    );
    assert_eq!(
        latest_tag_strictly_before(WireTag::finite(5, 0)),
        Some(WireTag::finite(4, u64::MAX))
    );
}

#[test]
fn net_reevaluates_self_then_sorted_downstream_federates() {
    let delay = WireDelay::from_nanos(1);
    let mut rti = new_rti(
        [
            federate_part("z", &[("source", delay)], &[]),
            federate_part("source", &[], &["a", "z"]),
            federate_part("a", &[("source", delay)], &[]),
        ],
        [
            endpoint_part("source", "z", "source.out->z.in", delay),
            endpoint_part("source", "a", "source.out->a.in", delay),
        ],
    );

    for target in [fed("z"), fed("a")] {
        assert!(rti
            .handle(FederateToRti::Net {
                federate_id: target,
                tag: WireTag::ZERO,
            })
            .unwrap()
            .is_empty());
    }

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("source"),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        vec![
            RtiDelivery::new(fed("source"), RtiToFederate::Tag { tag: WireTag::ZERO }),
            RtiDelivery::new(
                fed("a"),
                RtiToFederate::Tag {
                    tag: WireTag::finite(0, u64::MAX),
                },
            ),
            RtiDelivery::new(
                fed("z"),
                RtiToFederate::Tag {
                    tag: WireTag::finite(0, u64::MAX),
                },
            ),
        ]
    );
}

#[test]
fn unrelated_net_does_not_scan_an_overflowing_topology_component() {
    let delay = WireDelay::from_nanos(1);
    let overflowing = WireTag::finite(i128::MAX, 0);
    let mut rti = new_rti(
        [
            federate_part("isolated", &[], &[]),
            federate_part("source", &[], &["z"]),
            federate_part("z", &[("source", delay)], &[]),
        ],
        [endpoint_part("source", "z", "source.out->z.in", delay)],
    );

    assert_eq!(
        rti.request_tag(&fed("source"), overflowing).unwrap(),
        GrantDecision::Granted { tag: overflowing }
    );
    coordination_mut(&mut rti, &fed("z")).request(WireTag::ZERO);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("isolated"),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        vec![RtiDelivery::new(
            fed("isolated"),
            RtiToFederate::Tag { tag: WireTag::ZERO },
        )]
    );
    assert_eq!(coordination(&rti, &fed("z")).last_granted, None);
}

#[test]
fn same_timestamp_microstep_progression_unblocks_downstream_grant() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    assert_eq!(
        rti.request_tag(&fed("source"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 1),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(0, 1),
            earliest_incoming: Some(WireTag::finite(0, 1)),
        }
    );

    assert_eq!(
        rti.request_tag(&fed("source"), WireTag::finite(0, 2))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 2),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 1),
        }
    );
}

#[test]
fn multi_hop_chain_requires_each_upstream_to_advance_past_the_requested_tag() {
    let mut rti = new_rti(
        [
            federate_part("a", &[], &["b", "c"]),
            federate_part("b", &[("a", WireDelay::ZERO)], &["c"]),
            federate_part("c", &[("a", WireDelay::ZERO), ("b", WireDelay::ZERO)], &[]),
        ],
        [
            endpoint_part("a", "b", "a.out->b.in", WireDelay::ZERO),
            endpoint_part("b", "c", "b.out->c.in", WireDelay::ZERO),
        ],
    );

    assert_eq!(
        rti.request_tag(&fed("b"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::ZERO,
            earliest_incoming: Some(WireTag::NEVER),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("c"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::ZERO,
            earliest_incoming: Some(WireTag::NEVER),
        }
    );

    assert_eq!(
        rti.request_tag(&fed("a"), WireTag::ZERO).unwrap(),
        GrantDecision::Granted { tag: WireTag::ZERO }
    );
    assert_eq!(
        rti.request_tag(&fed("a"), WireTag::finite(0, 1)).unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 1),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("b"), WireTag::ZERO).unwrap(),
        GrantDecision::Granted { tag: WireTag::ZERO }
    );
    assert_eq!(
        rti.request_tag(&fed("c"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::ZERO,
            earliest_incoming: Some(WireTag::ZERO),
        }
    );

    assert_eq!(
        rti.request_tag(&fed("a"), WireTag::finite(0, 2)).unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 2),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("b"), WireTag::finite(0, 1)).unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 1),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("c"), WireTag::ZERO).unwrap(),
        GrantDecision::Granted { tag: WireTag::ZERO }
    );
}

#[test]
fn transitive_eimt_blocks_an_earlier_upstream_through_an_intermediate() {
    let mut rti = new_rti(
        [
            federate_part("a", &[], &["b", "c"]),
            federate_part("b", &[("a", WireDelay::ZERO)], &["c"]),
            federate_part("c", &[("a", WireDelay::ZERO), ("b", WireDelay::ZERO)], &[]),
        ],
        [
            endpoint_part("a", "b", "a.out->b.in", WireDelay::ZERO),
            endpoint_part("b", "c", "b.out->c.in", WireDelay::ZERO),
        ],
    );

    assert_eq!(
        rti.request_tag(&fed("a"), WireTag::finite(50, 0)).unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(50, 0),
        }
    );
    assert!(matches!(
        rti.request_tag(&fed("b"), WireTag::finite(100, 0)).unwrap(),
        GrantDecision::Blocked { .. }
    ));

    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("c")).unwrap(),
        Some(WireTag::finite(50, 0))
    );
    assert_eq!(
        rti.request_tag(&fed("c"), WireTag::finite(99, 0)).unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(99, 0),
            earliest_incoming: Some(WireTag::finite(50, 0)),
        }
    );
}

#[test]
fn positive_delay_cycle_allows_startup_grants_after_both_federates_advertise() {
    let delay = WireDelay::from_nanos(10);
    let mut rti = new_rti(
        [
            federate_part(
                "a",
                &[("a", WireDelay::from_nanos(20)), ("b", delay)],
                &["b"],
            ),
            federate_part(
                "b",
                &[("a", delay), ("b", WireDelay::from_nanos(20))],
                &["a"],
            ),
        ],
        [
            endpoint_part("a", "b", "a.out->b.in", delay),
            endpoint_part("b", "a", "b.out->a.in", delay),
        ],
    );

    assert_eq!(
        rti.request_tag(&fed("a"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::ZERO,
            earliest_incoming: Some(WireTag::NEVER),
        }
    );

    let deliveries = rti
        .handle(FederateToRti::Net {
            federate_id: fed("b"),
            tag: WireTag::ZERO,
        })
        .unwrap();

    assert_eq!(
        deliveries,
        vec![
            RtiDelivery {
                federate_id: fed("b"),
                message: RtiToFederate::Tag {
                    tag: WireTag::finite(9, u64::MAX),
                },
            },
            RtiDelivery {
                federate_id: fed("a"),
                message: RtiToFederate::Tag {
                    tag: WireTag::finite(9, u64::MAX),
                },
            },
        ]
    );
    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("a")).unwrap(),
        Some(WireTag::finite(10, 0))
    );
    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("b")).unwrap(),
        Some(WireTag::finite(10, 0))
    );
}

#[test]
fn net_forever_unblocks_pending_downstream_without_granting_forever() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("target"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap(),
        Vec::new()
    );

    let deliveries = rti
        .handle(FederateToRti::Net {
            federate_id: fed("source"),
            tag: WireTag::FOREVER,
        })
        .unwrap();

    assert_eq!(
        deliveries,
        vec![RtiDelivery {
            federate_id: fed("target"),
            message: RtiToFederate::Tag {
                tag: WireTag::FOREVER
            },
        }]
    );
    assert_eq!(
        coordination(&rti, &fed("source")).lifecycle,
        FederateLifecycle::Running {
            next_event: NextEvent::NoFuture,
        }
    );
    assert_eq!(coordination(&rti, &fed("source")).last_granted, None);
}

#[test]
fn no_future_net_unblocks_pending_downstream_before_stop() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("target"),
            tag: WireTag::finite(10, 0),
        })
        .unwrap(),
        Vec::new()
    );

    let deliveries = rti
        .handle(FederateToRti::Net {
            federate_id: fed("source"),
            tag: WireTag::FOREVER,
        })
        .unwrap();

    assert_eq!(
        deliveries,
        vec![RtiDelivery {
            federate_id: fed("target"),
            message: RtiToFederate::Tag {
                tag: WireTag::FOREVER
            },
        }]
    );
    assert_eq!(
        rti.handle(FederateToRti::Stop {
            federate_id: fed("source"),
        })
        .unwrap(),
        Vec::new()
    );
    let source_state = coordination(&rti, &fed("source"));
    assert_eq!(source_state.lifecycle, FederateLifecycle::Stopped);
    assert_eq!(source_state.last_granted, None);
}

#[test]
fn stopped_federate_rejects_later_events_without_mutation() {
    let mut rti = single_federate("solo");

    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: WireTag::FOREVER,
        })
        .unwrap(),
        Vec::new()
    );
    assert_eq!(
        rti.handle(FederateToRti::Stop {
            federate_id: fed("solo"),
        })
        .unwrap(),
        Vec::new()
    );
    let stopped = snapshot(&rti);
    assert_eq!(
        rti.handle(FederateToRti::Net {
            federate_id: fed("solo"),
            tag: WireTag::finite(5, 0),
        }),
        Err(RtiError::InvalidLifecycleTransition {
            federate_id: fed("solo"),
            event: "NET",
            lifecycle: "stopped",
        })
    );
    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("solo"),
            tag: WireTag::finite(5, 0),
        }),
        Err(RtiError::InvalidLifecycleTransition {
            federate_id: fed("solo"),
            event: "LTC",
            lifecycle: "stopped",
        })
    );
    assert_eq!(
        rti.handle(FederateToRti::Stop {
            federate_id: fed("solo"),
        }),
        Err(RtiError::InvalidLifecycleTransition {
            federate_id: fed("solo"),
            event: "Stop",
            lifecycle: "stopped",
        })
    );
    assert_eq!(snapshot(&rti), stopped);
}

#[test]
fn message_already_sent_by_a_peer_can_arrive_after_target_stop() {
    let mut rti = rti_with_edge(WireDelay::ZERO);
    let target = fed("target");
    let source = fed("source");
    let tag = WireTag::finite(10, 0);

    rti.handle(FederateToRti::Net {
        federate_id: target.clone(),
        tag: WireTag::FOREVER,
    })
    .unwrap();
    rti.handle(FederateToRti::Stop {
        federate_id: target.clone(),
    })
    .unwrap();

    assert_eq!(
        rti.handle_from(
            &source,
            FederateToRti::Msg {
                source: source.clone(),
                target: target.clone(),
                endpoint: endpoint("source.out->target.in"),
                tag,
                payload: vec![1],
            },
        )
        .unwrap(),
        vec![RtiDelivery::new(
            target.clone(),
            RtiToFederate::Msg {
                source,
                endpoint: endpoint("source.out->target.in"),
                tag,
                payload: vec![1],
            },
        )]
    );
    assert!(coordination(&rti, &target).in_transit.contains(&tag));
}

#[test]
fn topology_delays_shift_earliest_incoming_message_tags() {
    let mut rti = rti_with_edge(WireDelay::from_nanos(10));

    rti.request_tag(&fed("source"), WireTag::ZERO).unwrap();

    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("target")).unwrap(),
        Some(WireTag::finite(10, 0))
    );
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(9, 0))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(9, u64::MAX),
        }
    );
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(10, 0))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(10, 0),
            earliest_incoming: Some(WireTag::finite(10, 0)),
        }
    );
}

#[test]
fn msg_frames_are_recorded_as_in_transit_and_forwarded_to_the_target() {
    let mut rti = rti_with_edge(WireDelay::ZERO);

    rti.handle(FederateToRti::Net {
        federate_id: fed("source"),
        tag: WireTag::FOREVER,
    })
    .unwrap();

    let deliveries = rti
        .handle(FederateToRti::Msg {
            source: fed("source"),
            target: fed("target"),
            endpoint: endpoint("source.out->target.in"),
            tag: WireTag::finite(3, 0),
            payload: vec![1, 2, 3],
        })
        .unwrap();

    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("target")).unwrap(),
        Some(WireTag::FOREVER)
    );
    assert!(coordination(&rti, &fed("target"))
        .in_transit
        .contains(&WireTag::finite(3, 0)));
    assert_eq!(
        deliveries,
        vec![RtiDelivery {
            federate_id: fed("target"),
            message: RtiToFederate::Msg {
                source: fed("source"),
                endpoint: endpoint("source.out->target.in"),
                tag: WireTag::finite(3, 0),
                payload: vec![1, 2, 3],
            },
        }]
    );
}
