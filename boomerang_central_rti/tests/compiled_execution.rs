//! Exercises the compiled executor through a real image-backed RTI, without Cargo or processes.
use boomerang_runtime::{
    CompiledModeEffectRef, Config, Context, Duration, EnclaveBindings, FederateBindings, InputRef,
    OutputRef, PayloadType, ReactionBindingError, ReactionRefs, ReactorData, Tag,
};

#[path = "compiled_execution/source_sink.rs"]
mod source_sink;
use source_sink::*;

use boomerang_central_rti::compiled::{
    CentralRtiClient, CompiledRti, CoordinationIdentity, RtiClientBindings, RtiReply, RtiRequest,
};
use boomerang_central_rti::WireTag;
use boomerang_federated::conformance::{
    tagged_payload_exchange, Member, Outcome, ReferenceCoordinator, Route, VectorStep,
};
use boomerang_runtime::{execute_owned_federate_with_backend, image::*};
use std::sync::Arc;

#[path = "compiled_execution/transport.rs"]
mod transport;

/// Shared compiler-issued identity for this immutable test deployment.
const IDENTITY: CoordinationIdentity = CoordinationIdentity::new([7; 32]);
/// Stable names in the same canonical order as the Federate and RTI member tables.
static MEMBER_NAMES: [&str; 2] = ["a-source", "b-sink"];
/// Canonical Federates owning separate Enclave slices.
static FEDERATES: [FederateImage; 2] = [
    fixture_federate(MEMBER_NAMES[0], "host", "std", IndexSpan::new(0, 1)),
    fixture_federate(MEMBER_NAMES[1], "host", "std", IndexSpan::new(1, 2)),
];
/// Authoritative federation membership for the fixture.
static MEMBERS: [FederateIndex; 2] = [FederateIndex::new(0), FederateIndex::new(1)];
/// Authoritative logical route, shared by both compiled halves.
static EDGES: [FederationEdgeImage; 1] = [FederationEdgeImage::new(
    BoundaryId::new("pipe"),
    FederateIndex::new(0),
    FederateIndex::new(1),
    1_000_000,
)];
/// Direct and transitive dependency ranges from the canonical fixture analysis.
static DEPENDENCIES: [RtiDependencyImage; 2] =
    [RtiDependencyImage::new(FederateIndex::new(0), 1_000_000); 2];
/// Packed member-owned relationship ranges; no secondary graph analysis.
static RTI_MEMBERS: [RtiMemberImage; 2] = [
    RtiMemberImage::new(
        RecoveryPolicy::FailStop,
        SliceRange::new(0, 0),
        SliceRange::new(0, 0),
        SliceRange::new(0, 1),
        32,
    ),
    RtiMemberImage::new(
        RecoveryPolicy::FailStop,
        SliceRange::new(0, 1),
        SliceRange::new(1, 1),
        SliceRange::new(1, 0),
        32,
    ),
];
/// Immutable concrete route selected by the compiled boundary identity.
static RTI_ROUTES: [RtiRouteImage; 1] = [RtiRouteImage::new(
    BoundaryId::new("pipe"),
    FlowIndex::new(0),
    None,
    None,
    BoundaryFailurePolicy::PropagateStop,
    TransportPolicy::ReliableOrderedFramed,
    CodecPolicy::CanonicalBounded,
    TimingPolicy::BestEffort,
    SecurityPolicy::None,
    TransportCapabilityIndex::new(0),
    CodecCapabilityIndex::new(0),
    FederateIndex::new(0),
    FederateIndex::new(1),
    1_000_000,
)];
/// Receiving Federate also owns an Enclave that never executes a reaction.
static ENCLAVES: [EnclaveImage; 3] = [
    ROUTED_SOURCE_IMAGE,
    ROUTED_SINK_IMAGE,
    EnclaveImage {
        enclave_id: EnclaveId::new("idle"),
        routes: TinyMapView::new(&[]),
        ..ROUTED_SINK_IMAGE
    },
];
/// Both executor slices and the RTI borrow this one validated deployment.
const DEPLOYMENT: CompiledDeploymentImage = CompiledDeploymentImage {
    federation: GlobalFederationImage::new(&MEMBERS, &EDGES),
    federates: TinyMapView::new(&FEDERATES),
    enclaves: TinyMapView::new(&ENCLAVES),
    coordination: CoordinationProjection::CentralRti(RtiImage::new(
        TinyMapView::new(&RTI_MEMBERS),
        &DEPENDENCIES,
        &[FederateIndex::new(1)],
        TinyMapView::new(&RTI_ROUTES),
        TinyMapView::new(&["flow"]),
        TinyMapView::new(&[]),
        TinyMapView::new(&["test-ordered"]),
        TinyMapView::new(&["u32-le"]),
    )),
};

