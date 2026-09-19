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
            32,
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
        reports: Arc::default(),
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

#[test]
fn dnet_requires_accepted_authority_and_restores_latest_skipped_net() {
    use boomerang_runtime::{Duration as LogicalDuration, Tag};
    let sink = Arc::new(Requests::default());
    let mut client = CentralRtiClient::connect(
        sink.clone(),
        Replies(
            [
                RtiReply::Started,
                RtiReply::SuppressPublication {
                    tag: WireTag::finite(100, 0),
                },
                RtiReply::Grant {
                    revision: 0,
                    tag: WireTag::finite(100, 0),
                },
                RtiReply::SuppressPublication {
                    tag: WireTag::finite(15, 0),
                },
            ]
            .into(),
        ),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .unwrap();
    client.progress(Duration::ZERO).unwrap();
    client.progress(Duration::ZERO).unwrap(); // Receiving a grant does not prove runtime acceptance.
    let publication = |revision, nanos| {
        FederatePublication::new(
            CoordinationRevision::new(revision),
            Some(Tag::new(LogicalDuration::nanoseconds(nanos), 0)),
        )
    };
    client.publish(publication(1, 10)).unwrap();
    let accepted = Some(Tag::new(LogicalDuration::nanoseconds(100), 0));
    client
        .publish(publication(2, 20).with_grant_horizon(accepted))
        .unwrap();
    client
        .publish(publication(3, 30).with_grant_horizon(accepted))
        .unwrap();
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, RtiRequest::Publish { .. }))
            .count(),
        1
    );
    client.progress(Duration::ZERO).unwrap();
    assert!(
        matches!(sink.0.lock().unwrap().last(), Some(RtiRequest::Publish { revision: 3, next_event: Some(tag) }) if *tag == WireTag::finite(30, 0))
    );
    client
        .publish(
            FederatePublication::new(CoordinationRevision::new(4), None)
                .with_grant_horizon(accepted),
        )
        .unwrap();
    assert!(matches!(
        sink.0.lock().unwrap().last(),
        Some(RtiRequest::Publish {
            revision: 4,
            next_event: None
        })
    ));
}

#[test]
fn local_completions_are_silent_and_ltc_prevents_restoring_completed_net() {
    use boomerang_runtime::{FederateCompletion, Tag};
    let sink = Arc::new(Requests::default());
    let mut client = CentralRtiClient::connect(
        sink.clone(),
        Replies(
            [
                RtiReply::Started,
                RtiReply::SuppressPublication {
                    tag: WireTag::FOREVER,
                },
                RtiReply::SuppressPublication {
                    tag: WireTag::NEVER,
                },
            ]
            .into(),
        ),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .unwrap();
    client.progress(Duration::ZERO).unwrap();
    client
        .publish(
            FederatePublication::new(CoordinationRevision::new(1), Some(Tag::ZERO))
                .with_grant_horizon(Some(Tag::FOREVER)),
        )
        .unwrap();
    client.complete(FederateCompletion::new(Tag::ZERO)).unwrap();
    assert_eq!(sink.0.lock().unwrap().len(), 1); // Hello only: local completion is not an LTC trigger.
    client
        .complete(FederateCompletion::new(Tag::ZERO).with_network_input(true))
        .unwrap();
    client.progress(Duration::ZERO).unwrap();
    assert!(matches!(
        sink.0.lock().unwrap().as_slice(),
        [
            RtiRequest::Hello { .. },
            RtiRequest::Complete { tag: WireTag::ZERO }
        ]
    ));
}

#[test]
fn ten_event_trace_reduces_reports_without_changing_completion_frontier() {
    use boomerang_runtime::{Duration as LogicalDuration, FederateCompletion, Tag};
    let sink = Arc::new(Requests::default());
    let mut client = CentralRtiClient::connect(
        sink.clone(),
        Replies(
            [
                RtiReply::Started,
                RtiReply::SuppressPublication {
                    tag: WireTag::FOREVER,
                },
            ]
            .into(),
        ),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .unwrap();
    client.progress(Duration::ZERO).unwrap();
    for n in 1..=10 {
        let tag = Tag::new(LogicalDuration::nanoseconds(n), 0);
        let publication = FederatePublication::new(CoordinationRevision::new(n as u64), Some(tag));
        client
            .publish(if n == 1 {
                publication
            } else {
                publication.with_grant_horizon(Some(Tag::FOREVER))
            })
            .unwrap();
        client
            .complete(FederateCompletion::new(tag).with_network_input(n == 5 || n == 10))
            .unwrap();
    }
    let requests = sink.0.lock().unwrap();
    // An eager semantic oracle publishes and completes each of the ten local events.
    // Its final completion is 10; only tags 5 and 10 consumed network input here.
    assert_eq!(
        requests
            .iter()
            .filter(|r| matches!(r, RtiRequest::Publish { .. }))
            .count(),
        1
    );
    let confirmed: Vec<_> = requests
        .iter()
        .filter_map(|r| match r {
            RtiRequest::Complete { tag } => Some(*tag),
            _ => None,
        })
        .collect();
    assert_eq!(confirmed, [WireTag::finite(5, 0), WireTag::finite(10, 0)]);
}

#[test]
fn advice_delayed_across_idle_is_revoked_before_waking_net_can_remain_suppressed() {
    use boomerang_runtime::{Duration as LogicalDuration, Tag};
    let sink = Arc::new(Requests::default());
    let mut client = CentralRtiClient::connect(
        sink.clone(),
        Replies(
            [
                RtiReply::Started,
                RtiReply::SuppressPublication {
                    tag: WireTag::finite(100, 0),
                },
                RtiReply::SuppressPublication {
                    tag: WireTag::finite(20, 0),
                },
            ]
            .into(),
        ),
        bindings(),
        BTreeMap::new(),
        Duration::from_millis(10),
    )
    .unwrap();
    client
        .publish(FederatePublication::new(CoordinationRevision::new(2), None))
        .unwrap();
    client.progress(Duration::ZERO).unwrap(); // Old advice arrives after the idle publication.
    let tag = Tag::new(LogicalDuration::nanoseconds(30), 0);
    client
        .publish(
            FederatePublication::new(CoordinationRevision::new(3), Some(tag))
                .with_grant_horizon(Some(Tag::FOREVER)),
        )
        .unwrap();
    assert!(matches!(
        sink.0.lock().unwrap().last(),
        Some(RtiRequest::Publish {
            next_event: None,
            ..
        })
    ));
    client.progress(Duration::ZERO).unwrap();
    assert!(
        matches!(sink.0.lock().unwrap().last(), Some(RtiRequest::Publish { revision: 3, next_event: Some(tag) })
        if *tag == WireTag::finite(30, 0))
    );
}
