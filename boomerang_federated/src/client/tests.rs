use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Waker},
    time::{Duration as StdDuration, Instant},
};

use futures_util::{Sink, SinkExt, StreamExt};

use super::*;
use crate::client::coordination::ProtocolPoll;
use crate::{in_memory_transport_pair, EndpointId};

fn fed(id: &str) -> FederateId {
    FederateId::new(id)
}

fn endpoint() -> EndpointId {
    EndpointId::new("source.out->sink.in")
}

fn protocol_endpoint() -> EndpointId {
    endpoint()
}

fn route() -> FederateClientRoute {
    FederateClientRoute::new(endpoint(), fed("source"), fed("sink"))
}

#[derive(Debug, Default)]
struct DeliveryGate {
    open: AtomicBool,
    blocked: AtomicBool,
    fail: AtomicBool,
    waker: Mutex<Option<Waker>>,
    delivered: Mutex<Vec<ProtocolFrame>>,
}

impl DeliveryGate {
    fn open(&self) {
        self.open.store(true, Ordering::Release);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }

    fn close(&self) {
        self.open.store(false, Ordering::Release);
        self.blocked.store(false, Ordering::Release);
    }

    fn fail(&self) {
        self.fail.store(true, Ordering::Release);
    }

    async fn wait_until_blocked(&self) {
        let deadline = Instant::now() + StdDuration::from_secs(1);
        while !self.blocked.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "writer did not reach the gated transport flush"
            );
            tokio::task::yield_now().await;
        }
    }
}

#[derive(Debug)]
struct GatedSink {
    gate: Arc<DeliveryGate>,
    pending: Vec<ProtocolFrame>,
}

impl GatedSink {
    fn new(gate: Arc<DeliveryGate>) -> Self {
        gate.open();
        Self {
            gate,
            pending: Vec::new(),
        }
    }
}

