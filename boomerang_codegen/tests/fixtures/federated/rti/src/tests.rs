//! Validate generated tables through their real wire and RTI consumers.
use super::generated::*;
use boomerang_runtime::image::RtiImageView;

/// The generated resource contract bounds actual RTI admission before payload forwarding.
#[test]
fn generated_rti_enforces_in_transit_capacity() {
    use boomerang_central_rti::compiled::{CompiledRti, RtiReply, RtiRequest};
    use boomerang_federated::protocol::WireTag;
    let view = RtiImageView::new(COORDINATION_IMAGE, COORDINATION_MEMBERS).unwrap();
    let mut rti = CompiledRti::from_image(view, COORDINATION_IDENTITY).unwrap();
    for (member, _) in COORDINATION_IMAGE.members().iter() {
        rti.handle(
            member,
            RtiRequest::Hello {
                identity: COORDINATION_IDENTITY,
            },
        );
    }
    let (key, route) = COORDINATION_IMAGE.routes().iter().next().unwrap();
    let capacity = COORDINATION_IMAGE.members()[route.target()].in_transit_capacity();
    assert!(capacity > 0);
    rti.handle(
        route.source(),
        RtiRequest::Publish {
            revision: 1,
            next_event: Some(WireTag::ZERO),
        },
    );
    for index in 0..=capacity {
        let replies = rti.handle(
            route.source(),
            RtiRequest::Payload {
                route: key,
                tag: WireTag::finite(i128::from(route.delay_nanos()) + i128::from(index), 0),
                payload: vec![42],
            },
        );
        if index < capacity {
            assert!(replies
                .iter()
                .any(|delivery| matches!(delivery.reply, RtiReply::Payload { .. })));
        } else {
            assert!(!replies.is_empty());
            assert!(replies.iter().all(|delivery| matches!(&delivery.reply,
                RtiReply::Failed { message } if message.contains("in-transit tag capacity"))));
        }
    }
}
