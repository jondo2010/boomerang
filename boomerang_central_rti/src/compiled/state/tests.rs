//! Behavioral grant checks over precomputed images; no legacy topology oracle.
use super::*;
use boomerang_runtime::image::{
    BoundaryId, CodecCapabilityIndex, CodecPolicy, FlowIndex, IdentityTable, RtiDependencyImage,
    RtiImage, RtiMemberImage, RtiRouteImage, SliceRange, TinyMapView, TransportCapabilityIndex,
    TransportPolicy,
};

const A: FederateIndex = FederateIndex::new(0);
const B: FederateIndex = FederateIndex::new(1);
const C: FederateIndex = FederateIndex::new(2);
const IDENTITY: CoordinationIdentity = CoordinationIdentity::new([3; 32]);

const fn route(
    name: &'static str,
    source: FederateIndex,
    target: FederateIndex,
    delay: u64,
) -> RtiRouteImage<'static> {
    RtiRouteImage::new(
        BoundaryId::new(name),
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
        source,
        target,
        delay,
    )
}

// A -> B -> C, with all direct and transitive bounds embedded in member ranges.
static CHAIN: RtiImage<'static> = RtiImage::new(
    TinyMapView::new(&[
        RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 2),
            2,
        ),
        RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 1),
            SliceRange::new(1, 1),
            SliceRange::new(2, 1),
            2,
        ),
        RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(2, 1),
            SliceRange::new(3, 2),
            SliceRange::new(3, 0),
            2,
        ),
    ]),
    &[
        RtiDependencyImage::new(A, 0),
        RtiDependencyImage::new(A, 0),
        RtiDependencyImage::new(B, 0),
        RtiDependencyImage::new(A, 0),
        RtiDependencyImage::new(B, 0),
    ],
    &[B, C, C],
    TinyMapView::new(&[route("a-b", A, B, 0), route("b-c", B, C, 0)]),
    IdentityTable::new(&["flow"]),
    IdentityTable::new(&[]),
    IdentityTable::new(&["ordered"]),
    IdentityTable::new(&["bytes"]),
);

fn admitted(
    image: &'static RtiImage<'static>,
    identities: &'static [&'static str],
) -> CompiledRti<'static> {
    let view = RtiImageView::new(image.clone(), IdentityTable::new(identities)).unwrap();
    let mut rti = CompiledRti::from_image(view, IDENTITY).unwrap();
    for member in image.members().keys() {
        let replies = rti.handle(member, RtiRequest::Hello { identity: IDENTITY });
        assert!(replies
            .iter()
            .all(|delivery| matches!(delivery.reply, RtiReply::Started)));
    }
    rti
}

fn publish(
    rti: &mut CompiledRti<'_>,
    member: FederateIndex,
    revision: u64,
    tag: Option<WireTag>,
) -> Vec<RtiDelivery> {
    rti.handle(
        member,
        RtiRequest::Publish {
            revision,
            next_event: tag,
        },
    )
}

#[test]
fn coordinator_outlives_local_image_descriptor() {
    let names = [String::from("a"), String::from("b"), String::from("c")];
    let identities = names.each_ref().map(String::as_str);
    let mut rti = {
        let image = CHAIN.clone();
        let view = RtiImageView::new(image, IdentityTable::new(&identities)).unwrap();
        CompiledRti::from_image(view, IDENTITY).unwrap()
    };

    assert_eq!(rti.resolve_member("b").unwrap(), B);
    for member in [A, B, C] {
        let replies = rti.handle(member, RtiRequest::Hello { identity: IDENTITY });
        assert!(replies
            .iter()
            .all(|delivery| matches!(delivery.reply, RtiReply::Started)));
    }
    assert_eq!(rti.member_count(), 3);
}

fn grants(deliveries: Vec<RtiDelivery>) -> Vec<(FederateIndex, WireTag)> {
    deliveries
        .into_iter()
        .map(|delivery| match delivery.reply {
            RtiReply::Grant { tag, .. } => (delivery.member, tag),
            other => panic!("unexpected reply: {other:?}"),
        })
        .collect()
}

