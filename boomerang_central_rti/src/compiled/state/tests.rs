//! Behavioral grant checks over precomputed images; no legacy topology oracle.
use super::*;
use boomerang_runtime::image::{
    BoundaryId, CodecCapabilityIndex, CodecPolicy, FlowIndex, RtiDependencyImage, RtiMemberImage,
    RtiRouteImage, SliceRange, TinyMapView, TransportCapabilityIndex, TransportPolicy,
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
        ),
        RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 1),
            SliceRange::new(1, 1),
            SliceRange::new(2, 1),
        ),
        RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(2, 1),
            SliceRange::new(3, 2),
            SliceRange::new(3, 0),
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
    let view = RtiImageView::new(image, IdentityTable::new(identities)).unwrap();
    let mut rti = CompiledRti::from_image(&view, IDENTITY).unwrap();
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
    assert_eq!(grants(publish(&mut rti, A, 1, Some(early))), [(A, early)]);
    assert!(publish(&mut rti, B, 1, Some(middle)).is_empty());
    assert!(publish(&mut rti, C, 1, Some(target)).is_empty());
    // A's new lower bound reaches C through the compiled affected-downstream table.
    let advanced = WireTag::finite(101, 0);
    assert_eq!(
        grants(publish(&mut rti, A, 2, Some(advanced))),
        [(A, advanced), (B, middle), (C, target)]
    );
}

#[test]
fn chain_waits_for_each_upstream_to_advance_past_requested_tag() {
    let mut rti = admitted(&CHAIN, &["a", "b", "c"]);
    assert!(publish(&mut rti, B, 1, Some(WireTag::ZERO)).is_empty());
    assert!(publish(&mut rti, C, 1, Some(WireTag::ZERO)).is_empty());
    assert_eq!(
        grants(publish(&mut rti, A, 1, Some(WireTag::ZERO))),
        [(A, WireTag::ZERO)]
    );
    let next = WireTag::finite(0, 1);
    assert_eq!(
        grants(publish(&mut rti, A, 2, Some(next))),
        [(A, next), (B, WireTag::ZERO)]
    );
    assert_eq!(
        grants(publish(&mut rti, A, 3, Some(WireTag::finite(0, 2)))),
        [(A, WireTag::finite(0, 2))]
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
        [(A, pending)]
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
        [(B, WireTag::finite(0, 10))]
    );
    let target = WireTag::finite(0, 9);
    assert!(publish(&mut rti, C, 1, Some(target)).is_empty());
    assert_eq!(
        grants(rti.handle(B, RtiRequest::Complete { tag: pending })),
        [(C, target)]
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
            ),
            RtiMemberImage::new(
                RecoveryPolicy::FailStop,
                SliceRange::new(3, 1),
                SliceRange::new(4, 2),
                SliceRange::new(2, 2),
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
        [(B, WireTag::ZERO), (A, WireTag::ZERO)]
    );
}
