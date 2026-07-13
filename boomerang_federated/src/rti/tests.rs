use super::*;
use crate::protocol::{EndpointId, FederatedTopology, NeighborStructure, TopologyEdge};

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateSnapshot {
    topology: CompiledTopology,
    federates: BTreeMap<FederateId, FederateCoordination>,
}

fn snapshot(rti: &RtiState) -> StateSnapshot {
    StateSnapshot {
        topology: rti.topology.clone(),
        federates: rti.federates.clone(),
    }
}

fn fed(id: &str) -> FederateId {
    FederateId::new(id)
}

fn endpoint(id: &str) -> EndpointId {
    EndpointId::new(id)
}

fn new_rti(topology: FederatedTopology) -> RtiState {
    RtiState::new(topology).expect("valid test topology")
}

fn coordination<'a>(rti: &'a RtiState, federate_id: &FederateId) -> &'a FederateCoordination {
    rti.federates
        .get(federate_id)
        .expect("test federate must exist")
}

fn coordination_mut<'a>(
    rti: &'a mut RtiState,
    federate_id: &FederateId,
) -> &'a mut FederateCoordination {
    rti.federates
        .get_mut(federate_id)
        .expect("test federate must exist")
}

fn topology_with_edge(delay: WireDelay) -> FederatedTopology {
    FederatedTopology::with_edges(
        [fed("source"), fed("target")],
        [TopologyEdge::new(
            fed("source"),
            fed("target"),
            endpoint("source.out->target.in"),
            delay,
        )],
    )
}

#[test]
fn handle_characterizes_legal_transition_sequence() {
    let topology = topology_with_edge(WireDelay::ZERO);
    let mut rti = new_rti(topology.clone());
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
    assert_eq!(
        rti.handle(FederateToRti::MsgAck {
            federate_id: target.clone(),
            tag: WireTag::ZERO,
        })
        .unwrap(),
        Vec::new()
    );
    assert!(coordination(&rti, &target).in_transit.is_empty());
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
            in_transit: BTreeMap::new(),
        }
    );
    assert_eq!(
        coordination(&rti, &target),
        &FederateCoordination {
            lifecycle: FederateLifecycle::Stopped,
            last_completed: WireTag::ZERO,
            last_granted: Some(WireTag::ZERO),
            in_transit: BTreeMap::new(),
        }
    );
}

#[test]
fn repeated_net_is_idempotent_and_regression_is_failure_atomic() {
    let mut rti = new_rti(FederatedTopology::new([fed("solo")]));
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));
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
    let mut rti = new_rti(FederatedTopology::new([fed("solo")]));
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
    let mut rti = new_rti(FederatedTopology::new([fed("solo")]));
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
    let mut baseline = new_rti(FederatedTopology::new([fed("source"), fed("target")]));
    baseline
        .record_in_transit_message(&fed("source"), &fed("target"), WireTag::ZERO)
        .unwrap();
    let unknown = fed("unknown");
    let cases = [
        FederateToRti::Hello {
            federate_id: unknown.clone(),
            topology: NeighborStructure {
                federate_id: unknown.clone(),
                upstream: Vec::new(),
                downstream: Vec::new(),
            },
        },
        FederateToRti::Net {
            federate_id: unknown.clone(),
            tag: WireTag::ZERO,
        },
        FederateToRti::Ltc {
            federate_id: unknown.clone(),
            tag: WireTag::ZERO,
        },
        FederateToRti::MsgAck {
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
        let mut rti = baseline.clone();
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
            FederateToRti::MsgAck {
                federate_id: fed("target"),
                tag: WireTag::NEVER,
            },
            "MsgAck",
            fed("target"),
            WireTag::NEVER,
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));
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
fn compiled_topology_indexes_dependencies_and_routes_deterministically() {
    let topology = FederatedTopology::with_edges(
        [fed("c"), fed("a"), fed("b")],
        [
            TopologyEdge::new(
                fed("b"),
                fed("c"),
                endpoint("b.out->c.in"),
                WireDelay::from_nanos(2),
            ),
            TopologyEdge::new(
                fed("a"),
                fed("c"),
                endpoint("a.out->c.in"),
                WireDelay::from_nanos(1),
            ),
            TopologyEdge::new(fed("a"), fed("b"), endpoint("a.out->b.in"), WireDelay::ZERO),
        ],
    );
    let rti = new_rti(topology.clone());

    assert_eq!(rti.topology(), &topology);
    assert_eq!(
        rti.topology.incoming(&fed("c")),
        [
            IncomingDependency {
                source: fed("a"),
                endpoint: endpoint("a.out->c.in"),
                delay: WireDelay::from_nanos(1),
            },
            IncomingDependency {
                source: fed("b"),
                endpoint: endpoint("b.out->c.in"),
                delay: WireDelay::from_nanos(2),
            },
        ]
    );
    assert_eq!(rti.topology.downstream(&fed("a")), [fed("b"), fed("c")]);
    assert_eq!(rti.topology.downstream(&fed("b")), [fed("c")]);
    assert!(rti.contains_route(&fed("a"), &fed("c"), &endpoint("a.out->c.in")));
    assert!(!rti.contains_route(&fed("c"), &fed("a"), &endpoint("a.out->c.in")));
}

