use std::{collections::BTreeMap, time::Duration as StdDuration};

use boomerang_runtime::{
    CommonContext, CoordinationOutcome, FrontierPublication, LogicalTimeCoordinator,
    LogicalTimeFrontier, Tag,
};
use futures_util::{SinkExt, StreamExt};

use super::{
    service::FederateParticipantProxy, FederateCoordinationLayout, FederateCoordinationService,
};
use crate::{
    client::{FederateClientRoute, FederateProtocolClient},
    in_memory_transport_pair, EndpointId, FederateId, FederateToRti, ProtocolFrame,
    RtiLogicalTimeCoordinator, RtiToFederate, WireTag,
};

fn fed() -> FederateId {
    FederateId::new("federate")
}

type ParticipantWakes = BTreeMap<boomerang_runtime::EnclaveKey, boomerang_runtime::SendContext>;
type ParticipantReceivers = BTreeMap<
    boomerang_runtime::EnclaveKey,
    boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
>;
type ParticipantChannels = (
    ParticipantWakes,
    ParticipantReceivers,
    Vec<boomerang_runtime::keepalive::Sender>,
);

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
    routes: impl IntoIterator<Item = FederateClientRoute>,
    fake: F,
) -> (RtiLogicalTimeCoordinator, tokio::task::JoinHandle<()>)
where
    F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    coordinator_with_fake_rti_and_faults(crate::FederatedFaultState::default(), routes, fake).await
}