#[test]
fn transitive_source_blocks_grant_through_later_intermediate_publication() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    let early = WireTag::finite(50, 0);
    let middle = WireTag::finite(100, 0);
    let target = WireTag::finite(99, 0);
    assert_eq!(
        grants(publish(&mut rti, A, 1, Some(early))),
        [(A, WireTag::FOREVER)]
    );
    assert!(publish(&mut rti, B, 1, Some(middle)).is_empty());
    assert!(publish(&mut rti, C, 1, Some(target)).is_empty());
    // A's new lower bound reaches C through the compiled affected-downstream table.
    let advanced = WireTag::finite(101, 0);
    assert_eq!(
        grants(publish(&mut rti, A, 2, Some(advanced))),
        [
            (A, WireTag::FOREVER),
            (B, WireTag::finite(100, u64::MAX)),
            (C, WireTag::finite(99, u64::MAX))
        ]
    );
}

#[test]
fn chain_waits_for_each_upstream_to_advance_past_requested_tag() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    assert!(publish(&mut rti, B, 1, Some(WireTag::ZERO)).is_empty());
    assert!(publish(&mut rti, C, 1, Some(WireTag::ZERO)).is_empty());
    assert_eq!(
        grants(publish(&mut rti, A, 1, Some(WireTag::ZERO))),
        [(A, WireTag::FOREVER)]
    );
    let next = WireTag::finite(0, 1);
    assert_eq!(
        grants(publish(&mut rti, A, 2, Some(next))),
        [(A, WireTag::FOREVER), (B, WireTag::ZERO)]
    );
    assert_eq!(
        grants(publish(&mut rti, A, 3, Some(WireTag::finite(0, 2)))),
        [(A, WireTag::FOREVER), (B, WireTag::finite(0, 1))]
    );
    assert_eq!(
        grants(publish(&mut rti, B, 2, Some(next))),
        [(B, next), (C, WireTag::ZERO)]
    );
}

#[test]
fn completion_clears_in_transit_tag_and_reconsiders_downstream() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    let pending = WireTag::finite(0, 5);
    assert_eq!(
        grants(publish(&mut rti, A, 1, Some(pending))),
        [(A, WireTag::FOREVER)]
    );
    let payloads = rti.handle(
        A,
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: pending,
            payload: vec![42],
        },
    );
    assert!(matches!(
        payloads.as_slice(),
        [RtiDelivery {
            member: B,
            reply: RtiReply::Payload { .. }
        }]
    ));
    assert!(rti
        .handle(A, RtiRequest::Complete { tag: pending })
        .is_empty());
    assert!(publish(&mut rti, A, 2, None).is_empty());
    // B advertises later local work while the earlier delivery is still in transit.
    assert_eq!(
        grants(publish(&mut rti, B, 1, Some(WireTag::finite(0, 10)))),
        [(B, WireTag::FOREVER)]
    );
    let target = WireTag::finite(0, 9);
    assert!(publish(&mut rti, C, 1, Some(target)).is_empty());
    assert_eq!(
        grants(rti.handle(B, RtiRequest::Complete { tag: pending })),
        [(C, WireTag::finite(0, 9))]
    );
}

#[test]
fn positive_delay_cycle_starts_after_both_members_publish() {
    static CYCLE: RtiImage<'static> = RtiImage::new(
        TinyMapView::new(&[
            RtiMemberImage::new(
                RecoveryPolicy::FailStop,
                SliceRange::new(0, 1),
                SliceRange::new(1, 2),
                SliceRange::new(0, 2),
                2,
            ),
            RtiMemberImage::new(
                RecoveryPolicy::FailStop,
                SliceRange::new(3, 1),
                SliceRange::new(4, 2),
                SliceRange::new(2, 2),
                2,
            ),
        ]),
        &[
            RtiDependencyImage::new(B, 10),
            RtiDependencyImage::new(A, 20),
            RtiDependencyImage::new(B, 10),
            RtiDependencyImage::new(A, 10),
            RtiDependencyImage::new(A, 10),
            RtiDependencyImage::new(B, 20),
        ],
        &[A, B, A, B],
        TinyMapView::new(&[route("a-b", A, B, 10), route("b-a", B, A, 10)]),
        IdentityTable::new(&["flow"]),
        IdentityTable::new(&[]),
        IdentityTable::new(&["ordered"]),
        IdentityTable::new(&["bytes"]),
    );
    let mut rti = admitted(&CYCLE, &["a", "b"]);
    assert!(publish(&mut rti, A, 1, Some(WireTag::ZERO)).is_empty());
    assert_eq!(
        grants(publish(&mut rti, B, 1, Some(WireTag::ZERO))),
        [
            (B, WireTag::finite(9, u64::MAX)),
            (A, WireTag::finite(9, u64::MAX))
        ]
    );
}

