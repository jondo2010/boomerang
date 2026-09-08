#[cfg(feature = "serde-json-codec")]
use std::collections::{BTreeMap, BTreeSet};

use futures_channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
#[cfg(feature = "serde-json-codec")]
use futures_util::stream::FuturesUnordered;
use futures_util::{stream::Map, StreamExt};
#[cfg(feature = "serde-json-codec")]
use tokio::net::{TcpListener, TcpStream};
#[cfg(feature = "serde-json-codec")]
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(feature = "serde-json-codec")]
use crate::{
    FederateId, FederatedTopology, ProtocolFrame, RtiSessionEndpoint, SessionError,
    StaticRtiSession,
};

#[cfg(feature = "serde-json-codec")]
pub type JsonProtocolFrameTransport = tokio_serde::SymmetricallyFramed<
    Framed<TcpStream, LengthDelimitedCodec>,
    ProtocolFrame,
    tokio_serde::formats::SymmetricalJson<ProtocolFrame>,
>;

#[cfg(feature = "serde-json-codec")]
pub type JsonProtocolFrameSink =
    futures_util::stream::SplitSink<JsonProtocolFrameTransport, ProtocolFrame>;

#[cfg(feature = "serde-json-codec")]
pub type JsonProtocolFrameStream = futures_util::stream::SplitStream<JsonProtocolFrameTransport>;

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

/// Accept a static topology's TCP federate connections and run the shared RTI session loop.
///
/// Accepted sockets are identified by their first `Hello` frame, independently of arrival order,
/// and then driven by [`StaticRtiSession`].
#[cfg(feature = "serde-json-codec")]
pub async fn run_tcp_static_rti_session(
    listener: TcpListener,
    topology: FederatedTopology,
) -> Result<(), SessionError> {
    run_tcp_static_rti_session_compiled(listener, crate::CompiledTopology::new(topology)?).await
}