/// Validates the RTI projection after the caller has checked full-deployment coherence.
fn rti_view(view: &CompiledDeploymentView<'static>) -> RtiImageView<'static> {
    let CoordinationProjection::CentralRti(image) = view.coordination() else {
        panic!("fixture must select central-rti")
    };
    RtiImageView::new(image.clone(), IdentityTable::new(&MEMBER_NAMES)).unwrap()
}

/// Runs the real scheduler/backend/RTI path with transport confined to test support.
fn execute_pair(mismatch: bool, fail_rti: bool, fail_scheduler: bool) {
    bounded(move || {
        let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
        let rti = CompiledRti::from_image(rti_view(&view), IDENTITY).unwrap();
        let source_rti = RtiClientBindings::new(&view, MEMBERS[0], IDENTITY).unwrap();
        let sink_rti = RtiClientBindings::new(
            &view,
            MEMBERS[1],
            if mismatch {
                CoordinationIdentity::new([8; 32])
            } else {
                IDENTITY
            },
        )
        .unwrap();
        let (source, sink, server) = transport::start(rti, fail_rti);
        let source_thread = spawn_traced(move || {
            let _member = source_rti.execution_span().entered();
            let outbound = source_rti
                .outbound_sink(source.0.clone(), BoundaryId::new("pipe"))
                .unwrap();
            execute_owned_federate_with_backend(
                MEMBERS[0],
                &FEDERATES[0],
                &ENCLAVES[..1],
                FederateBindings::new()
                    .bind_enclave(EnclaveIndex::new(0), source_bindings())
                    .bind_outbound_route(
                        BoundaryId::new("pipe"),
                        PayloadType::<u32>::new(),
                        move |value: &u32| {
                            if fail_scheduler {
                                Err(std::io::Error::other("injected codec failure"))
                            } else {
                                Ok(value.to_le_bytes().to_vec())
                            }
                        },
                        outbound,
                    ),
                Config::default().with_fast_forward(true),
                |inbound| {
                    CentralRtiClient::connect(
                        source.0,
                        source.1,
                        source_rti,
                        inbound,
                        std::time::Duration::from_secs(2),
                    )
                },
            )
        });
        let sink_thread = spawn_traced(move || {
            let _member = sink_rti.execution_span().entered();
            execute_owned_federate_with_backend(
                MEMBERS[1],
                &FEDERATES[1],
                &ENCLAVES[1..],
                FederateBindings::new()
                    .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                    .bind_enclave(EnclaveIndex::new(2), sink_bindings())
                    .bind_inbound_route(
                        BoundaryId::new("pipe"),
                        PayloadType::<u32>::new(),
                        |bytes: &[u8]| -> Result<u32, std::array::TryFromSliceError> {
                            Ok(u32::from_le_bytes(bytes.try_into()?))
                        },
                    ),
                Config::default().with_fast_forward(true),
                |inbound| {
                    CentralRtiClient::connect(
                        sink.0,
                        sink.1,
                        sink_rti,
                        inbound,
                        std::time::Duration::from_secs(2),
                    )
                },
            )
        });
        let source = source_thread.join().unwrap();
        let sink = sink_thread.join().unwrap();
        server.join().unwrap();
        if fail_scheduler {
            assert!(source
                .unwrap_err()
                .to_string()
                .contains("injected codec failure"));
            assert!(sink
                .unwrap_err()
                .to_string()
                .contains("stopped before global quiescence"));
        } else if mismatch || fail_rti {
            let expected = if mismatch {
                "identity"
            } else {
                "injected RTI failure"
            };
            let source = source.unwrap_err().to_string();
            assert!(source.contains(expected), "source: {source}");
            let sink = sink.unwrap_err().to_string();
            assert!(sink.contains(expected), "sink: {sink}");
        } else {
            assert_eq!(
                source
                    .unwrap()
                    .enclave(EnclaveIndex::new(0))
                    .unwrap()
                    .final_tag(),
                Tag::ZERO
            );
            let sink = sink.unwrap();
            let result = sink.enclave(EnclaveIndex::new(1)).unwrap();
            assert_eq!(result.final_tag(), Tag::new(Duration::milliseconds(1), 0));
            assert_eq!(
                result
                    .state::<RoutedSinkState>(StateSlotIndex::new(0))
                    .unwrap()
                    .values,
                [42]
            );
        }
    });
}

/// Runs separate compiled schedulers through the production RTI state and client.
#[test]
fn compiled_federates_exchange_tagged_payload_through_rti() {
    if !client::run_trace_test() {
        return;
    }
    let (_, events) = client::capture_coordination(|| execute_pair(false, false, false));
    for kind in [
        "coordination.reaction.started",
        "coordination.codec.encoded",
        "coordination.payload.sent",
        "coordination.rti.decision",
        "coordination.rti.payload.forwarded",
        "coordination.rti.grant.issued",
        "coordination.rti.accounting.completed",
        "coordination.shutdown.requested",
        "coordination.shutdown.completed",
        "coordination.rti.idle.issued",
        "coordination.rti.dnet.issued",
        "coordination.payload.received",
        "coordination.boundary.admitted",
        "coordination.reaction.finished",
    ] {
        assert!(
            events.iter().any(|event| event["fields"]["event"] == kind),
            "missing {kind}: {events:#?}"
        );
    }
    let reactions: Vec<_> = events
        .iter()
        .filter(|event| event["fields"]["event"] == "coordination.reaction.started")
        .collect();
    for member in ["FederateIndex(0)", "FederateIndex(1)"] {
        assert!(reactions.iter().any(|event| event["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["federate"] == member
                && span["coordination"] == format!("{IDENTITY:?}"))));
    }
    let position = |kind: &str, member: &str| {
        events
            .iter()
            .position(|event| {
                event["fields"]["event"] == kind
                    && (event["fields"]["federate"] == member
                        || event["spans"].as_array().is_some_and(|spans| {
                            spans.iter().any(|span| span["federate"] == member)
                        }))
            })
            .unwrap_or_else(|| panic!("missing {kind} for {member}"))
    };
    let source = "FederateIndex(0)";
    let sink = "FederateIndex(1)";
    assert!(
        position("coordination.reaction.started", source)
            < position("coordination.codec.encoded", source)
    );
    assert!(
        position("coordination.codec.encoded", source)
            < position("coordination.rti.payload.forwarded", source)
    );
    assert!(
        position("coordination.rti.payload.forwarded", source)
            < position("coordination.boundary.admitted", sink)
    );
    // Admission logs after enqueue, so it need not precede a concurrently woken reaction.
    // The RTI must forward before the destination can execute and retire its accounting.
    assert!(
        position("coordination.rti.payload.forwarded", source)
            < position("coordination.reaction.started", sink)
    );
    assert!(
        position("coordination.reaction.finished", sink)
            < position("coordination.rti.accounting.completed", sink)
    );
    let forwarded = &events[position("coordination.rti.payload.forwarded", source)]["fields"];
    assert_eq!(forwarded["route"], "RtiRouteIndex(0)");
    assert_eq!(forwarded["destination"], sink);
    assert_eq!(forwarded["pending_tags"], 1);
    assert_eq!(forwarded["coordination"], format!("{IDENTITY:?}"));
    assert_eq!(
        events[position("coordination.codec.encoded", source)]["fields"]["local_route"],
        "RouteIndex(0)"
    );
    let sent = &events[position("coordination.payload.sent", source)]["fields"];
    assert_eq!(sent["route"], "RtiRouteIndex(0)");
    assert_eq!(sent["coordination"], format!("{IDENTITY:?}"));
    assert_eq!(
        sent["tag"],
        format!("{:?}", Tag::new(Duration::milliseconds(1), 0))
    );
    assert!(sent.as_object().unwrap().keys().all(|key| matches!(
        key.as_str(),
        "event" | "message" | "coordination" | "federate" | "route" | "tag"
    )));
    let decision = &events[position("coordination.rti.decision", source)]["fields"];
    assert_eq!(decision["request"], "hello");
    assert!(decision["deliveries"].is_u64());
    assert_eq!(
        events[position("coordination.rti.accounting.completed", sink)]["fields"]["pending_tags"],
        0
    );
}
/// Rejects mismatched artifact identities before execution.
#[test]
fn coordination_identity_mismatch_rejects_both_federates() {
    execute_pair(true, false, false);
}
/// Propagates a terminal RTI failure to both blocked executors.
#[test]
fn rti_failure_releases_both_compiled_federates() {
    if !client::run_trace_test() {
        return;
    }
    let (_, events) = client::capture_coordination(|| execute_pair(false, true, false));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["fields"]["event"] == "coordination.rti.failure.first")
            .count(),
        1
    );
}

