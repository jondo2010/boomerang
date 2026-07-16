use futures_util::{SinkExt, StreamExt};

use super::*;
use crate::{in_memory_transport_pair, EndpointId, FederatedTopology, TopologyEdge, WireDelay};

fn fed(id: &str) -> FederateId {
    FederateId::new(id)
}

fn endpoint() -> EndpointId {
    EndpointId::new("source.out->sink.in")
}

fn protocol_endpoint() -> EndpointId {
    endpoint()
}

fn source_sink_topology() -> FederatedTopology {
    FederatedTopology::with_edges(
        [fed("source"), fed("sink")],
        [TopologyEdge::new(
            fed("source"),
            fed("sink"),
            protocol_endpoint(),
            WireDelay::ZERO,
        )],
    )
}

fn route() -> FederateClientRoute {
    FederateClientRoute::new(endpoint(), fed("source"), fed("sink"))
}

async fn recv_federate_to_rti(
    transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
) -> FederateToRti {
    match transport.1.next().await.unwrap().unwrap() {
        ProtocolFrame::FederateToRti(message) => message,
        frame => panic!("expected federate-to-RTI frame, got {frame:?}"),
    }
}

async fn send_rti_to_federate(
    transport: &mut crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    message: RtiToFederate,
) {
    transport
        .0
        .send(ProtocolFrame::RtiToFederate(message))
        .await
        .unwrap();
}

async fn connect_client_with_fake_rti<F, Fut>(
    federate_id: FederateId,
    rti: F,
) -> (FederateProtocolClient, JoinHandle<()>)
where
    F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    connect_client_with_fake_rti_and_mailbox(federate_id, FederateClientMailbox::new(), rti).await
}

async fn connect_client_with_fake_rti_and_mailbox<F, Fut>(
    federate_id: FederateId,
    mailbox: FederateClientMailbox,
    rti: F,
) -> (FederateProtocolClient, JoinHandle<()>)
where
    F: FnOnce(crate::InMemoryTransport<ProtocolFrame, ProtocolFrame>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let topology = source_sink_topology();
    let (client_transport, rti_transport) = in_memory_transport_pair();
    let handle = tokio::spawn(rti(rti_transport));
    let (sink, stream) = client_transport;
    let client = FederateProtocolClient::connect_with_mailbox(
        federate_id.clone(),
        topology.neighbors_for(&federate_id),
        sink,
        stream,
        mailbox,
    )
    .await
    .unwrap();
    assert_eq!(client.start_unix_epoch_ns(), 0);
    (client, handle)
}

fn empty_event_rx() -> boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent> {
    boomerang_runtime::Enclave::default().event_rx
}

