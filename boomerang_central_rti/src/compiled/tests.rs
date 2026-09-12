//! Scripted transport behavior for bounded client lifecycle checks; never a runtime transport.
use super::*;
use boomerang_runtime::{CoordinationRevision, FederateCoordinationBackend, FederatePublication};
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

/// Expires startup and tells the transport owner to release peers.
#[test]
fn admission_timeout_aborts_session() {
    let requests = Arc::new(Requests::default());
    let error = CentralRtiClient::connect(
        requests.clone(),
        Replies(VecDeque::new()),
        CoordinationIdentity::new([1; 32]),
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
            CoordinationIdentity::new([1; 32]),
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