#[test]
fn compiled_topology_rejects_duplicate_federates() {
    assert_eq!(
        RtiState::new(FederatedTopology::new([fed("a"), fed("a")])).unwrap_err(),
        RtiError::DuplicateFederate(fed("a"))
    );
}

#[test]
fn compiled_topology_rejects_undeclared_edge_members() {
    for (edge, missing) in [
        (
            TopologyEdge::new(
                fed("missing"),
                fed("target"),
                endpoint("missing.out->target.in"),
                WireDelay::ZERO,
            ),
            fed("missing"),
        ),
        (
            TopologyEdge::new(
                fed("source"),
                fed("missing"),
                endpoint("source.out->missing.in"),
                WireDelay::ZERO,
            ),
            fed("missing"),
        ),
    ] {
        let endpoint = edge.endpoint.clone();
        assert_eq!(
            RtiState::new(FederatedTopology::with_edges(
                [fed("source"), fed("target")],
                [edge],
            ))
            .unwrap_err(),
            RtiError::UndeclaredEdgeFederate {
                endpoint,
                federate_id: missing,
            }
        );
    }
}

#[test]
fn compiled_topology_rejects_missing_duplicate_and_conflicting_routes() {
    let source = fed("source");
    let target = fed("target");
    let route_endpoint = endpoint("source.out->target.in");
    let valid = TopologyEdge::new(
        source.clone(),
        target.clone(),
        route_endpoint.clone(),
        WireDelay::ZERO,
    );

    assert_eq!(
        RtiState::new(FederatedTopology::with_edges(
            [source.clone(), target.clone()],
            [TopologyEdge::new(
                source.clone(),
                target.clone(),
                endpoint(""),
                WireDelay::ZERO,
            )],
        ))
        .unwrap_err(),
        RtiError::MissingRouteEndpoint {
            route_source: source.clone(),
            route_target: target.clone(),
        }
    );
    assert_eq!(
        RtiState::new(FederatedTopology::with_edges(
            [source.clone(), target.clone()],
            [valid.clone(), valid.clone()],
        ))
        .unwrap_err(),
        RtiError::DuplicateRoute {
            route_source: source.clone(),
            route_target: target.clone(),
            endpoint: route_endpoint.clone(),
        }
    );
    assert_eq!(
        RtiState::new(FederatedTopology::with_edges(
            [source, target],
            [
                valid,
                TopologyEdge::new(
                    fed("source"),
                    fed("target"),
                    route_endpoint.clone(),
                    WireDelay::from_nanos(1),
                ),
            ],
        ))
        .unwrap_err(),
        RtiError::ConflictingRoute {
            endpoint: route_endpoint,
        }
    );
}