/// Starts a pure RTI without a transport for protocol-boundary assertions.
fn admitted_rti() -> CompiledRti<'static> {
    let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
    let mut rti = CompiledRti::from_image(rti_view(&view), IDENTITY).unwrap();
    for member in MEMBERS {
        rti.handle(member, RtiRequest::Hello { identity: IDENTITY });
    }
    rti
}

/// Test-only bridge between the portable vector domains and this fixture image.
struct ReferenceVectorAdapter;

impl ReferenceVectorAdapter {
    fn member(member: Member) -> FederateIndex {
        if member == Member::new(0) {
            MEMBERS[0]
        } else if member == Member::new(1) {
            MEMBERS[1]
        } else {
            panic!("vector member is absent from compiled fixture")
        }
    }

    fn vector_member(member: FederateIndex) -> Member {
        if member == MEMBERS[0] {
            Member::new(0)
        } else if member == MEMBERS[1] {
            Member::new(1)
        } else {
            panic!("compiled fixture member is absent from vector")
        }
    }

    fn route(route: Route) -> RtiRouteIndex {
        if route == Route::new(0) {
            RtiRouteIndex::new(0)
        } else {
            panic!("vector route is absent from compiled fixture")
        }
    }

    fn vector_route(route: RtiRouteIndex) -> Route {
        if route == RtiRouteIndex::new(0) {
            Route::new(0)
        } else {
            panic!("compiled fixture route is absent from vector")
        }
    }

