//! Scripted transport behavior for bounded client lifecycle checks; never a runtime transport.
use super::*;
use boomerang_runtime::{
    image::{RecoveryPolicy, RtiImage, RtiMemberImage, SliceRange, TinyMapView},
    CoordinationRevision, FederateCoordinationBackend, FederatePublication,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Ordered request recorder used only by these protocol tests.
#[derive(Default)]
struct Requests(Mutex<Vec<RtiRequest>>);
impl RtiRequestSink for Requests {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        self.0.lock().unwrap().push(request);
        Ok(())
    }
}
/// Scripted bounded replies, with `None` representing expiration of the supplied bound.
struct Replies(VecDeque<RtiReply>);
impl RtiReplySource for Replies {
    fn receive(&mut self, timeout: Duration) -> Result<Option<RtiReply>, CentralRtiError> {
        assert!(timeout <= Duration::from_millis(10));
        Ok(self.0.pop_front())
    }
}

/// Isolates client lifecycle tests with one route-less reference member.
fn bindings() -> RtiClientBindings<'static> {
    const IMAGE: RtiImage<'static> = RtiImage::new(
        TinyMapView::new(&[RtiMemberImage::new(
            RecoveryPolicy::FailStop,
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
            SliceRange::new(0, 0),
        )]),
        &[],
        &[],
        TinyMapView::new(&[]),
        TinyMapView::new(&[]),
        TinyMapView::new(&[]),
        TinyMapView::new(&[]),
        TinyMapView::new(&[]),
    );
    RtiClientBindings {
        image: IMAGE,
        member: FederateIndex::new(0),
        identity: CoordinationIdentity::new([1; 32]),
    }
}

/// Expires startup and tells the transport owner to release peers.
#[test]
fn admission_timeout_aborts_session() {
    let requests = Arc::new(Requests::default());
    let error = CentralRtiClient::connect(
        requests.clone(),
        Replies(VecDeque::new()),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("admission timed out"));
    assert!(matches!(
        requests.0.lock().unwrap().as_slice(),
        [RtiRequest::Hello { .. }, RtiRequest::Abort { .. }]
    ));
}

/// Rejects unauthorized stop and aborts on missing acknowledgement.
#[test]
fn stop_requires_authority_and_bounded_acknowledgement() {
    for authorized in [false, true] {
        let requests = Arc::new(Requests::default());
        let replies = if authorized {
            vec![RtiReply::Started, RtiReply::Idle { revision: 1 }]
        } else {
            vec![RtiReply::Started]
        };
        let mut client = CentralRtiClient::connect(
            requests.clone(),
            Replies(replies.into()),
            bindings(),
            BTreeMap::new(),
            Duration::from_millis(10),
        )
        .unwrap();
        if authorized {
            let revision = CoordinationRevision::new(1);
            client
                .publish(FederatePublication::new(revision, None))
                .unwrap();
            assert!(!client.confirm_idle(revision).unwrap());
            client.progress(Duration::ZERO).unwrap();
            assert!(client.confirm_idle(revision).unwrap());
        }
        let error = client.stop().unwrap_err().to_string();
        assert!(error.contains(if authorized {
            "acknowledgement timed out"
        } else {
            "before global quiescence"
        }));
        assert!(matches!(
            requests.0.lock().unwrap().last(),
            Some(RtiRequest::Abort { .. })
        ));
    }
}

/// Rejects delivery keys absent from the preflight mapping without indexing a local route domain.
#[test]
fn unknown_inbound_route_key_fails_closed() {
    let requests = Arc::new(Requests::default());
    let mut client = CentralRtiClient::connect(
        requests.clone(),
        Replies(
            vec![
                RtiReply::Started,
                RtiReply::Payload {
                    route: RtiRouteIndex::new(9),
                    tag: WireTag::ZERO,
                    payload: vec![42],
                },
            ]
            .into(),
        ),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .unwrap();
    assert!(client
        .progress(Duration::ZERO)
        .unwrap_err()
        .to_string()
        .contains("unknown inbound RTI route key"));
}
