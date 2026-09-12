//! Exercises the compiled executor through a real image-backed RTI, without Cargo or processes.
use super::*;
use boomerang::central_rti::compiled::{CentralRtiClient, CompiledRti, CoordinationIdentity};
use boomerang::runtime::{execute_owned_federate_with_backend, image::*};
use std::sync::Arc;

#[path = "central_rti_transport.rs"]
mod transport;

/// Shared compiler-issued identity for this immutable test deployment.
const IDENTITY: CoordinationIdentity = CoordinationIdentity::new([7; 32]);
/// Canonical Federates owning separate Enclave slices.
static FEDERATES: [FederateImage; 2] = [
    fixture_federate("a-source", "host", "std", IndexSpan::new(0, 1)),
    fixture_federate("b-sink", "host", "std", IndexSpan::new(1, 2)),
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
    ),
    RtiMemberImage::new(
        RecoveryPolicy::FailStop,
        SliceRange::new(0, 1),
        SliceRange::new(1, 1),
        SliceRange::new(1, 0),
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
static DEPLOYMENT: CompiledDeploymentImage = CompiledDeploymentImage {
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

/// Runs the real scheduler/backend/RTI path with transport confined to test support.
fn execute_pair(mismatch: bool, fail_rti: bool, fail_scheduler: bool) {
    bounded(move || {
        let view = CompiledDeploymentView::new(&DEPLOYMENT).unwrap();
        let rti = CompiledRti::new(&view, IDENTITY).unwrap();
        let (source, sink, server) = transport::start(rti, fail_rti);
        let source_thread = std::thread::spawn(move || {
            let outbound =
                CentralRtiClient::outbound_sink(source.0.clone(), BoundaryId::new("pipe"));
            execute_owned_federate_with_backend(
                MEMBERS[0],
                FEDERATES[0],
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
                        IDENTITY,
                        inbound,
                        std::time::Duration::from_secs(2),
                    )
                },
            )
        });
        let sink_thread = std::thread::spawn(move || {
            execute_owned_federate_with_backend(
                MEMBERS[1],
                FEDERATES[1],
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
                        if mismatch {
                            CoordinationIdentity::new([8; 32])
                        } else {
                            IDENTITY
                        },
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
    execute_pair(false, false, false);
}
/// Rejects mismatched artifact identities before execution.
#[test]
fn coordination_identity_mismatch_rejects_both_federates() {
    execute_pair(true, false, false);
}
/// Propagates a terminal RTI failure to both blocked executors.
#[test]
fn rti_failure_releases_both_compiled_federates() {
    execute_pair(false, true, false);
}

/// Starts a pure RTI without a transport for protocol-boundary assertions.
fn admitted_rti() -> CompiledRti<'static> {
    use boomerang::central_rti::compiled::RtiRequest;
    let view = CompiledDeploymentView::new(&DEPLOYMENT).unwrap();
    let mut rti = CompiledRti::new(&view, IDENTITY).unwrap();
    for member in MEMBERS {
        rti.handle(member, RtiRequest::Hello { identity: IDENTITY });
    }
    rti
}

/// Rejects payload submission after the source exhausted its granted horizon.
#[test]
fn completed_source_cannot_emit_late_payload() {
    use boomerang::central_rti::{
        compiled::{RtiReply, RtiRequest},
        WireTag,
    };
    let mut rti = admitted_rti();
    rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 0,
            next_event: Some(WireTag::ZERO),
        },
    );
    rti.handle(MEMBERS[0], RtiRequest::Complete { tag: WireTag::ZERO });
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            boundary: "pipe".into(),
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
    use boomerang::central_rti::{
        compiled::{RtiReply, RtiRequest},
        WireTag,
    };
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
        .is_empty());
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 0,
            next_event: Some(WireTag::ZERO),
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if delivery.member == MEMBERS[0] && matches!(delivery.reply, RtiReply::Grant { .. }))
    );
    rti.handle(
        MEMBERS[1],
        RtiRequest::Publish {
            revision: 1,
            next_event: None,
        },
    );
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            boundary: "pipe".into(),
            tag: destination,
            payload: vec![42],
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if delivery.member == MEMBERS[1] && matches!(delivery.reply, RtiReply::Payload { .. }))
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
        .is_empty());
    let replies = rti.handle(
        MEMBERS[1],
        RtiRequest::Publish {
            revision: 2,
            next_event: Some(destination),
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { revision: 2, tag } if tag == destination))
    );
    rti.handle(MEMBERS[1], RtiRequest::Complete { tag: destination });
    rti.handle(
        MEMBERS[1],
        RtiRequest::Publish {
            revision: 3,
            next_event: None,
        },
    );
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
    use boomerang::central_rti::{
        compiled::{RtiReply, RtiRequest},
        WireTag,
    };
    let mut rti = admitted_rti();
    for member in MEMBERS {
        let replies = rti.handle(
            member,
            RtiRequest::Publish {
                revision: 0,
                next_event: None,
            },
        );
        assert!(
            replies.is_empty(),
            "local idle must not imply terminal participation"
        );
    }
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 1,
            next_event: Some(WireTag::ZERO),
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { .. }))
    );
}

/// Keeps positive-delay microstep collapse from granting a destination too early.
#[test]
fn positive_delay_completion_does_not_cover_later_source_microsteps() {
    use boomerang::central_rti::{
        compiled::{RtiReply, RtiRequest},
        WireTag,
    };
    let mut rti = admitted_rti();
    let destination = WireTag::finite(1_000_000, 0);
    rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 0,
            next_event: Some(WireTag::ZERO),
        },
    );
    rti.handle(MEMBERS[0], RtiRequest::Complete { tag: WireTag::ZERO });
    let replies = rti.handle(
        MEMBERS[1],
        RtiRequest::Publish {
            revision: 0,
            next_event: Some(destination),
        },
    );
    assert!(
        replies.is_empty(),
        "a later source microstep can still reach the requested destination tag"
    );
    rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 1,
            next_event: Some(WireTag::finite(0, 1)),
        },
    );
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            boundary: "pipe".into(),
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
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 2,
            next_event: None,
        },
    );
    assert!(
        matches!(replies.as_slice(), [delivery] if matches!(delivery.reply, RtiReply::Grant { tag, .. } if tag == destination))
    );
}

/// Aborts the blocked receiver after a local encoder failure.
#[test]
fn scheduler_failure_releases_blocked_peer() {
    execute_pair(false, false, true);
}

/// Preserves authority when local work is discovered below an existing grant.
#[test]
fn revised_candidates_preserve_grant_horizons_for_payloads_and_completion() {
    use boomerang::central_rti::{
        compiled::{RtiReply, RtiRequest},
        WireTag,
    };
    let mut rti = admitted_rti();
    rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 0,
            next_event: Some(WireTag::finite(10, 0)),
        },
    );
    // An already queued grant can cover earlier work discovered inside the Federate.
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Payload {
            boundary: "pipe".into(),
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
    rti.handle(
        MEMBERS[0],
        RtiRequest::Publish {
            revision: 1,
            next_event: Some(WireTag::finite(6, 0)),
        },
    );
    let replies = rti.handle(
        MEMBERS[0],
        RtiRequest::Complete {
            tag: WireTag::finite(10, 0),
        },
    );
    assert!(!rti.is_finished());
    assert!(replies.is_empty());
}