    fn request(step: VectorStep) -> (FederateIndex, RtiRequest) {
        match step {
            VectorStep::Publish {
                member,
                revision,
                next_event,
            } => (
                Self::member(member),
                RtiRequest::Publish {
                    revision,
                    next_event,
                },
            ),
            VectorStep::Complete { member, tag } => {
                (Self::member(member), RtiRequest::Complete { tag })
            }
            VectorStep::Payload { member, route, tag } => (
                Self::member(member),
                RtiRequest::Payload {
                    route: Self::route(route),
                    tag,
                    payload: vec![],
                },
            ),
            VectorStep::Stop { .. } => panic!("this NET/DNET/LTC vector does not stop members"),
        }
    }

    fn observation(delivery: boomerang_central_rti::compiled::RtiDelivery) -> VectorObservation {
        let boomerang_central_rti::compiled::RtiDelivery { member, reply } = delivery;
        match reply {
            RtiReply::Grant { revision, tag } => VectorObservation::Semantic(Outcome::grant(
                Self::vector_member(member),
                revision,
                tag,
            )),
            RtiReply::Payload { route, tag, .. } => VectorObservation::Semantic(Outcome::payload(
                Self::vector_member(member),
                Self::vector_route(route),
                tag,
            )),
            RtiReply::Stopped => VectorObservation::Semantic(Outcome::Stopped {
                member: Self::vector_member(member),
            }),
            RtiReply::Started => VectorObservation::Started {
                member: Self::vector_member(member),
            },
            RtiReply::Idle { revision } => VectorObservation::Idle {
                member: Self::vector_member(member),
                revision,
            },
            RtiReply::Failed { message } => VectorObservation::Failed {
                member: Self::vector_member(member),
                message,
            },
            RtiReply::SuppressPublication { tag } => VectorObservation::Suppress {
                member: Self::vector_member(member),
                tag,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum VectorObservation {
    Semantic(Outcome),
    Started { member: Member },
    Idle { member: Member, revision: u64 },
    Failed { member: Member, message: String },
    Suppress { member: Member, tag: WireTag },
}

struct VectorExecution {
    observations: Vec<VectorObservation>,
    semantic_outcomes: Vec<Outcome>,
    dnet_controls: usize,
}

fn run_reference_vector(vector: impl IntoIterator<Item = VectorStep>) -> VectorExecution {
    let mut rti = admitted_rti();
    let mut observations = Vec::new();
    let mut semantic_outcomes = Vec::new();
    let mut dnet_controls = 0;
    for step in vector {
        let (member, request) = ReferenceVectorAdapter::request(step);
        for delivery in rti.handle(member, request) {
            let observation = ReferenceVectorAdapter::observation(delivery);
            dnet_controls += usize::from(matches!(observation, VectorObservation::Suppress { .. }));
            if let VectorObservation::Semantic(outcome) = &observation {
                semantic_outcomes.push(*outcome);
            }
            observations.push(observation);
        }
    }
    VectorExecution {
        observations,
        semantic_outcomes,
        dnet_controls,
    }
}

/// Checks one hand-authored NET/DNET/LTC exchange against the portable oracle.
#[test]
fn compiled_rti_outcomes_are_permitted_by_the_reference_vector() {
    let exchange = tagged_payload_exchange();
    let (source, destination, route, tag, members, topology, vector, expected) = (
        exchange.source,
        exchange.destination,
        exchange.route,
        exchange.tag,
        exchange.members,
        exchange.topology,
        exchange.steps,
        exchange.expected_outcomes,
    );
    let mut oracle = ReferenceCoordinator::new(members, topology, 1).unwrap();
    let oracle_outcomes = vector
        .into_iter()
        .flat_map(|step| oracle.apply(step))
        .collect::<Vec<_>>();
    assert_eq!(oracle_outcomes, expected);

    let execution = run_reference_vector(vector);
    let expected_observations = [
        VectorObservation::Semantic(Outcome::grant(source, 0, WireTag::FOREVER)),
        VectorObservation::Suppress {
            member: source,
            tag: WireTag::finite(0, u64::MAX),
        },
        VectorObservation::Suppress {
            member: destination,
            tag: WireTag::FOREVER,
        },
        VectorObservation::Semantic(Outcome::payload(destination, route, tag)),
        VectorObservation::Semantic(Outcome::grant(destination, 0, WireTag::FOREVER)),
    ];
    assert_eq!(execution.observations, expected_observations);
    assert_eq!(execution.semantic_outcomes, expected);
    assert_eq!(execution.dnet_controls, 2);
}

/// Rejects payload submission before the delayed source completion frontier.
#[test]
fn completed_source_cannot_emit_earlier_payload() {
    let mut rti = admitted_rti();
    publish(&mut rti, MEMBERS[0], 0, Some(WireTag::ZERO));
    rti.handle(
        MEMBERS[0],
        RtiRequest::Complete {
            tag: WireTag::finite(1, 0),
        },
    );
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::finite(1_000_000, 0),
            payload: vec![42],
        },
    );
    assert!(replies
        .iter()
        .all(|delivery| matches!(delivery.reply, RtiReply::Failed { .. })));
    assert!(rti.is_finished());
}

/// Keeps unknown upstream state and undrained payloads conservative.
#[test]
fn unknown_upstream_blocks_and_in_transit_payload_prevents_idle() {
    let mut rti = admitted_rti();
    let destination = WireTag::finite(1_000_000, 0);
    assert!(rti
        .handle(
            MEMBERS[1],
            RtiRequest::Publish {
                revision: 0,
                next_event: Some(destination)
            }
        )
        .iter()
        .all(|d| matches!(d.reply, RtiReply::SuppressPublication { .. })));
    let replies = publish(&mut rti, MEMBERS[0], 0, Some(WireTag::ZERO));
    assert!(
        matches!(replies.as_slice(), [delivery] if delivery.member == MEMBERS[0] && matches!(delivery.reply, RtiReply::Grant { .. }))
    );
    publish(&mut rti, MEMBERS[1], 1, None);
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: destination,
            payload: vec![42],
        },
    );
    assert!(
        matches!(replies.first(), Some(delivery) if delivery.member == MEMBERS[1] && matches!(delivery.reply, RtiReply::Payload { .. }))
    );
    rti.handle(MEMBERS[0], RtiRequest::Complete { tag: WireTag::ZERO });
    assert!(rti
        .handle(
            MEMBERS[0],
            RtiRequest::Publish {
                revision: 1,
                next_event: None
            }
        )
        .iter()
        .all(|d| matches!(d.reply, RtiReply::SuppressPublication { .. })));
    let replies = publish(&mut rti, MEMBERS[1], 2, Some(destination));
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { revision: 2, tag } if tag == WireTag::FOREVER))
    );
    rti.handle(MEMBERS[1], RtiRequest::Complete { tag: destination });
    publish(&mut rti, MEMBERS[1], 3, None);
    rti.handle(MEMBERS[0], RtiRequest::ConfirmIdle { revision: 1 });
    let replies = rti.handle(MEMBERS[1], RtiRequest::ConfirmIdle { revision: 3 });
    assert_eq!(
        replies
            .iter()
            .filter(|delivery| matches!(delivery.reply, RtiReply::Idle { .. }))
            .count(),
        2
    );
}

