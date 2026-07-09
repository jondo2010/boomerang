use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures_util::{stream::Map, StreamExt};
#[cfg(feature = "serde-json-codec")]
use tokio::net::TcpStream;
#[cfg(feature = "serde-json-codec")]
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(feature = "serde-json-codec")]
use crate::ProtocolFrame;

#[cfg(feature = "serde-json-codec")]
pub type JsonProtocolFrameTransport = tokio_serde::SymmetricallyFramed<
    Framed<TcpStream, LengthDelimitedCodec>,
    ProtocolFrame,
    tokio_serde::formats::SymmetricalJson<ProtocolFrame>,
>;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("transport peer is closed")]
    Closed,

    #[error("transport I/O error: {0}")]
    Io(String),

    #[error("transport frame codec error: {0}")]
    Codec(String),
}

impl From<mpsc::SendError> for TransportError {
    fn from(_: mpsc::SendError) -> Self {
        Self::Closed
    }
}

impl From<std::io::Error> for TransportError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Sending half of an in-memory ordered transport.
pub type InMemoryFrameSink<M> = UnboundedSender<M>;

/// Receiving half of an in-memory ordered transport.
pub type InMemoryFrameStream<M> = Map<UnboundedReceiver<M>, fn(M) -> Result<M, TransportError>>;

/// In-memory ordered transport for deterministic protocol tests.
pub type InMemoryTransport<Outgoing, Incoming> =
    (InMemoryFrameSink<Outgoing>, InMemoryFrameStream<Incoming>);

pub fn in_memory_transport_pair<A, B>() -> (InMemoryTransport<A, B>, InMemoryTransport<B, A>) {
    let (a_sender, a_receiver) = mpsc::unbounded();
    let (b_sender, b_receiver) = mpsc::unbounded();

    (
        (
            a_sender,
            b_receiver.map(ok_frame::<B> as fn(B) -> Result<B, TransportError>),
        ),
        (
            b_sender,
            a_receiver.map(ok_frame::<A> as fn(A) -> Result<A, TransportError>),
        ),
    )
}

fn ok_frame<M>(frame: M) -> Result<M, TransportError> {
    Ok(frame)
}