impl Sink<ProtocolFrame> for GatedSink {
    type Error = crate::TransportError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: ProtocolFrame) -> Result<(), Self::Error> {
        self.pending.push(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.gate.fail.load(Ordering::Acquire) {
            self.pending.clear();
            return Poll::Ready(Err(crate::TransportError::Closed));
        }
        if !self.gate.open.load(Ordering::Acquire) {
            let mut waker = self.gate.waker.lock().unwrap();
            *waker = Some(cx.waker().clone());
            if !self.gate.open.load(Ordering::Acquire) {
                self.gate.blocked.store(true, Ordering::Release);
                return Poll::Pending;
            }
            waker.take();
        }
        let delivered = self.pending.drain(..).collect::<Vec<_>>();
        self.gate.delivered.lock().unwrap().extend(delivered);
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }
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
    let (client_transport, rti_transport) = in_memory_transport_pair();
    let handle = tokio::spawn(rti(rti_transport));
    let (sink, stream) = client_transport;
    let client =
        FederateProtocolClient::connect_with_mailbox(federate_id.clone(), sink, stream, mailbox)
            .await
            .unwrap();
    assert_eq!(client.start_unix_epoch_ns(), 0);
    (client, handle)
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
    let (outbound, _) = connections
        .outbound_endpoint(&fed("sink"), &endpoint())
        .unwrap();
    let mailbox = connections.take_mailbox(&fed("source")).unwrap();
    let (client, rti) = connect_client_with_fake_rti_and_mailbox(
        fed("source"),
        mailbox,
        |mut transport| async move {
            assert_eq!(
                recv_federate_to_rti(&mut transport).await,
                FederateToRti::Hello {
                    federate_id: fed("source"),
                },
            );
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

    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    loop {
        match barrier.poll().unwrap() {
            ProtocolPoll::Granted(tag) => {
                assert_eq!(tag, boomerang_runtime::Tag::ZERO);
                break;
            }
            ProtocolPoll::Pending | ProtocolPoll::Progress => {}
        }
    }
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

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    assert_eq!(barrier.poll().unwrap(), ProtocolPoll::Progress);
    let event = event_rx
        .recv()
        .expect("inbound MSG should schedule an event");
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

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    for expected in [41, 42] {
        assert_eq!(barrier.poll().unwrap(), ProtocolPoll::Progress);
        let event = event_rx
            .recv()
            .expect("each preceding MSG must schedule before TAG");
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
    assert_eq!(
        barrier.poll().unwrap(),
        ProtocolPoll::Granted(boomerang_runtime::Tag::ZERO)
    );
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

    let (inbound, _event_rx, _action_key, _shutdown_tx) = inbound_endpoint_for_u32();
    let mut inbound_route = route();
    inbound_route.bind_inbound(inbound);
    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("sink"),
        client,
        [inbound_route],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    assert!(matches!(
        barrier.poll(),
        Err(FederateClientError::RuntimeEndpoint(_))
    ));
    assert!(barrier.failed);
    assert!(matches!(
        barrier.submit_net(boomerang_runtime::Tag::ZERO),
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

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    assert_eq!(barrier.poll().unwrap(), ProtocolPoll::Progress);
    assert!(event_rx.recv().is_ok());
    assert_eq!(
        barrier.poll().unwrap(),
        ProtocolPoll::Granted(boomerang_runtime::Tag::ZERO)
    );
    let next = boomerang_runtime::Tag::new(boomerang_runtime::Duration::seconds(1), 0);
    barrier.submit_net(next).unwrap();
    assert_eq!(barrier.poll().unwrap(), ProtocolPoll::Granted(next));

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

    let mut barrier = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();

    barrier.submit_net(boomerang_runtime::Tag::ZERO).unwrap();
    let error = loop {
        match barrier.poll() {
            Err(error) => break error,
            Ok(ProtocolPoll::Pending | ProtocolPoll::Progress) => {}
            Ok(ProtocolPoll::Granted(tag)) => panic!("unexpected TAG {tag}"),
        }
    };
    assert!(error.to_string().contains("boom"));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_stop_waits_for_terminal_transport_delivery() {
    let (client_transport, mut rti_transport) = in_memory_transport_pair();
    send_rti_to_federate(
        &mut rti_transport,
        RtiToFederate::Start {
            start_unix_epoch_ns: 0,
        },
    )
    .await;

    let gate = Arc::new(DeliveryGate::default());
    let client = FederateProtocolClient::connect(
        fed("source"),
        GatedSink::new(gate.clone()),
        client_transport.1,
    )
    .await
    .unwrap();
    gate.close();

    let mut coordinator = RtiLogicalTimeCoordinator::new(
        fed("source"),
        client,
        [route()],
        crate::FederatedFaultState::default(),
    )
    .unwrap();
    let (returned_tx, returned_rx) = std::sync::mpsc::channel();
    let stop = tokio::task::spawn_blocking(move || {
        let result = coordinator.stop();
        returned_tx.send(()).unwrap();
        (result, coordinator)
    });

    gate.wait_until_blocked().await;
    assert!(
        matches!(
            returned_rx.recv_timeout(StdDuration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "stop returned before the terminal frames reached the transport"
    );

    gate.open();
    let (result, coordinator) = stop.await.unwrap();
    result.unwrap();
    drop(coordinator);

    assert_eq!(
        *gate.delivered.lock().unwrap(),
        [
            ProtocolFrame::FederateToRti(FederateToRti::Hello {
                federate_id: fed("source"),
            }),
            ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: fed("source"),
                tag: WireTag::FOREVER,
            }),
            ProtocolFrame::FederateToRti(FederateToRti::Stop {
                federate_id: fed("source"),
            }),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_delivery_times_out_when_transport_does_not_flush() {
    let (client_transport, mut rti_transport) = in_memory_transport_pair();
    send_rti_to_federate(
        &mut rti_transport,
        RtiToFederate::Start {
            start_unix_epoch_ns: 0,
        },
    )
    .await;

    let gate = Arc::new(DeliveryGate::default());
    let client = FederateProtocolClient::connect(
        fed("source"),
        GatedSink::new(gate.clone()),
        client_transport.1,
    )
    .await
    .unwrap();
    gate.close();

    let delivery = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let result = client.send_confirmed(
            FederateToRti::Stop {
                federate_id: fed("source"),
            },
            StdDuration::from_millis(20),
        );
        (result, started.elapsed(), client)
    });

    gate.wait_until_blocked().await;
    let (result, elapsed, client) = delivery.await.unwrap();
    assert!(matches!(result, Err(FederateClientError::DeliveryTimeout)));
    assert!(elapsed < StdDuration::from_secs(1));

    gate.open();
    drop(client);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_transport_failure_reaches_caller_and_client_input() {
    let (client_transport, mut rti_transport) = in_memory_transport_pair();
    send_rti_to_federate(
        &mut rti_transport,
        RtiToFederate::Start {
            start_unix_epoch_ns: 0,
        },
    )
    .await;

    let gate = Arc::new(DeliveryGate::default());
    let client = FederateProtocolClient::connect(
        fed("source"),
        GatedSink::new(gate.clone()),
        client_transport.1,
    )
    .await
    .unwrap();
    gate.fail();

    let (delivery_error, input_error, client) = tokio::task::spawn_blocking(move || {
        let delivery_error = client
            .send_confirmed(
                FederateToRti::Stop {
                    federate_id: fed("source"),
                },
                StdDuration::from_secs(1),
            )
            .unwrap_err();
        let input_error = client.recv_timeout(StdDuration::from_secs(1)).unwrap_err();
        (delivery_error, input_error, client)
    })
    .await
    .unwrap();

    assert!(matches!(
        delivery_error,
        FederateClientError::Transport(crate::TransportError::Closed)
    ));
    assert!(matches!(
        input_error,
        FederateClientError::Transport(crate::TransportError::Closed)
    ));
    drop(client);
}