/// Preserves wakeable idle for members that have not requested termination.
#[test]
fn local_idle_is_reversible_without_terminal_participation() {
    use boomerang_central_rti::{compiled::RtiReply, WireTag};
    let mut rti = admitted_rti();
    for member in MEMBERS {
        let replies = publish(&mut rti, member, 0, None);
        assert!(
            replies.is_empty(),
            "local idle must not imply terminal participation"
        );
    }
    let replies = publish(&mut rti, MEMBERS[0], 1, Some(WireTag::ZERO));
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { .. }))
    );
}

/// Keeps positive-delay microstep collapse from granting a destination too early.
#[test]
fn positive_delay_completion_does_not_cover_later_source_microsteps() {
    let mut rti = admitted_rti();
    let destination = WireTag::finite(1_000_000, 0);
    publish(&mut rti, MEMBERS[0], 0, Some(WireTag::ZERO));
    rti.handle(MEMBERS[0], RtiRequest::Complete { tag: WireTag::ZERO });
    let replies = publish(&mut rti, MEMBERS[1], 0, Some(destination));
    assert!(
        replies.is_empty(),
        "a later source microstep can still reach the requested destination tag"
    );
    publish(&mut rti, MEMBERS[0], 1, Some(WireTag::finite(0, 1)));
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: destination,
            payload: vec![42],
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Payload { .. }))
    );
    rti.handle(
        MEMBERS[0],
        RtiRequest::Complete {
            tag: WireTag::finite(0, 1),
        },
    );
    let replies = publish(&mut rti, MEMBERS[0], 2, None);
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { tag, .. } if tag == WireTag::FOREVER))
    );
}