/// Length-delimited TCP transport for serde JSON encoded [`ProtocolFrame`] values.
///
/// Each frame is encoded as a big-endian `u32` byte length followed by that many JSON bytes. The
/// transport is reliable and ordered because it is backed by a single TCP stream.
#[cfg(feature = "serde-json-codec")]
pub fn json_protocol_frame_transport(stream: TcpStream) -> JsonProtocolFrameTransport {
    tokio_serde::SymmetricallyFramed::new(
        Framed::new(stream, LengthDelimitedCodec::new()),
        tokio_serde::formats::SymmetricalJson::default(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    use futures_util::{SinkExt, StreamExt};

    use super::*;
    use crate::{
        protocol::{
            EndpointId, FederateId, FederateToRti, FederatedTopology, RtiToFederate, TopologyEdge,
            WireDelay, WireTag,
        },
        ProtocolFrame,
    };

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    #[test]
    fn memory_transport_delivers_frames_in_order() {
        let ((mut federate_sink, _), (_, mut rti_stream)) =
            in_memory_transport_pair::<FederateToRti, RtiToFederate>();

        block_on(federate_sink.send(FederateToRti::Net {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();
        block_on(federate_sink.send(FederateToRti::Ltc {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();

        assert_eq!(
            block_on(rti_stream.next()).unwrap().unwrap(),
            FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            }
        );
        assert_eq!(
            block_on(rti_stream.next()).unwrap().unwrap(),
            FederateToRti::Ltc {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            }
        );
    }

    #[test]
    fn memory_transport_is_bidirectional() {
        let ((mut federate_sink, mut federate_stream), (mut rti_sink, mut rti_stream)) =
            in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();

        block_on(
            federate_sink.send(ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })),
        )
        .unwrap();
        block_on(
            rti_sink.send(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
                tag: WireTag::ZERO,
            })),
        )
        .unwrap();

        assert_eq!(
            block_on(rti_stream.next()).unwrap().unwrap(),
            ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })
        );
        assert_eq!(
            block_on(federate_stream.next()).unwrap().unwrap(),
            ProtocolFrame::RtiToFederate(RtiToFederate::Tag { tag: WireTag::ZERO })
        );
    }

    #[test]
    fn memory_transport_reports_end_of_stream_when_peer_drops() {
        let (federate, (_, mut rti_stream)) =
            in_memory_transport_pair::<FederateToRti, RtiToFederate>();
        drop(federate);

        assert_eq!(block_on(rti_stream.next()), None);
    }

    #[cfg(feature = "serde-json-codec")]
    #[tokio::test]
    #[ignore = "localhost TCP smoke test; run with `cargo test -p boomerang_federated tcp_smoke -- --ignored`"]
    async fn tcp_smoke_routes_msg_through_rti_and_shuts_down() {
        use tokio::net::TcpListener;

        let source = FederateId::new("source");
        let sink = FederateId::new("sink");
        let endpoint = EndpointId::new("source.out->sink.in");
        let topology = FederatedTopology::with_edges(
            [source.clone(), sink.clone()],
            [TopologyEdge::new(
                source.clone(),
                sink.clone(),
                endpoint.clone(),
                WireDelay::ZERO,
            )],
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let rti_topology = topology.clone();
        let rti_source = source.clone();
        let rti_sink = sink.clone();
        let rti = tokio::spawn(async move {
            let (first_stream, _) = listener.accept().await.unwrap();
            let mut first = json_protocol_frame_transport(first_stream);
            let first_id = recv_hello(&mut first, &rti_topology).await;

            let (second_stream, _) = listener.accept().await.unwrap();
            let mut second = json_protocol_frame_transport(second_stream);
            let second_id = recv_hello(&mut second, &rti_topology).await;

            let (mut source_transport, mut sink_transport) = match (first_id, second_id) {
                (id, other) if id == rti_source && other == rti_sink => (first, second),
                (id, other) if id == rti_sink && other == rti_source => (second, first),
                (id, other) => panic!("unexpected federates {id:?} and {other:?}"),
            };

            let start = ProtocolFrame::RtiToFederate(RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            });
            source_transport.send(start.clone()).await.unwrap();
            sink_transport.send(start).await.unwrap();

            let frame = source_transport.next().await.unwrap().unwrap();
            let deliveries = match frame {
                ProtocolFrame::FederateToRti(message @ FederateToRti::Msg { .. }) => {
                    let mut rti = crate::RtiState::new(rti_topology);
                    rti.handle(message).unwrap()
                }
                other => panic!("expected source MSG, got {other:?}"),
            };

            assert_eq!(deliveries.len(), 1);
            let delivery = deliveries.into_iter().next().unwrap();
            assert_eq!(delivery.federate_id, rti_sink);
            sink_transport
                .send(ProtocolFrame::RtiToFederate(delivery.message))
                .await
                .unwrap();

            expect_stop(&mut source_transport, &rti_source).await;
            expect_stop(&mut sink_transport, &rti_sink).await;
            source_transport
                .send(ProtocolFrame::RtiToFederate(RtiToFederate::Stop))
                .await
                .unwrap();
            sink_transport
                .send(ProtocolFrame::RtiToFederate(RtiToFederate::Stop))
                .await
                .unwrap();
        });

        let source_client = tokio::spawn(run_source_client(
            addr,
            source.clone(),
            sink.clone(),
            endpoint.clone(),
            topology.neighbors_for(&source),
        ));
        let sink_client = tokio::spawn(run_sink_client(
            addr,
            sink.clone(),
            topology.neighbors_for(&sink),
        ));

        source_client.await.unwrap();
        let delivered = sink_client.await.unwrap();
        rti.await.unwrap();

        assert_eq!(
            delivered,
            RtiToFederate::Msg {
                source,
                endpoint,
                tag: WireTag::ZERO,
                payload: b"hello over tcp".to_vec(),
            }
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn recv_hello(
        transport: &mut JsonProtocolFrameTransport,
        topology: &FederatedTopology,
    ) -> FederateId {
        match transport.next().await.unwrap().unwrap() {
            ProtocolFrame::FederateToRti(FederateToRti::Hello {
                federate_id,
                topology: neighbor_structure,
            }) => {
                assert_eq!(neighbor_structure, topology.neighbors_for(&federate_id));
                federate_id
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[cfg(feature = "serde-json-codec")]
    async fn expect_start(transport: &mut JsonProtocolFrameTransport) {
        assert_eq!(
            transport.next().await.unwrap().unwrap(),
            ProtocolFrame::RtiToFederate(RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            })
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn expect_stop(transport: &mut JsonProtocolFrameTransport, federate_id: &FederateId) {
        assert_eq!(
            transport.next().await.unwrap().unwrap(),
            ProtocolFrame::FederateToRti(FederateToRti::Stop {
                federate_id: federate_id.clone(),
            })
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn run_source_client(
        addr: std::net::SocketAddr,
        source: FederateId,
        sink: FederateId,
        endpoint: EndpointId,
        topology: crate::NeighborStructure,
    ) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = json_protocol_frame_transport(stream);
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Hello {
                federate_id: source.clone(),
                topology,
            }))
            .await
            .unwrap();
        expect_start(&mut transport).await;
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Msg {
                source: source.clone(),
                target: sink,
                endpoint,
                tag: WireTag::ZERO,
                payload: b"hello over tcp".to_vec(),
            }))
            .await
            .unwrap();
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Stop {
                federate_id: source,
            }))
            .await
            .unwrap();
        assert_eq!(
            transport.next().await.unwrap().unwrap(),
            ProtocolFrame::RtiToFederate(RtiToFederate::Stop)
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn run_sink_client(
        addr: std::net::SocketAddr,
        sink: FederateId,
        topology: crate::NeighborStructure,
    ) -> RtiToFederate {
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = json_protocol_frame_transport(stream);
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Hello {
                federate_id: sink.clone(),
                topology,
            }))
            .await
            .unwrap();
        expect_start(&mut transport).await;

        let delivered = match transport.next().await.unwrap().unwrap() {
            ProtocolFrame::RtiToFederate(message @ RtiToFederate::Msg { .. }) => message,
            other => panic!("expected routed MSG, got {other:?}"),
        };
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Stop {
                federate_id: sink,
            }))
            .await
            .unwrap();
        assert_eq!(
            transport.next().await.unwrap().unwrap(),
            ProtocolFrame::RtiToFederate(RtiToFederate::Stop)
        );
        delivered
    }
}