#[cfg(feature = "serde-json-codec")]
pub(crate) async fn run_tcp_static_rti_session_compiled(
    listener: TcpListener,
    topology: crate::CompiledTopology,
) -> Result<(), SessionError> {
    let manifest = topology.topology();
    let expected = manifest.federates.iter().cloned().collect::<BTreeSet<_>>();
    if expected.len() != manifest.federates.len() {
        return Err(SessionError::Shutdown(
            "duplicate federate id in TCP topology".into(),
        ));
    }

    let mut accepted = Vec::with_capacity(expected.len());
    for peer_index in 0..expected.len() {
        let (stream, _) = listener.accept().await.map_err(|error| {
            SessionError::Shutdown(format!("failed to accept TCP federate connection: {error}"))
        })?;
        let (sink, stream) = json_protocol_frame_transport(stream).split();
        accepted.push((peer_index, sink, stream));
    }

    let mut first_frames = FuturesUnordered::new();
    for (peer_index, sink, mut stream) in accepted {
        first_frames.push(async move {
            let frame = stream
                .next()
                .await
                .ok_or_else(|| {
                    SessionError::Shutdown(format!(
                        "TCP peer {peer_index} closed before its Hello frame"
                    ))
                })?
                .map_err(|error| {
                    SessionError::Shutdown(format!(
                        "failed to read TCP peer {peer_index} Hello frame: {error}"
                    ))
                })?;
            Ok::<_, SessionError>((peer_index, sink, stream, frame))
        });
    }

    let mut endpoints = BTreeMap::<
        FederateId,
        RtiSessionEndpoint<JsonProtocolFrameSink, JsonProtocolFrameStream>,
    >::new();
    while let Some(first_frame) = first_frames.next().await {
        let (peer_index, sink, stream, frame) = first_frame?;
        let federate_id = match &frame {
            ProtocolFrame::FederateToRti(crate::FederateToRti::Hello { federate_id, .. }) => {
                federate_id.clone()
            }
            _ => {
                return Err(SessionError::Shutdown(format!(
                    "TCP peer {peer_index} sent a non-Hello first frame"
                )))
            }
        };
        if !expected.contains(&federate_id) {
            return Err(SessionError::Protocol {
                federate_id,
                message: "Hello declared an unknown federate id".into(),
            });
        }
        if endpoints
            .insert(
                federate_id.clone(),
                RtiSessionEndpoint::with_initial_frame(sink, stream, frame),
            )
            .is_some()
        {
            return Err(SessionError::Protocol {
                federate_id: federate_id.clone(),
                message: format!("duplicate Hello for federate `{federate_id}`"),
            });
        }
    }

    let observed = endpoints.keys().cloned().collect::<BTreeSet<_>>();
    let missing = expected.difference(&observed).collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(SessionError::Shutdown(format!(
            "TCP peers omitted expected federates: {missing:?}"
        )));
    }

    StaticRtiSession::from_compiled(topology, endpoints)
        .run()
        .await
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    use futures_util::{SinkExt, StreamExt};

    use super::*;
    use crate::{
        protocol::{
            EndpointId, FederateId, FederateToRti, FederatedTopology, RtiToFederate, TopologyEdge,
            WireDelay, WireTag,
        },
        FederateProtocolClient, NeighborStructure, ProtocolFrame,
    };

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut context = Context::from_waker(Waker::noop());
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "localhost TCP smoke test; run with `cargo test -p boomerang_federated tcp_smoke -- --ignored`"]
    async fn tcp_smoke_identifies_reverse_order_peers_by_hello() {
        use std::time::Duration as StdDuration;
        use tokio::net::{TcpListener, TcpStream};

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
        let rti = tokio::spawn(run_tcp_static_rti_session(listener, topology.clone()));

        let sink_stream = TcpStream::connect(addr).await.unwrap();
        let source_stream = TcpStream::connect(addr).await.unwrap();
        let source_connect = tokio::spawn(connect_tcp_client(
            source.clone(),
            topology.neighbors_for(&source),
            source_stream,
        ));
        let sink_connect = tokio::spawn(connect_tcp_client(
            sink.clone(),
            topology.neighbors_for(&sink),
            sink_stream,
        ));
        let source_client = source_connect.await.unwrap();
        let sink_client = sink_connect.await.unwrap();
        let recv_timeout = StdDuration::from_secs(1);

        sink_client
            .send(FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .unwrap();
        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            })
            .unwrap();
        assert_eq!(
            recv_rti_message(&source_client, recv_timeout, "source TAG(ZERO)"),
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );

        source_client
            .send(FederateToRti::Msg {
                source: source.clone(),
                target: sink.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello over tcp".to_vec(),
            })
            .unwrap();

        assert_eq!(
            recv_rti_message(&sink_client, recv_timeout, "routed sink MSG"),
            RtiToFederate::Msg {
                source: source.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello over tcp".to_vec(),
            }
        );

        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 1),
            })
            .unwrap();
        assert_eq!(
            recv_rti_message(&source_client, recv_timeout, "source TAG([0ns+1])"),
            RtiToFederate::Tag {
                tag: WireTag::finite(0, 1),
            }
        );
        assert_eq!(
            recv_rti_message(&sink_client, recv_timeout, "sink TAG(ZERO)"),
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );
        sink_client
            .send(FederateToRti::Ltc {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .unwrap();

        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::FOREVER,
            })
            .unwrap();
        source_client
            .send(FederateToRti::Stop {
                federate_id: source,
            })
            .unwrap();
        sink_client
            .send(FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::FOREVER,
            })
            .unwrap();
        sink_client
            .send(FederateToRti::Stop { federate_id: sink })
            .unwrap();
        assert_eq!(
            recv_rti_message(&source_client, recv_timeout, "source Stop"),
            RtiToFederate::Stop
        );
        assert_eq!(
            recv_rti_message(&sink_client, recv_timeout, "sink Stop"),
            RtiToFederate::Stop
        );

        drop(source_client);
        drop(sink_client);
        rti.await.unwrap().unwrap();
    }

    #[cfg(feature = "serde-json-codec")]
    async fn connect_tcp_client(
        federate_id: FederateId,
        topology: NeighborStructure,
        stream: TcpStream,
    ) -> FederateProtocolClient {
        let (sink, stream) = json_protocol_frame_transport(stream).split();
        FederateProtocolClient::connect(federate_id, topology, sink, stream)
            .await
            .unwrap()
    }

    #[cfg(feature = "serde-json-codec")]
    fn recv_rti_message(
        client: &FederateProtocolClient,
        timeout: std::time::Duration,
        label: &str,
    ) -> RtiToFederate {
        client
            .recv_timeout(timeout)
            .unwrap()
            .unwrap_or_else(|| panic!("timed out waiting for {label}"))
    }
}