fn inbound_endpoint_for_u32() -> (
    crate::FederatedInboundEndpoint,
    boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    boomerang_runtime::ActionKey,
    boomerang_runtime::keepalive::Sender,
) {
    let mut enclave = boomerang_runtime::Enclave::default();
    let action_key = enclave.insert_action(|key| {
        boomerang_runtime::Action::<u32>::new("inbound", key, None, true).boxed()
    });
    let action_ref = enclave.create_async_action_ref::<u32>(action_key);
    let context = enclave.create_send_context(boomerang_runtime::EnclaveKey::from(0));
    let endpoint = crate::FederatedInboundEndpoint::new(
        context,
        action_ref,
        Box::new(|bytes: &[u8]| {
            std::str::from_utf8(bytes)
                .map_err(|error| crate::CodecError::message(error.to_string()))?
                .parse::<u32>()
                .map_err(|error| crate::CodecError::message(error.to_string()))
        }),
    )
    .unwrap();
    (endpoint, enclave.event_rx, action_key, enclave.shutdown_tx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_sends_net_outbound_msg_and_ltc_frames() {
    boomerang_util::test_tracing::init_with_directive("boomerang_federated=debug");

    let mut connections =
        crate::FederatedRuntimeConnections::new([fed("source"), fed("sink")], [route()]).unwrap();
    let (outbound, _) = connections.outbound_endpoint(&endpoint()).unwrap();
    let mailbox = connections.take_mailbox(&fed("source")).unwrap();
    let (client, rti) = connect_client_with_fake_rti_and_mailbox(
        fed("source"),
        mailbox,
        |mut transport| async move {
            assert!(matches!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Hello { federate_id, .. } if federate_id == fed("source")
            ));
            send_rti_to_federate(
                &mut transport,
                RtiToFederate::Start {
                    start_unix_epoch_ns: 0,
                },
            )
            .await;
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Net {
                    federate_id: fed("source"),
                    tag: WireTag::ZERO,
                }
            );
            send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Msg {
                    source: fed("source"),
                    target: fed("sink"),
                    endpoint: protocol_endpoint(),
                    tag: WireTag::ZERO,
                    payload: b"7".to_vec(),
                }
            );
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Ltc {
                    federate_id: fed("source"),
                    tag: WireTag::ZERO,
                }
            );
        },
    )
    .await;

    let event_rx = empty_event_rx();
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    assert_eq!(
        barrier
            .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
            .unwrap()
            .map(|event| format!("{event:?}")),
        None
    );
    outbound
        .send(crate::FederatedOutboundCommand::Msg(
            crate::FederatedOutboundMessage {
                tag: boomerang_runtime::Tag::ZERO,
                payload: b"7".to_vec(),
            },
        ))
        .unwrap();
    barrier
        .report_logical_tag_complete(boomerang_runtime::Tag::ZERO)
        .unwrap();

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_schedules_inbound_msg_before_reporting_completion() {
    let (client, rti) = connect_client_with_fake_rti(fed("sink"), |mut transport| async move {
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Hello { federate_id, .. } if federate_id == fed("sink")
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        assert_eq!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed("sink"),
                tag: WireTag::ZERO,
            }
        );
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Msg {
                source: fed("source"),
                endpoint: protocol_endpoint(),
                tag: WireTag::ZERO,
                payload: b"42".to_vec(),
            },
        )
        .await;
        assert_eq!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Ltc {
                federate_id: fed("sink"),
                tag: WireTag::ZERO,
            }
        );
    })
    .await;

    let (inbound, event_rx, action_key, _shutdown_tx) = inbound_endpoint_for_u32();
    let mut inbound_route = route();
    inbound_route.bind_inbound(inbound);
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("sink"),
        client,
        [inbound_route],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    let event = barrier
        .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
        .unwrap()
        .expect("inbound MSG should interrupt the barrier wait");
    match event {
        boomerang_runtime::AsyncEvent::Logical { tag, key, value } => {
            assert_eq!(tag, boomerang_runtime::Tag::ZERO);
            assert_eq!(key, action_key);
            match value.downcast::<u32>() {
                Ok(value) => assert_eq!(*value, 42),
                Err(_) => panic!("expected u32 logical event payload"),
            }
        }
        event => panic!("expected logical async event, got {event:?}"),
    }
    barrier
        .report_logical_tag_complete(boomerang_runtime::Tag::ZERO)
        .unwrap();

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_admits_all_preceding_messages_before_consuming_tag() {
    let (client, rti) = connect_client_with_fake_rti(fed("sink"), |mut transport| async move {
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Hello { federate_id, .. } if federate_id == fed("sink")
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        for payload in [b"41".to_vec(), b"42".to_vec()] {
            send_rti_to_federate(
                &mut transport,
                RtiToFederate::Msg {
                    source: fed("source"),
                    endpoint: protocol_endpoint(),
                    tag: WireTag::ZERO,
                    payload,
                },
            )
            .await;
        }
        send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;

        assert_eq!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Ltc {
                federate_id: fed("sink"),
                tag: WireTag::ZERO,
            }
        );
    })
    .await;

    let (inbound, event_rx, action_key, _shutdown_tx) = inbound_endpoint_for_u32();
    let mut inbound_route = route();
    inbound_route.bind_inbound(inbound);
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("sink"),
        client,
        [inbound_route],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    for expected in [41, 42] {
        let event = barrier
            .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
            .unwrap()
            .expect("each preceding MSG must interrupt before TAG");
        let boomerang_runtime::AsyncEvent::Logical { tag, key, value } = event else {
            panic!("expected logical async event");
        };
        assert_eq!(tag, boomerang_runtime::Tag::ZERO);
        assert_eq!(key, action_key);
        match value.downcast::<u32>() {
            Ok(value) => assert_eq!(*value, expected),
            Err(_) => panic!("expected u32 payload"),
        }
    }
    assert!(barrier
        .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
        .unwrap()
        .is_none());
    barrier
        .report_logical_tag_complete(boomerang_runtime::Tag::ZERO)
        .unwrap();

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inbound_admission_failure_makes_the_coordinator_terminal_before_later_tag() {
    let (client, rti) = connect_client_with_fake_rti(fed("sink"), |mut transport| async move {
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Hello { federate_id, .. } if federate_id == fed("sink")
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Net {
                tag: WireTag::ZERO,
                ..
            }
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Msg {
                source: fed("source"),
                endpoint: protocol_endpoint(),
                tag: WireTag::ZERO,
                payload: b"not-a-u32".to_vec(),
            },
        )
        .await;
        send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
    })
    .await;

    let (inbound, event_rx, _action_key, _shutdown_tx) = inbound_endpoint_for_u32();
    let mut inbound_route = route();
    inbound_route.bind_inbound(inbound);
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("sink"),
        client,
        [inbound_route],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    assert!(matches!(
        barrier.wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx),
        Err(FederateClientError::RuntimeEndpoint(_))
    ));
    assert!(barrier.failed);
    assert!(matches!(
        barrier.wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx),
        Err(FederateClientError::CoordinationFailed)
    ));
    assert!(matches!(
        barrier.report_logical_tag_complete(boomerang_runtime::Tag::ZERO),
        Err(FederateClientError::CoordinationFailed)
    ));

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_does_not_repeat_pending_net_after_inbound_interruption() {
    let next_tag = WireTag::finite(1_000_000_000, 0);
    let (client, rti) =
        connect_client_with_fake_rti(fed("sink"), move |mut transport| async move {
            assert!(matches!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Hello { federate_id, .. } if federate_id == fed("sink")
            ));
            send_rti_to_federate(
                &mut transport,
                RtiToFederate::Start {
                    start_unix_epoch_ns: 0,
                },
            )
            .await;
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Net {
                    federate_id: fed("sink"),
                    tag: WireTag::ZERO,
                }
            );
            send_rti_to_federate(
                &mut transport,
                RtiToFederate::Msg {
                    source: fed("source"),
                    endpoint: protocol_endpoint(),
                    tag: WireTag::ZERO,
                    payload: b"42".to_vec(),
                },
            )
            .await;
            send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO }).await;
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Net {
                    federate_id: fed("sink"),
                    tag: next_tag,
                }
            );
            send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: next_tag }).await;
        })
        .await;

    let (inbound, event_rx, _action_key, _shutdown_tx) = inbound_endpoint_for_u32();
    let mut inbound_route = route();
    inbound_route.bind_inbound(inbound);
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("sink"),
        client,
        [inbound_route],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    assert!(barrier
        .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
        .unwrap()
        .is_some());
    assert!(barrier
        .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
        .unwrap()
        .is_none());
    assert!(barrier
        .wait_for_tag(
            boomerang_runtime::Tag::new(boomerang_runtime::Duration::seconds(1), 0),
            &event_rx,
        )
        .unwrap()
        .is_none());

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_reports_rti_error_frame() {
    let (client, rti) = connect_client_with_fake_rti(fed("source"), |mut transport| async move {
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Hello { federate_id, .. } if federate_id == fed("source")
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Net { .. }
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Error {
                message: "boom".into(),
            },
        )
        .await;
    })
    .await;

    let event_rx = empty_event_rx();
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    assert!(matches!(
        boomerang_runtime::LogicalTimeCoordinator::acquire(
            &mut barrier,
            boomerang_runtime::Tag::ZERO,
            &event_rx,
        ),
        Err(error) if error.to_string().contains("boom")
    ));
    assert_eq!(barrier.pending_request, None);

    rti.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_stop_sends_no_future_before_stop() {
    let (client, rti) = connect_client_with_fake_rti(fed("source"), |mut transport| async move {
        assert!(matches!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Hello { federate_id, .. } if federate_id == fed("source")
        ));
        send_rti_to_federate(
            &mut transport,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            },
        )
        .await;
        assert_eq!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Net {
                federate_id: fed("source"),
                tag: WireTag::FOREVER,
            }
        );
        assert_eq!(
            recv_federate_to_rti(&mut transport).await,
            FederateToRti::Stop {
                federate_id: fed("source"),
            }
        );
    })
    .await;

    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    barrier.stop().unwrap();
    assert_eq!(barrier.pending_request, None);

    rti.await.unwrap();
}
