use std::{
    future::Future,
    pin::Pin,
    sync::mpsc::{self, Receiver, Sender},
};

#[cfg(feature = "serde-json-codec")]
use bytes::Bytes;
#[cfg(feature = "serde-json-codec")]
use futures_util::{SinkExt, StreamExt};
#[cfg(feature = "serde-json-codec")]
use tokio::net::{TcpStream, ToSocketAddrs};
#[cfg(feature = "serde-json-codec")]
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(feature = "serde-json-codec")]
use crate::ProtocolFrame;

pub type TransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[cfg(feature = "serde-json-codec")]
const DEFAULT_MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("transport peer is closed")]
    Closed,

    #[error("transport I/O error: {0}")]
    Io(String),

    #[error("transport frame codec error: {0}")]
    Codec(String),

    #[error("transport frame length {len} exceeds maximum {max}")]
    FrameTooLarge { len: usize, max: usize },
}

/// Async sink for ordered protocol frames.
pub trait FrameSink<M>: Send {
    fn send<'a>(&'a mut self, frame: M) -> TransportFuture<'a, Result<(), TransportError>>;
}

/// Async stream for ordered protocol frames.
pub trait FrameStream<M>: Send {
    fn recv<'a>(&'a mut self) -> TransportFuture<'a, Result<Option<M>, TransportError>>;
}

/// In-memory ordered transport for deterministic protocol tests.
#[derive(Debug)]
pub struct InMemoryTransport<Outgoing, Incoming> {
    sender: Sender<Outgoing>,
    receiver: Receiver<Incoming>,
}

impl<Outgoing, Incoming> InMemoryTransport<Outgoing, Incoming> {
    fn new(sender: Sender<Outgoing>, receiver: Receiver<Incoming>) -> Self {
        Self { sender, receiver }
    }
}

pub fn in_memory_transport_pair<A, B>() -> (InMemoryTransport<A, B>, InMemoryTransport<B, A>) {
    let (a_sender, a_receiver) = mpsc::channel();
    let (b_sender, b_receiver) = mpsc::channel();

    (
        InMemoryTransport::new(a_sender, b_receiver),
        InMemoryTransport::new(b_sender, a_receiver),
    )
}

/// Length-delimited TCP transport for serde JSON encoded [`ProtocolFrame`] values.
///
/// Each frame is encoded as a big-endian `u32` byte length followed by that many JSON bytes. The
/// transport is reliable and ordered because it is backed by a single TCP stream.
#[cfg(feature = "serde-json-codec")]
#[derive(Debug)]
pub struct TcpTransport {
    framed: Framed<TcpStream, LengthDelimitedCodec>,
    max_frame_len: usize,
}

#[cfg(feature = "serde-json-codec")]
impl TcpTransport {
    pub fn from_stream(stream: TcpStream) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(DEFAULT_MAX_FRAME_LEN)
            .new_codec();
        Self {
            framed: Framed::new(stream, codec),
            max_frame_len: DEFAULT_MAX_FRAME_LEN,
        }
    }

    pub fn with_max_frame_len(mut self, max_frame_len: usize) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(max_frame_len)
            .new_codec();
        self.framed = self.framed.map_codec(|_| codec);
        self.max_frame_len = max_frame_len;
        self
    }

    pub async fn connect(addr: impl ToSocketAddrs) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|error| TransportError::Io(error.to_string()))?;
        Ok(Self::from_stream(stream))
    }

    pub fn max_frame_len(&self) -> usize {
        self.max_frame_len
    }
}

#[cfg(feature = "serde-json-codec")]
impl FrameSink<ProtocolFrame> for TcpTransport {
    fn send<'a>(
        &'a mut self,
        frame: ProtocolFrame,
    ) -> TransportFuture<'a, Result<(), TransportError>> {
        Box::pin(async move {
            let payload = serde_json::to_vec(&frame)
                .map_err(|error| TransportError::Codec(error.to_string()))?;
            if payload.len() > self.max_frame_len {
                return Err(TransportError::FrameTooLarge {
                    len: payload.len(),
                    max: self.max_frame_len,
                });
            }
            if payload.len() > u32::MAX as usize {
                return Err(TransportError::FrameTooLarge {
                    len: payload.len(),
                    max: u32::MAX as usize,
                });
            }

            self.framed
                .send(Bytes::from(payload))
                .await
                .map_err(map_io_error)?;
            Ok(())
        })
    }
}