/// Aborts the blocked receiver after a local encoder failure.
#[test]
fn scheduler_failure_releases_blocked_peer() {
    if !client::run_trace_test() {
        return;
    }
    let (_, events) = client::capture_coordination(|| execute_pair(false, false, true));
    assert!(events
        .iter()
        .any(|event| event["fields"]["event"] == "coordination.reaction.cancelled"));
    assert!(!events
        .iter()
        .any(|event| event["fields"]["event"] == "coordination.codec.encoded"));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("injected codec failure"));
}

/// Preserves authority when local work is discovered below an existing grant.
#[test]
fn revised_candidates_preserve_grant_horizons_for_payloads_and_completion() {
    let mut rti = admitted_rti();
    publish(&mut rti, MEMBERS[0], 0, Some(WireTag::finite(10, 0)));
    // An already queued grant can cover earlier work discovered inside the Federate.
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::finite(1_000_005, 0),
            payload: vec![42],
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Payload { .. }))
    );
    rti.handle(
        MEMBERS[0],
        RtiRequest::Complete {
            tag: WireTag::finite(5, 0),
        },
    );
    publish(&mut rti, MEMBERS[0], 1, Some(WireTag::finite(6, 0)));
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Complete {
            tag: WireTag::finite(10, 0),
        },
    );
    assert!(!rti.is_finished());
    assert!(replies.is_empty());
}