#[test]
fn grant_covers_the_safe_horizon_before_the_next_possible_input() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    assert_eq!(
        grants(publish(&mut rti, A, 1, Some(WireTag::finite(50, 0)))),
        [(A, WireTag::FOREVER)]
    );
    assert_eq!(
        grants(publish(&mut rti, B, 1, Some(WireTag::finite(10, 0)))),
        [(B, WireTag::finite(49, u64::MAX))]
    );
}

#[test]
fn incoming_tag_capacity_coalesces_equal_tags_and_fails_before_forwarding() {
    // The destination's compiled resource contract reserves two incoming tags.
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    publish(&mut rti, A, 1, Some(WireTag::finite(100, 0)));
    for nanos in [1, 1, 2] {
        let replies = rti.handle(
            A,
            RtiRequest::Payload {
                route: RtiRouteIndex::new(0),
                tag: WireTag::finite(nanos, 0),
                payload: vec![42],
            },
        );
        assert!(matches!(
            replies.as_slice(),
            [RtiDelivery {
                member: B,
                reply: RtiReply::Payload { .. }
            }]
        ));
    }
    let replies = rti.handle(
        A,
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::finite(3, 0),
            payload: vec![43],
        },
    );
    assert_eq!(replies.len(), 3);
    assert!(replies.iter().all(|reply| matches!(&reply.reply, RtiReply::Failed { message } if message.contains("in-transit tag capacity"))));
    let first_failure = match &replies[0].reply {
        RtiReply::Failed { message } => message.clone(),
        _ => unreachable!(),
    };
    let later = rti.handle(
        A,
        RtiRequest::Complete {
            tag: WireTag::finite(100, 0),
        },
    );
    assert!(
        matches!(later.as_slice(), [RtiDelivery { reply: RtiReply::Failed { message }, .. }] if message == &first_failure)
    );
}

#[test]
fn cumulative_completion_releases_bounded_accounting_for_later_tags() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    publish(&mut rti, A, 1, Some(WireTag::finite(100, 0)));
    publish(&mut rti, B, 1, Some(WireTag::finite(1, 0)));
    // Insert out of order so retirement must use tag order, not arrival order.
    for nanos in [2, 1] {
        let replies = rti.handle(
            A,
            RtiRequest::Payload {
                route: RtiRouteIndex::new(0),
                tag: WireTag::finite(nanos, 0),
                payload: vec![42],
            },
        );
        assert!(matches!(
            replies.as_slice(),
            [RtiDelivery {
                member: B,
                reply: RtiReply::Payload { .. }
            }]
        ));
    }
    rti.handle(
        B,
        RtiRequest::Complete {
            tag: WireTag::finite(1, 0),
        },
    );
    let replies = rti.handle(
        A,
        RtiRequest::Payload {
            route: RtiRouteIndex::new(0),
            tag: WireTag::finite(3, 0),
            payload: vec![43],
        },
    );
    assert!(matches!(
        replies.as_slice(),
        [RtiDelivery {
            member: B,
            reply: RtiReply::Payload { .. }
        }]
    ));
    // Tag 2 still blocks C, even though B advertises much later local work.
    publish(&mut rti, B, 2, Some(WireTag::finite(50, 0)));
    assert!(publish(&mut rti, C, 1, Some(WireTag::finite(2, 0))).is_empty());
    assert_eq!(
        grants(rti.handle(
            B,
            RtiRequest::Complete {
                tag: WireTag::finite(2, 0)
            }
        )),
        [(C, WireTag::finite(2, u64::MAX))]
    );
}

#[test]
fn covered_revisions_are_answered_and_current_horizons_extend_once() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    publish(&mut rti, A, 1, Some(WireTag::finite(50, 0)));
    let horizon = WireTag::finite(49, u64::MAX);
    assert_eq!(
        grants(publish(&mut rti, B, 1, Some(WireTag::finite(10, 0)))),
        [(B, horizon)]
    );
    assert_eq!(
        grants(publish(&mut rti, B, 2, Some(WireTag::finite(20, 0)))),
        [(B, horizon)]
    );
    assert!(publish(&mut rti, B, 2, Some(WireTag::finite(20, 0))).is_empty());
    let replies = publish(&mut rti, A, 2, Some(WireTag::finite(60, 0)));
    assert!(replies.iter().any(|delivery| matches!(delivery,
        RtiDelivery { member: B, reply: RtiReply::Grant { revision: 2, tag } }
            if *tag == WireTag::finite(59, u64::MAX))));
    assert!(publish(&mut rti, A, 2, Some(WireTag::finite(60, 0))).is_empty());
}