async fn coordinator_with_fake_rti_and_faults<F, Fut>(
    faults: crate::FederatedFaultState,
    routes: impl IntoIterator<Item = FederateClientRoute>,
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
        RtiLogicalTimeCoordinator::new(fed(), client, routes, faults).unwrap(),
        handle,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_net_failure_fans_out_the_first_concrete_error() {
    let faults = crate::FederatedFaultState::default();
    faults.record(crate::FederatedEndpointError::codec("first action failure"));
    faults.record(crate::FederatedEndpointError::send("later failure"));
    let (coordinator, rti) =
        coordinator_with_fake_rti_and_faults(faults, [], |mut transport| async move {
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
    let first = 0usize.into();
    let second = 1usize.into();
    let (wakes, mut receivers, _guards) = participant_channels([first, second]);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([first, second]),
        wakes,
    )
    .unwrap();
    let mut first_participant = service.participant(first);
    let mut second_participant = service.participant(second);
    first_participant
        .publish_frontier(candidate(Tag::ZERO))
        .unwrap();
    let first_events = receivers.remove(&first).unwrap();
    let acquire = std::thread::spawn(move || first_participant.acquire(Tag::ZERO, &first_events));
    std::thread::sleep(StdDuration::from_millis(20));

    let publish_error = second_participant
        .publish_frontier(candidate(Tag::ZERO))
        .unwrap_err();
    assert!(publish_error.to_string().contains("first action failure"));
    let acquire_error = acquire.join().unwrap().unwrap_err();
    assert!(acquire_error.to_string().contains("first action failure"));
    assert!(!acquire_error.to_string().contains("service stopped"));
    assert!(service.join().unwrap_err().contains("first action failure"));
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_ltc_failure_fails_the_pending_completion_with_the_first_error() {
    let faults = crate::FederatedFaultState::default();
    let coordinator_faults = faults.clone();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let (coordinator, rti) =
        coordinator_with_fake_rti_and_faults(coordinator_faults, [], |mut transport| async move {
            assert!(matches!(
                recv_frame(&mut transport).await,
                FederateToRti::Net {
                    tag: WireTag::ZERO,
                    ..
                }
            ));
            send_frame(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
            let _ = done_rx.await;
        })
        .await;
    let participant_key = 0usize.into();
    let (wakes, mut receivers, _guards) = participant_channels([participant_key]);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([participant_key]),
        wakes,
    )
    .unwrap();
    let mut participant = service.participant(participant_key);
    participant.publish_frontier(candidate(Tag::ZERO)).unwrap();
    let events = receivers.remove(&participant_key).unwrap();
    let _ = participant.acquire(Tag::ZERO, &events).unwrap();

    faults.record(crate::FederatedEndpointError::codec(
        "completion action failure",
    ));
    let completion_error = participant.complete(Tag::ZERO).unwrap_err();
    assert!(completion_error
        .to_string()
        .contains("completion action failure"));
    assert!(!completion_error.to_string().contains("service stopped"));
    assert!(service
        .join()
        .unwrap_err()
        .contains("completion action failure"));
    let _ = done_tx.send(());
    rti.await.unwrap();
}

fn participant_channels(
    keys: impl IntoIterator<Item = boomerang_runtime::EnclaveKey>,
) -> ParticipantChannels {
    let mut wakes = BTreeMap::new();
    let mut receivers = BTreeMap::new();
    let mut guards = Vec::new();
    for key in keys {
        let enclave = boomerang_runtime::Enclave::default();
        wakes.insert(key, enclave.create_send_context(key));
        receivers.insert(key, enclave.event_rx);
        guards.push(enclave.shutdown_tx);
    }
    (wakes, receivers, guards)
}

fn candidate(tag: Tag) -> FrontierPublication {
    FrontierPublication {
        frontier: LogicalTimeFrontier::Candidate(tag),
        consumed_wake: None,
    }
}

#[test]
fn participant_proxy_is_the_only_federated_scheduler_coordinator() {
    fn assert_coordinator<T: boomerang_runtime::LogicalTimeCoordinator>() {}
    assert_coordinator::<FederateParticipantProxy>();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_grant_releases_all_participants_and_fixed_point_sends_one_ltc_and_stop() {
    let (coordinator, rti) = coordinator_with_fake_rti([], |mut transport| async move {
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
            FederateToRti::Ltc {
                federate_id: fed(),
                tag: WireTag::ZERO,
            }
        );
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
    let keys = [0usize.into(), 1usize.into()];
    let (wakes, receivers, _guards) = participant_channels(keys);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new(keys),
        wakes,
    )
    .unwrap();
    let mut first = service.participant(keys[0]);
    let mut second = service.participant(keys[1]);
    first.publish_frontier(candidate(Tag::ZERO)).unwrap();
    second.publish_frontier(candidate(Tag::ZERO)).unwrap();
    let first_wake = match receivers[&keys[0]]
        .recv_timeout(StdDuration::from_secs(1))
        .unwrap()
    {
        boomerang_runtime::AsyncEvent::CoordinationWake(wake) => wake,
        event => panic!("expected coordination wake, got {event:?}"),
    };
    let second_wake = match receivers[&keys[1]]
        .recv_timeout(StdDuration::from_secs(1))
        .unwrap()
    {
        boomerang_runtime::AsyncEvent::CoordinationWake(wake) => wake,
        event => panic!("expected coordination wake, got {event:?}"),
    };
    assert!(matches!(
        first.acquire(Tag::ZERO, &receivers[&keys[0]]).unwrap(),
        CoordinationOutcome::Granted
    ));
    assert!(matches!(
        second.acquire(Tag::ZERO, &receivers[&keys[1]]).unwrap(),
        CoordinationOutcome::Granted
    ));
    first
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: Some(first_wake),
        })
        .unwrap();
    second
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: Some(second_wake),
        })
        .unwrap();
    let first_retry = match receivers[&keys[0]]
        .recv_timeout(StdDuration::from_secs(1))
        .unwrap()
    {
        boomerang_runtime::AsyncEvent::CoordinationWake(wake) => wake,
        event => panic!("expected retry wake, got {event:?}"),
    };
    let second_retry = match receivers[&keys[1]]
        .recv_timeout(StdDuration::from_secs(1))
        .unwrap()
    {
        boomerang_runtime::AsyncEvent::CoordinationWake(wake) => wake,
        event => panic!("expected retry wake, got {event:?}"),
    };
    first
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: Some(first_retry),
        })
        .unwrap();
    second
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: Some(second_retry),
        })
        .unwrap();
    first.complete(Tag::ZERO).unwrap();
    first
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    second.complete(Tag::ZERO).unwrap();
    second
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    service.join().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn candidate_regression_revises_net_and_all_idle_sends_no_finite_net() {
    let later = Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
    let (coordinator, rti) = coordinator_with_fake_rti([], move |mut transport| async move {
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed(),
                tag: WireTag::try_from(later).unwrap(),
            }
        );
        assert_eq!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed(),
                tag: WireTag::ZERO,
            }
        );
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
    let keys = [0usize.into(), 1usize.into()];
    let (wakes, _receivers, _guards) = participant_channels(keys);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new(keys),
        wakes,
    )
    .unwrap();
    let mut first = service.participant(keys[0]);
    let mut second = service.participant(keys[1]);
    first.publish_frontier(candidate(later)).unwrap();
    second.publish_frontier(candidate(later)).unwrap();
    second.publish_frontier(candidate(Tag::ZERO)).unwrap();
    first
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: None,
        })
        .unwrap();
    second
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Idle,
            consumed_wake: None,
        })
        .unwrap();
    first
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    second
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    service.join().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_stop_fails_pending_acquire_without_deadlock() {
    let (coordinator, rti) = coordinator_with_fake_rti([], |mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
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
    let key = 0usize.into();
    let (wakes, mut receivers, _guards) = participant_channels([key]);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([key]),
        wakes,
    )
    .unwrap();
    let mut participant = service.participant(key);
    participant.publish_frontier(candidate(Tag::ZERO)).unwrap();
    let event_rx = receivers.remove(&key).unwrap();
    let waiter = std::thread::spawn(move || participant.acquire(Tag::ZERO, &event_rx));
    std::thread::sleep(StdDuration::from_millis(20));
    service.force_stop().unwrap();
    let error = waiter.join().unwrap().unwrap_err();
    assert!(error.to_string().contains("coordination stopped"));
    service.join().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_error_fans_out_to_all_pending_acquires() {
    let (coordinator, rti) = coordinator_with_fake_rti([], |mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        std::thread::sleep(StdDuration::from_millis(30));
        send_frame(
            &mut transport,
            RtiToFederate::Error {
                message: "boom".into(),
            },
        )
        .await;
    })
    .await;
    let keys = [0usize.into(), 1usize.into()];
    let (wakes, mut receivers, _guards) = participant_channels(keys);
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new(keys),
        wakes,
    )
    .unwrap();
    let mut first = service.participant(keys[0]);
    let mut second = service.participant(keys[1]);
    first.publish_frontier(candidate(Tag::ZERO)).unwrap();
    second.publish_frontier(candidate(Tag::ZERO)).unwrap();
    let first_rx = receivers.remove(&keys[0]).unwrap();
    let second_rx = receivers.remove(&keys[1]).unwrap();
    let first_waiter = std::thread::spawn(move || first.acquire(Tag::ZERO, &first_rx));
    let second_waiter = std::thread::spawn(move || second.acquire(Tag::ZERO, &second_rx));
    assert!(first_waiter.join().unwrap().is_err());
    assert!(second_waiter.join().unwrap().is_err());
    assert!(service.join().is_err());
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inbound_msg_interrupts_waiting_participant_before_grant_release() {
    let endpoint = EndpointId::new("source.out->federate.in");
    let source = FederateId::new("source");
    let (grant_tx, grant_rx) = std::sync::mpsc::channel();
    let fake_endpoint = endpoint.clone();
    let fake_source = source.clone();
    let (mut enclave, key) = (boomerang_runtime::Enclave::default(), 0usize.into());
    let action = enclave.insert_action(|action_key| {
        boomerang_runtime::Action::<u32>::new("inbound", action_key, None, true).boxed()
    });
    let inbound = crate::FederatedInboundEndpoint::new(
        enclave.create_send_context(key),
        enclave.create_async_action_ref::<u32>(action),
        Box::new(|payload: &[u8]| {
            std::str::from_utf8(payload)
                .map_err(|error| crate::CodecError::message(error.to_string()))?
                .parse::<u32>()
                .map_err(|error| crate::CodecError::message(error.to_string()))
        }),
    )
    .unwrap();
    let mut route = FederateClientRoute::new(endpoint, source, fed());
    route.bind_inbound(inbound);
    let (coordinator, rti) = coordinator_with_fake_rti([route], move |mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        send_frame(
            &mut transport,
            RtiToFederate::Msg {
                source: fake_source,
                endpoint: fake_endpoint,
                tag: WireTag::ZERO,
                payload: b"7".to_vec(),
            },
        )
        .await;
        tokio::task::spawn_blocking(move || grant_rx.recv().unwrap())
            .await
            .unwrap();
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
    let wake = enclave.create_send_context(key);
    let event_rx = enclave.event_rx;
    let _guard = enclave.shutdown_tx;
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([key]),
        BTreeMap::from([(key, wake)]),
    )
    .unwrap();
    let mut participant = service.participant(key);
    participant.publish_frontier(candidate(Tag::ZERO)).unwrap();
    assert!(matches!(
        participant.acquire(Tag::ZERO, &event_rx).unwrap(),
        CoordinationOutcome::Interrupted(boomerang_runtime::AsyncEvent::Logical {
            tag,
            key: observed,
            ..
        }) if tag == Tag::ZERO && observed == action
    ));
    grant_tx.send(()).unwrap();
    std::thread::sleep(StdDuration::from_millis(20));
    service.force_stop().unwrap();
    service.join().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_acquire_is_superseded_by_next_publication() {
    let later = Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
    let (coordinator, rti) = coordinator_with_fake_rti([], move |mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net { tag, .. } if tag == WireTag::try_from(later).unwrap()
        ));
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
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
    let key = 0usize.into();
    let enclave = boomerang_runtime::Enclave::default();
    let wake = enclave.create_send_context(key);
    let event_rx = enclave.event_rx;
    let _guard = enclave.shutdown_tx;
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([key]),
        BTreeMap::from([(key, wake.clone())]),
    )
    .unwrap();
    let mut participant = service.participant(key);
    participant.publish_frontier(candidate(later)).unwrap();
    wake.schedule_external(boomerang_runtime::AsyncEvent::TagReleaseProvisional {
        enclave: key,
        tag: later,
    });
    assert!(matches!(
        participant.acquire(later, &event_rx).unwrap(),
        CoordinationOutcome::Interrupted(_)
    ));
    participant.publish_frontier(candidate(Tag::ZERO)).unwrap();
    participant
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    service.join().unwrap();
    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_participant_event_queue_does_not_block_service_wake_delivery() {
    let (grant_sent_tx, grant_sent_rx) = std::sync::mpsc::channel();
    let (coordinator, rti) = coordinator_with_fake_rti([], move |mut transport| async move {
        assert!(matches!(
            recv_frame(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        send_frame(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
        grant_sent_tx.send(()).unwrap();
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
    let key = 0usize.into();
    let enclave = boomerang_runtime::Enclave::with_event_q_size(1);
    let wake = enclave.create_send_context(key);
    let event_rx = enclave.event_rx;
    let _guard = enclave.shutdown_tx;
    assert!(
        wake.schedule_external(boomerang_runtime::AsyncEvent::Shutdown {
            delay: boomerang_runtime::Duration::ZERO,
        })
    );
    let service = FederateCoordinationService::spawn(
        coordinator,
        FederateCoordinationLayout::new([key]),
        BTreeMap::from([(key, wake)]),
    )
    .unwrap();
    let mut participant = service.participant(key);
    participant.publish_frontier(candidate(Tag::ZERO)).unwrap();
    grant_sent_rx.recv().unwrap();
    std::thread::sleep(StdDuration::from_millis(20));

    let (publication_tx, publication_rx) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn(move || {
        let result = participant.publish_frontier(candidate(Tag::ZERO));
        publication_tx.send(result).unwrap();
        participant
    });
    publication_rx
        .recv_timeout(StdDuration::from_millis(100))
        .expect("service blocked while delivering a wake to a full participant queue")
        .unwrap();

    assert!(matches!(
        event_rx.recv().unwrap(),
        boomerang_runtime::AsyncEvent::Shutdown { .. }
    ));
    assert!(matches!(
        event_rx.recv_timeout(StdDuration::from_secs(1)).unwrap(),
        boomerang_runtime::AsyncEvent::CoordinationWake(_)
    ));
    let mut participant = publisher.join().unwrap();
    participant
        .publish_frontier(FrontierPublication {
            frontier: LogicalTimeFrontier::Finished,
            consumed_wake: None,
        })
        .unwrap();
    service.join().unwrap();
    rti.await.unwrap();
}