#[test]
fn grants_tag_when_federate_has_no_upstream_or_in_transit_messages() {
    let mut rti = new_rti(FederatedTopology::new([fed("solo")]));

    let decision = rti.request_tag(&fed("solo"), WireTag::ZERO).unwrap();

    assert_eq!(decision, GrantDecision::Granted { tag: WireTag::ZERO });
}

#[test]
fn upstream_net_at_requested_tag_blocks_tag_grant() {
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
fn in_transit_message_blocks_grant_until_target_msg_ack() {
    let mut rti = new_rti(FederatedTopology::new([fed("source"), fed("target")]));

    rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::finite(5, 0))
        .unwrap();

    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(10, 0))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(10, 0),
            earliest_incoming: Some(WireTag::finite(5, 0)),
        }
    );

    rti.acknowledge_message(&fed("target"), WireTag::finite(5, 0))
        .unwrap();

    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(10, 0))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(10, 0),
        }
    );
}

#[test]
fn multiple_same_tag_messages_require_one_msg_ack_each() {
    let mut rti = new_rti(FederatedTopology::new([fed("source"), fed("target")]));

    rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::ZERO)
        .unwrap();
    rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::ZERO)
        .unwrap();

    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(0, 1),
            earliest_incoming: Some(WireTag::ZERO),
        }
    );

    rti.complete_tag(&fed("target"), WireTag::ZERO).unwrap();

    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(0, 1),
            earliest_incoming: Some(WireTag::ZERO),
        }
    );

    rti.acknowledge_message(&fed("target"), WireTag::ZERO)
        .unwrap();
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Blocked {
            requested: WireTag::finite(0, 1),
            earliest_incoming: Some(WireTag::ZERO),
        }
    );

    rti.acknowledge_message(&fed("target"), WireTag::ZERO)
        .unwrap();

    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(0, 1))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(0, 1),
        }
    );
}

