use futures_util::{SinkExt, StreamExt};

use super::*;
use crate::{in_memory_transport_pair, FederateToRti, ProtocolFrame, RtiToFederate};

fn fed() -> FederateId {
    FederateId::new("federate")
}

async fn recv_frame(
    transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
) -> FederateToRti {
    match transport.1.next().await.unwrap().unwrap() {
        ProtocolFrame::FederateToRti(message) => message,
        frame => panic!("expected federate frame, got {frame:?}"),
    }
}

async fn send_frame(
    transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    message: RtiToFederate,
) {
    transport
        .0
        .send(ProtocolFrame::RtiToFederate(message))
        .await
        .unwrap();
}

async fn coordinator_with_fake_rti<F, Fut>(
    fake: F,
) -> (RtiLogicalTimeCoordinator, tokio::task::JoinHandle<()>)
where
    F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (client_transport, rti_transport) = in_memory_transport_pair();
    let handle = tokio::spawn(async move {
        let mut transport = rti_transport;
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Hello { federate_id: fed() }
        );
        send_frame(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        fake(transport).await;
    });
    let (sink, stream) = client_transport;
    let client = FederateProtocolClient::connect(fed(), sink, stream)
        .await
        .unwrap();
    (
        RtiLogicalTimeCoordinator::new(fed(), client, [], crate::FederatedFaultState::default())
            .unwrap(),
        handle,
    )
}

#[test]
fn protocol_poll_outcomes_are_wire_progress_only() {
    assert_ne!(ProtocolPoll::Pending, ProtocolPoll::Progress);
    assert_eq!(
        ProtocolPoll::Granted(boomerang_runtime::Tag::ZERO),
        ProtocolPoll::Granted(boomerang_runtime::Tag::ZERO)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_pending_net_is_idempotent_and_sufficient_tag_clears_it() {
    let (mut coordinator, rti) = coordinator_with_fake_rti(|mut transport| async move {
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed(),
                tag: WireTag::ZERO,
            }
        );
        send_frame(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed(),
                tag: WireTag::FOREVER,
            }
        );
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Stop { federate_id: fed() }
        );
    })
    .await;

    coordinator
        .submit_net(boomerang_runtime::Tag::ZERO)
        .unwrap();
    coordinator
        .submit_net(boomerang_runtime::Tag::ZERO)
        .unwrap();
    loop {
        match coordinator.poll().unwrap() {
            ProtocolPoll::Granted(tag) => {
                assert_eq!(tag, boomerang_runtime::Tag::ZERO);
                break;
            }
            ProtocolPoll::Pending | ProtocolPoll::Progress => {}
        }
    }
    assert_eq!(coordinator.pending_request, None);
    coordinator.stop().unwrap();
    coordinator.stop().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insufficient_tag_preserves_revised_pending_request() {
    let later = boomerang_runtime::Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
    let wire_later = WireTag::try_from(later).unwrap();
    let (mut coordinator, rti) = coordinator_with_fake_rti(move |mut transport| async move {
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed(),
                tag: wire_later,
            }
        );
        send_frame(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::FOREVER,
                ..
            }
        ));
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Stop { .. }
        ));
    })
    .await;
    coordinator.submit_net(later).unwrap();
    loop {
        match coordinator.poll().unwrap() {
            ProtocolPoll::Granted(tag) => {
                assert_eq!(tag, boomerang_runtime::Tag::ZERO);
                break;
            }
            ProtocolPoll::Pending | ProtocolPoll::Progress => {}
        }
    }
    assert_eq!(coordinator.pending_request, Some(wire_later));
    coordinator.submit_net(later).unwrap();
    coordinator.stop().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_error_is_terminal_for_later_operations() {
    let (mut coordinator, rti) = coordinator_with_fake_rti(|mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        send_frame(
            &mut transport,
            RtiToFederate::Error {
                message: "boom".into(),
            },
        )
        .await;
    })
    .await;
    coordinator
        .submit_net(boomerang_runtime::Tag::ZERO)
        .unwrap();
    loop {
        match coordinator.poll() {
            Err(error) => {
                assert!(error.to_string().contains("boom"));
                break;
            }
            Ok(ProtocolPoll::Pending | ProtocolPoll::Progress) => {}
            Ok(ProtocolPoll::Granted(tag)) => panic!("unexpected TAG {tag}"),
        }
    }
    assert!(matches!(
        coordinator.submit_net(boomerang_runtime::Tag::ZERO),
        Err(FederateClientError::CoordinationFailed)
    ));
    assert!(matches!(
        coordinator.report_logical_tag_complete(boomerang_runtime::Tag::ZERO),
        Err(FederateClientError::CoordinationFailed)
    ));
    rti.await.unwrap();
}