/// Publishes a fixture candidate through the public RTI protocol boundary.
fn publish(
    rti: &mut CompiledRti<'_>,
    member: FederateIndex,
    revision: u64,
    next_event: Option<boomerang_central_rti::WireTag>,
) -> Vec<boomerang_central_rti::compiled::RtiDelivery> {
    rti.handle(
        member,
        boomerang_central_rti::compiled::RtiRequest::Publish {
            revision,
            next_event,
        },
    )
    .into_iter()
    .filter(|delivery| !matches!(delivery.reply, RtiReply::SuppressPublication { .. }))
    .collect()
}

/// Preflight authorizes stable outbound identities and retains only the shared route key.
#[test]
fn outbound_preflight_resolves_and_authorizes_typed_routes() {
    use boomerang_central_rti::compiled::in_memory::InMemorySender;
    let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
    assert!(RtiClientBindings::new(&view, FederateIndex::new(9), IDENTITY).is_err());
    assert!(
        RtiClientBindings::from_image(rti_view(&view), FederateIndex::new(9), IDENTITY).is_err()
    );
    let source = RtiClientBindings::from_image(rti_view(&view), MEMBERS[0], IDENTITY).unwrap();
    let target = RtiClientBindings::from_image(rti_view(&view), MEMBERS[1], IDENTITY).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = Arc::new(InMemorySender::new(MEMBERS[0], tx));
    assert!(source
        .outbound_sink(sender.clone(), BoundaryId::new("missing"))
        .is_err());
    assert!(target
        .outbound_sink(sender.clone(), BoundaryId::new("pipe"))
        .is_err());
    source
        .outbound_sink(sender, BoundaryId::new("pipe"))
        .unwrap()
        .send(boomerang_runtime::TaggedPayload {
            tag: Tag::ZERO,
            payload: vec![42],
        })
        .unwrap();
    assert!(
        matches!(rx.recv().unwrap(), (member, RtiRequest::Payload { route, tag: WireTag::ZERO, payload })
        if member == MEMBERS[0] && route == RtiRouteIndex::new(0) && payload == [42])
    );
}

/// Missing destination bindings fail before admission and tell the connection owner to abort.
#[test]
fn inbound_preflight_requires_complete_member_bindings() {
    use boomerang_central_rti::compiled::in_memory::{InMemoryReceiver, InMemorySender};
    let bindings = {
        let image = DEPLOYMENT;
        let view = CompiledDeploymentView::new(image).unwrap();
        RtiClientBindings::new(&view, MEMBERS[1], IDENTITY).unwrap()
    };
    let (tx, requests) = std::sync::mpsc::channel();
    let (_replies, rx) = std::sync::mpsc::channel();
    let error = CentralRtiClient::connect(
        Arc::new(InMemorySender::new(MEMBERS[1], tx)),
        InMemoryReceiver::new(rx),
        bindings,
        std::collections::BTreeMap::new(),
        std::time::Duration::ZERO,
    )
    .err()
    .unwrap();
    assert!(error
        .to_string()
        .contains("missing inbound boundary adapter"));
    assert!(matches!(
        requests.try_recv().unwrap().1,
        RtiRequest::Abort { .. }
    ));
    assert!(requests.try_recv().is_err());
}