#[cfg(feature = "serde-json-codec")]
impl FrameStream<ProtocolFrame> for TcpTransport {
    fn recv<'a>(
        &'a mut self,
    ) -> TransportFuture<'a, Result<Option<ProtocolFrame>, TransportError>> {
        Box::pin(async move {
            let Some(payload) = self.framed.next().await.transpose().map_err(map_io_error)? else {
                return Ok(None);
            };
            let frame = serde_json::from_slice(&payload)
                .map_err(|error| TransportError::Codec(error.to_string()))?;
            Ok(Some(frame))
        })
    }
}

#[cfg(feature = "serde-json-codec")]
fn map_io_error(error: std::io::Error) -> TransportError {
    match error.kind() {
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::UnexpectedEof => TransportError::Closed,
        _ => TransportError::Io(error.to_string()),
    }
}

impl<Outgoing, Incoming> FrameSink<Outgoing> for InMemoryTransport<Outgoing, Incoming>
where
    Outgoing: Send + 'static,
    Incoming: Send + 'static,
{
    fn send<'a>(&'a mut self, frame: Outgoing) -> TransportFuture<'a, Result<(), TransportError>> {
        Box::pin(async move { self.sender.send(frame).map_err(|_| TransportError::Closed) })
    }
}

impl<Outgoing, Incoming> FrameStream<Incoming> for InMemoryTransport<Outgoing, Incoming>
where
    Outgoing: Send + 'static,
    Incoming: Send + 'static,
{
    fn recv<'a>(&'a mut self) -> TransportFuture<'a, Result<Option<Incoming>, TransportError>> {
        Box::pin(async move {
            match self.receiver.recv() {
                Ok(frame) => Ok(Some(frame)),
                Err(_) => Ok(None),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

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
        let (mut federate, mut rti) = in_memory_transport_pair::<FederateToRti, RtiToFederate>();

        block_on(federate.send(FederateToRti::Net {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();
        block_on(federate.send(FederateToRti::Ltc {
            federate_id: FederateId::new("fed-a"),
            tag: WireTag::ZERO,
        }))
        .unwrap();

        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })
        );
        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(FederateToRti::Ltc {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })
        );
    }

    #[test]
    fn memory_transport_is_bidirectional() {
        let (mut federate, mut rti) = in_memory_transport_pair::<ProtocolFrame, ProtocolFrame>();

        block_on(
            federate.send(ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            })),
        )
        .unwrap();
        block_on(rti.send(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
            tag: WireTag::ZERO,
        })))
        .unwrap();

        assert_eq!(
            block_on(rti.recv()).unwrap(),
            Some(ProtocolFrame::FederateToRti(FederateToRti::Net {
                federate_id: FederateId::new("fed-a"),
                tag: WireTag::ZERO,
            }))
        );
        assert_eq!(
            block_on(federate.recv()).unwrap(),
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Tag {
                tag: WireTag::ZERO,
            }))
        );
    }

    #[test]
    fn memory_transport_reports_end_of_stream_when_peer_drops() {
        let (federate, mut rti) = in_memory_transport_pair::<FederateToRti, RtiToFederate>();
        drop(federate);

        assert_eq!(block_on(rti.recv()).unwrap(), None);
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
            let mut first = TcpTransport::from_stream(first_stream);
            let first_id = recv_hello(&mut first, &rti_topology).await;

            let (second_stream, _) = listener.accept().await.unwrap();
            let mut second = TcpTransport::from_stream(second_stream);
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

            let frame = source_transport.recv().await.unwrap().unwrap();
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
    async fn recv_hello(transport: &mut TcpTransport, topology: &FederatedTopology) -> FederateId {
        match transport.recv().await.unwrap().unwrap() {
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
    async fn expect_start(transport: &mut TcpTransport) {
        assert_eq!(
            transport.recv().await.unwrap(),
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }))
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn expect_stop(transport: &mut TcpTransport, federate_id: &FederateId) {
        assert_eq!(
            transport.recv().await.unwrap(),
            Some(ProtocolFrame::FederateToRti(FederateToRti::Stop {
                federate_id: federate_id.clone(),
            }))
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
        let mut transport = TcpTransport::connect(addr).await.unwrap();
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
            transport.recv().await.unwrap(),
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Stop))
        );
    }

    #[cfg(feature = "serde-json-codec")]
    async fn run_sink_client(
        addr: std::net::SocketAddr,
        sink: FederateId,
        topology: crate::NeighborStructure,
    ) -> RtiToFederate {
        let mut transport = TcpTransport::connect(addr).await.unwrap();
        transport
            .send(ProtocolFrame::FederateToRti(FederateToRti::Hello {
                federate_id: sink.clone(),
                topology,
            }))
            .await
            .unwrap();
        expect_start(&mut transport).await;

        let delivered = match transport.recv().await.unwrap().unwrap() {
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
            transport.recv().await.unwrap(),
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Stop))
        );
        delivered
    }
}