#[test]
fn in_transit_count_overflow_is_failure_atomic() {
    let mut rti = new_rti(FederatedTopology::new([fed("source"), fed("target")]));
    coordination_mut(&mut rti, &fed("target"))
        .in_transit
        .insert(
            WireTag::ZERO,
            NonZeroUsize::new(usize::MAX).expect("maximum usize is nonzero"),
        );
    let before = snapshot(&rti);

    assert_eq!(
        rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::ZERO),
        Err(RtiError::MessageCountOverflow {
            federate_id: fed("target"),
            tag: WireTag::ZERO,
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn msg_ack_can_trigger_pending_grant() {
    let mut rti = new_rti(FederatedTopology::new([fed("source"), fed("target")]));
    rti.record_in_transit_message(&fed("source"), &fed("target"), WireTag::finite(5, 0))
        .unwrap();
    assert!(matches!(
        rti.request_tag(&fed("target"), WireTag::finite(10, 0))
            .unwrap(),
        GrantDecision::Blocked { .. }
    ));

    let deliveries = rti
        .handle(FederateToRti::MsgAck {
            federate_id: fed("target"),
            tag: WireTag::finite(5, 0),
        })
        .unwrap();

    assert_eq!(
        deliveries,
        vec![RtiDelivery {
            federate_id: fed("target"),
            message: RtiToFederate::Tag {
                tag: WireTag::finite(10, 0),
            },
        }]
    );
}

#[test]
fn unmatched_msg_ack_returns_typed_error() {
    let mut rti = new_rti(FederatedTopology::new([fed("source"), fed("target")]));
    let other_tag = WireTag::finite(1, 0);
    rti.record_in_transit_message(&fed("source"), &fed("target"), other_tag)
        .unwrap();
    let before = snapshot(&rti);

    assert_eq!(
        rti.acknowledge_message(&fed("target"), WireTag::ZERO),
        Err(RtiError::UnmatchedMessageAck {
            federate_id: fed("target"),
            tag: WireTag::ZERO,
        })
    );
    assert_eq!(snapshot(&rti), before);
}

#[test]
fn net_overflow_is_failure_atomic() {
    let delay = WireDelay::from_nanos(1);
    let overflowing = WireTag::finite(i128::MAX, 0);
    let mut rti = new_rti(topology_with_edge(delay));
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
fn global_grant_scan_overflow_is_failure_atomic() {
    let delay = WireDelay::from_nanos(1);
    let overflowing = WireTag::finite(i128::MAX, 0);
    let mut rti = new_rti(FederatedTopology::with_edges(
        [fed("a"), fed("source"), fed("z")],
        [TopologyEdge::new(
            fed("source"),
            fed("z"),
            endpoint("source.out->z.in"),
            delay,
        )],
    ));

    rti.record_in_transit_message(&fed("source"), &fed("a"), WireTag::ZERO)
        .unwrap();
    assert!(matches!(
        rti.request_tag(&fed("a"), WireTag::ZERO).unwrap(),
        GrantDecision::Blocked { .. }
    ));
    rti.acknowledge_message(&fed("a"), WireTag::ZERO).unwrap();
    assert_eq!(
        rti.request_tag(&fed("source"), overflowing).unwrap(),
        GrantDecision::Granted { tag: overflowing }
    );
    let before = snapshot(&rti);

    assert_eq!(
        rti.handle(FederateToRti::Ltc {
            federate_id: fed("source"),
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
fn same_timestamp_microstep_progression_unblocks_downstream_grant() {
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
    let mut rti = new_rti(FederatedTopology::with_edges(
        [fed("a"), fed("b"), fed("c")],
        [
            TopologyEdge::new(fed("a"), fed("b"), endpoint("a.out->b.in"), WireDelay::ZERO),
            TopologyEdge::new(fed("b"), fed("c"), endpoint("b.out->c.in"), WireDelay::ZERO),
        ],
    ));

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
            earliest_incoming: Some(WireTag::ZERO),
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
fn positive_delay_cycle_allows_startup_grants_after_both_federates_advertise() {
    let delay = WireDelay::from_nanos(10);
    let mut rti = new_rti(FederatedTopology::with_edges(
        [fed("a"), fed("b")],
        [
            TopologyEdge::new(fed("a"), fed("b"), endpoint("a.out->b.in"), delay),
            TopologyEdge::new(fed("b"), fed("a"), endpoint("b.out->a.in"), delay),
        ],
    ));

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
                message: RtiToFederate::Tag { tag: WireTag::ZERO },
            },
            RtiDelivery {
                federate_id: fed("a"),
                message: RtiToFederate::Tag { tag: WireTag::ZERO },
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
                tag: WireTag::finite(10, 0),
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
                tag: WireTag::finite(10, 0),
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
    let mut rti = new_rti(FederatedTopology::new([fed("solo")]));

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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));
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
    assert_eq!(
        coordination(&rti, &target).in_transit.get(&tag),
        NonZeroUsize::new(1).as_ref()
    );
}

#[test]
fn topology_delays_shift_earliest_incoming_message_tags() {
    let mut rti = new_rti(topology_with_edge(WireDelay::from_nanos(10)));

    rti.request_tag(&fed("source"), WireTag::ZERO).unwrap();

    assert_eq!(
        rti.earliest_incoming_message_tag(&fed("target")).unwrap(),
        Some(WireTag::finite(10, 0))
    );
    assert_eq!(
        rti.request_tag(&fed("target"), WireTag::finite(9, 0))
            .unwrap(),
        GrantDecision::Granted {
            tag: WireTag::finite(9, 0),
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
    let mut rti = new_rti(topology_with_edge(WireDelay::ZERO));

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
        Some(WireTag::finite(3, 0))
    );
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