/// Dense route keys are range-checked and authorized against the authenticated source binding.
#[test]
fn payload_rejects_unknown_and_foreign_route_keys() {
    for (member, route, message) in [
        (MEMBERS[0], RtiRouteIndex::new(9), "unknown RTI route key"),
        (
            MEMBERS[1],
            RtiRouteIndex::new(0),
            "payload route belongs to another source",
        ),
    ] {
        let mut rti = admitted_rti();
        let replies = rti.handle(
            member,
            RtiRequest::Payload {
                route,
                tag: WireTag::ZERO,
                payload: vec![42],
            },
        );
        assert!(replies.iter().all(|reply| matches!(&reply.reply, RtiReply::Failed { message: actual } if actual.contains(message))), "expected {message}, received {replies:?}");
        assert_eq!(replies.len(), MEMBERS.len());
        assert!(rti.is_finished());
    }
}

/// Extra or foreign inbound adapters are rejected through the executor's real preflight seam.
#[test]
fn inbound_preflight_rejects_extra_and_foreign_bindings() {
    use boomerang_central_rti::compiled::in_memory::{InMemoryReceiver, InMemorySender};
    for extra in [false, true] {
        let view = CompiledDeploymentView::new(DEPLOYMENT).unwrap();
        let bindings =
            RtiClientBindings::new(&view, if extra { MEMBERS[1] } else { MEMBERS[0] }, IDENTITY)
                .unwrap();
        let (tx, requests) = std::sync::mpsc::channel();
        let (_replies, rx) = std::sync::mpsc::channel();
        let error = execute_owned_federate_with_backend(
            MEMBERS[1],
            &FEDERATES[1],
            &ENCLAVES[1..],
            FederateBindings::new()
                .bind_enclave(EnclaveIndex::new(1), sink_bindings())
                .bind_enclave(EnclaveIndex::new(2), sink_bindings())
                .bind_inbound_route(
                    BoundaryId::new("pipe"),
                    PayloadType::<u32>::new(),
                    |bytes: &[u8]| -> Result<u32, std::array::TryFromSliceError> {
                        Ok(u32::from_le_bytes(bytes.try_into()?))
                    },
                ),
            Config::default().with_fast_forward(true),
            |mut inbound| {
                if extra {
                    let adapter = inbound.values().next().unwrap().clone();
                    inbound.insert(BoundaryId::new("extra"), adapter);
                }
                CentralRtiClient::connect(
                    Arc::new(InMemorySender::new(MEMBERS[1], tx)),
                    InMemoryReceiver::new(rx),
                    bindings,
                    inbound,
                    std::time::Duration::ZERO,
                )
            },
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown inbound boundary for member"));
        assert!(matches!(
            requests.try_recv().unwrap().1,
            RtiRequest::Abort { .. }
        ));
        assert!(requests.try_recv().is_err());
    }
}

/// Bounds deadlock detection to one second outside Miri and 30 seconds under Miri, whose
/// interpreter overhead would otherwise cause false watchdog failures.
fn owned_federate_watchdog_timeout() -> std::time::Duration {
    #[cfg(miri)]
    {
        std::time::Duration::from_secs(30)
    }

    #[cfg(not(miri))]
    {
        std::time::Duration::from_secs(1)
    }
}

fn bounded<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn_traced(move || tx.send(run()).unwrap());
    let result = rx
        .recv_timeout(owned_federate_watchdog_timeout())
        .expect("owned Federate execution must complete within the watchdog timeout");
    worker.join().unwrap();
    result
}

fn spawn_traced<T: Send + 'static>(
    run: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    std::thread::spawn(move || tracing::dispatcher::with_default(&dispatch, run))
}

#[path = "compiled_execution/client.rs"]
mod client;
