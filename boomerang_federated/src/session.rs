use std::collections::{BTreeMap, BTreeSet};

use futures_util::{Sink, SinkExt, TryStream, TryStreamExt};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    rti::{RtiDelivery, RtiError, RtiState},
    FederateId, FederateToRti, FederatedTopology, ProtocolFrame, RtiToFederate, TransportError,
};

/// One federate's RTI-side transport halves.
#[derive(Debug)]
pub struct RtiSessionEndpoint<S, R> {
    sink: S,
    stream: R,
    initial_frame: Option<ProtocolFrame>,
}

impl<S, R> RtiSessionEndpoint<S, R> {
    pub fn new(sink: S, stream: R) -> Self {
        Self {
            sink,
            stream,
            initial_frame: None,
        }
    }

    /// Construct an endpoint whose first frame was consumed while identifying the peer.
    pub fn with_initial_frame(sink: S, stream: R, frame: ProtocolFrame) -> Self {
        Self {
            sink,
            stream,
            initial_frame: Some(frame),
        }
    }
}

/// A static in-memory RTI session for persistent federates.
#[derive(Debug)]
pub struct StaticRtiSession<S, R> {
    rti: RtiState,
    endpoints: BTreeMap<FederateId, RtiSessionEndpoint<S, R>>,
    start_unix_epoch_ns: i128,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("transport error for federate `{federate_id}`: {source}")]
    Transport {
        federate_id: FederateId,
        source: TransportError,
    },

    #[error("RTI error: {0}")]
    Rti(#[from] RtiError),

    #[error("protocol error for federate `{federate_id}`: {message}")]
    Protocol {
        federate_id: FederateId,
        message: String,
    },

    #[error("session shutdown error: {0}")]
    Shutdown(String),
}

enum SessionInput {
    Frame {
        federate_id: FederateId,
        frame: ProtocolFrame,
    },
    Closed {
        federate_id: FederateId,
    },
    TransportError {
        federate_id: FederateId,
        error: TransportError,
    },
}

impl<S, R> StaticRtiSession<S, R>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError>,
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError>,
{
    pub fn new(
        topology: FederatedTopology,
        endpoints: BTreeMap<FederateId, RtiSessionEndpoint<S, R>>,
    ) -> Self {
        Self {
            rti: RtiState::new(topology),
            endpoints,
            start_unix_epoch_ns: 0,
        }
    }

    pub fn with_start_unix_epoch_ns(mut self, start_unix_epoch_ns: i128) -> Self {
        self.start_unix_epoch_ns = start_unix_epoch_ns;
        self
    }

    pub async fn run(mut self) -> Result<(), SessionError> {
        let expected = expected_federates(self.rti.topology());
        self.validate_endpoint_set(&expected)?;

        let (input_tx, mut input_rx) = mpsc::unbounded_channel();
        let mut sinks = BTreeMap::new();
        let endpoints = std::mem::take(&mut self.endpoints);
        for (federate_id, endpoint) in endpoints {
            if let Some(frame) = endpoint.initial_frame {
                input_tx
                    .send(SessionInput::Frame {
                        federate_id: federate_id.clone(),
                        frame,
                    })
                    .map_err(|_| {
                        SessionError::Shutdown(
                            "session input closed while queueing an initial Hello".into(),
                        )
                    })?;
            }
            sinks.insert(federate_id.clone(), endpoint.sink);
            spawn_stream_reader(federate_id, endpoint.stream, input_tx.clone());
        }
        drop(input_tx);

        self.receive_hellos(&mut input_rx, &mut sinks, &expected)
            .await?;
        self.send_start(&mut sinks, &expected).await?;
        self.run_protocol_loop(&mut input_rx, &mut sinks, &expected)
            .await
    }

    fn validate_endpoint_set(&self, expected: &BTreeSet<FederateId>) -> Result<(), SessionError> {
        let observed = self.endpoints.keys().cloned().collect::<BTreeSet<_>>();
        if &observed == expected {
            return Ok(());
        }

        let missing = expected
            .difference(&observed)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let unexpected = observed
            .difference(expected)
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        Err(SessionError::Shutdown(format!(
            "session endpoint set does not match topology; missing={missing:?}, unexpected={unexpected:?}"
        )))
    }

    async fn receive_hellos(
        &mut self,
        input_rx: &mut mpsc::UnboundedReceiver<SessionInput>,
        sinks: &mut BTreeMap<FederateId, S>,
        expected: &BTreeSet<FederateId>,
    ) -> Result<(), SessionError> {
        let mut seen = BTreeSet::new();
        while seen.len() < expected.len() {
            let input = receive_session_input(input_rx).await?;
            match input {
                SessionInput::Frame { federate_id, frame } => {
                    let ProtocolFrame::FederateToRti(FederateToRti::Hello {
                        federate_id: hello_federate,
                        topology,
                    }) = frame
                    else {
                        return protocol_error(sinks, &federate_id, "expected Hello before Start")
                            .await;
                    };

                    if hello_federate != federate_id {
                        return protocol_error(
                            sinks,
                            &federate_id,
                            format!(
                                "Hello identified federate `{hello_federate}`, but endpoint is `{federate_id}`"
                            ),
                        )
                        .await;
                    }
                    if !seen.insert(federate_id.clone()) {
                        return protocol_error(sinks, &federate_id, "duplicate Hello").await;
                    }

                    let expected_topology = self.rti.topology().neighbors_for(&federate_id);
                    if topology != expected_topology {
                        return protocol_error(
                            sinks,
                            &federate_id,
                            "Hello neighbor structure does not match RTI topology",
                        )
                        .await;
                    }

                    self.rti
                        .handle(FederateToRti::Hello {
                            federate_id,
                            topology,
                        })
                        .map_err(SessionError::Rti)?;
                }
                SessionInput::Closed { federate_id } => {
                    return Err(SessionError::Shutdown(format!(
                        "federate `{federate_id}` closed before Hello"
                    )));
                }
                SessionInput::TransportError { federate_id, error } => {
                    return Err(SessionError::Transport {
                        federate_id,
                        source: error,
                    });
                }
            }
        }

        Ok(())
    }

    async fn send_start(
        &mut self,
        sinks: &mut BTreeMap<FederateId, S>,
        expected: &BTreeSet<FederateId>,
    ) -> Result<(), SessionError> {
        for federate_id in expected {
            send_frame(
                sinks,
                federate_id,
                ProtocolFrame::RtiToFederate(RtiToFederate::Start {
                    start_unix_epoch_ns: self.start_unix_epoch_ns,
                }),
            )
            .await?;
        }

        Ok(())
    }

    async fn run_protocol_loop(
        &mut self,
        input_rx: &mut mpsc::UnboundedReceiver<SessionInput>,
        sinks: &mut BTreeMap<FederateId, S>,
        expected: &BTreeSet<FederateId>,
    ) -> Result<(), SessionError> {
        let mut stopped = BTreeSet::new();

        while stopped.len() < expected.len() {
            let input = receive_session_input(input_rx).await?;
            match input {
                SessionInput::Frame { federate_id, frame } => {
                    if stopped.contains(&federate_id) {
                        return protocol_error(
                            sinks,
                            &federate_id,
                            "received protocol frame after Stop",
                        )
                        .await;
                    }

                    let ProtocolFrame::FederateToRti(message) = frame else {
                        return protocol_error(
                            sinks,
                            &federate_id,
                            "federate sent an RTI-to-federate frame",
                        )
                        .await;
                    };

                    if let Err(message) = self.validate_federate_message(&federate_id, &message) {
                        return protocol_error(sinks, &federate_id, message).await;
                    }

                    if matches!(message, FederateToRti::Hello { .. }) {
                        return protocol_error(sinks, &federate_id, "unexpected Hello after Start")
                            .await;
                    }

                    if matches!(message, FederateToRti::Stop { .. }) {
                        let deliveries = match self.rti.handle(message) {
                            Ok(deliveries) => deliveries,
                            Err(error) => {
                                return protocol_error(sinks, &federate_id, error.to_string())
                                    .await;
                            }
                        };
                        stopped.insert(federate_id);
                        send_deliveries(sinks, deliveries).await?;
                        continue;
                    }

                    let deliveries = match self.rti.handle(message) {
                        Ok(deliveries) => deliveries,
                        Err(error) => {
                            return protocol_error(sinks, &federate_id, error.to_string()).await;
                        }
                    };
                    send_deliveries(sinks, deliveries).await?;
                }
                SessionInput::Closed { federate_id } => {
                    return Err(SessionError::Shutdown(format!(
                        "federate `{federate_id}` closed before Stop"
                    )));
                }
                SessionInput::TransportError { federate_id, error } => {
                    return Err(SessionError::Transport {
                        federate_id,
                        source: error,
                    });
                }
            }
        }

        for federate_id in expected {
            send_frame(
                sinks,
                federate_id,
                ProtocolFrame::RtiToFederate(RtiToFederate::Stop),
            )
            .await?;
        }

        Ok(())
    }

    fn validate_federate_message(
        &self,
        federate_id: &FederateId,
        message: &FederateToRti,
    ) -> Result<(), String> {
        match message {
            FederateToRti::Hello {
                federate_id: message_federate,
                ..
            }
            | FederateToRti::Net {
                federate_id: message_federate,
                ..
            }
            | FederateToRti::Ltc {
                federate_id: message_federate,
                ..
            }
            | FederateToRti::MsgAck {
                federate_id: message_federate,
                ..
            }
            | FederateToRti::Stop {
                federate_id: message_federate,
            } => {
                if message_federate != federate_id {
                    return Err(format!(
                        "message identified federate `{message_federate}`, but endpoint is `{federate_id}`"
                    ));
                }
            }
            FederateToRti::Msg {
                source,
                target,
                endpoint,
                ..
            } => {
                if source != federate_id {
                    return Err(format!(
                        "MSG source `{source}` does not match endpoint `{federate_id}`"
                    ));
                }
                let valid_route = self.rti.topology().edges.iter().any(|edge| {
                    &edge.source == source && &edge.target == target && &edge.endpoint == endpoint
                });
                if !valid_route {
                    return Err(format!(
                        "MSG route {source} -> {target} endpoint `{endpoint}` is not in the RTI topology"
                    ));
                }
            }
        }

        Ok(())
    }
}

fn expected_federates(topology: &FederatedTopology) -> BTreeSet<FederateId> {
    topology.federates.iter().cloned().collect()
}

fn spawn_stream_reader<R>(
    federate_id: FederateId,
    mut stream: R,
    input_tx: mpsc::UnboundedSender<SessionInput>,
) -> JoinHandle<()>
where
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError>,
{
    tokio::spawn(async move {
        loop {
            let input = match stream.try_next().await {
                Ok(Some(frame)) => SessionInput::Frame {
                    federate_id: federate_id.clone(),
                    frame,
                },
                Err(error) => SessionInput::TransportError {
                    federate_id: federate_id.clone(),
                    error: error.into(),
                },
                Ok(None) => SessionInput::Closed {
                    federate_id: federate_id.clone(),
                },
            };

            let should_exit = matches!(
                input,
                SessionInput::Closed { .. } | SessionInput::TransportError { .. }
            );
            if input_tx.send(input).is_err() || should_exit {
                break;
            }
        }
    })
}

async fn receive_session_input(
    input_rx: &mut mpsc::UnboundedReceiver<SessionInput>,
) -> Result<SessionInput, SessionError> {
    input_rx.recv().await.ok_or_else(|| {
        SessionError::Shutdown(
            "all federate transport streams closed before session completion".into(),
        )
    })
}

async fn send_deliveries<S>(
    sinks: &mut BTreeMap<FederateId, S>,
    deliveries: Vec<RtiDelivery>,
) -> Result<(), SessionError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError>,
{
    for delivery in deliveries {
        send_frame(
            sinks,
            &delivery.federate_id,
            ProtocolFrame::RtiToFederate(delivery.message),
        )
        .await?;
    }

    Ok(())
}

async fn send_frame<S>(
    sinks: &mut BTreeMap<FederateId, S>,
    federate_id: &FederateId,
    frame: ProtocolFrame,
) -> Result<(), SessionError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError>,
{
    let sink = sinks.get_mut(federate_id).ok_or_else(|| {
        SessionError::Shutdown(format!("no transport sink for federate `{federate_id}`"))
    })?;
    sink.send(frame)
        .await
        .map_err(|source| SessionError::Transport {
            federate_id: federate_id.clone(),
            source: source.into(),
        })
}

async fn protocol_error<S>(
    sinks: &mut BTreeMap<FederateId, S>,
    federate_id: &FederateId,
    message: impl Into<String>,
) -> Result<(), SessionError>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError>,
{
    let message = message.into();
    let _ = send_frame(
        sinks,
        federate_id,
        ProtocolFrame::RtiToFederate(RtiToFederate::Error {
            message: message.clone(),
        }),
    )
    .await;
    Err(SessionError::Protocol {
        federate_id: federate_id.clone(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use futures_util::{FutureExt, SinkExt, StreamExt};

    use super::*;
    use crate::test_trace::{Trace, TraceActor, TraceEvent, TraceMessage, TracePattern};
    use crate::{
        in_memory_transport_pair, transport::InMemoryTransport, EndpointId, TopologyEdge,
        WireDelay, WireTag,
    };

    type SessionFixture = (
        InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        JoinHandle<Result<(), SessionError>>,
    );

    fn fed(id: &str) -> FederateId {
        FederateId::new(id)
    }

    fn endpoint(id: &str) -> EndpointId {
        EndpointId::new(id)
    }

    fn source_sink_topology() -> FederatedTopology {
        let source = fed("source");
        let sink = fed("sink");
        FederatedTopology::with_edges(
            [source.clone(), sink.clone()],
            [TopologyEdge::new(
                source,
                sink,
                endpoint("source.out->sink.in"),
                WireDelay::ZERO,
            )],
        )
    }

    async fn send_client_frame(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        message: FederateToRti,
    ) {
        transport
            .0
            .send(ProtocolFrame::FederateToRti(message))
            .await
            .unwrap();
    }

    async fn send_client_frame_traced(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        message: FederateToRti,
        trace: &mut Trace,
    ) {
        trace.push(TraceEvent::client_to_rti(&message));
        send_client_frame(transport, message).await;
    }

    async fn recv_client_frame(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    ) -> RtiToFederate {
        match transport.1.next().await.unwrap().unwrap() {
            ProtocolFrame::RtiToFederate(message) => message,
            other => panic!("expected RTI-to-federate frame, got {other:?}"),
        }
    }

    async fn recv_client_frame_traced(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        target: &FederateId,
        trace: &mut Trace,
    ) -> RtiToFederate {
        let message = recv_client_frame(transport).await;
        trace.push(TraceEvent::rti_to_client(target, &message));
        message
    }

    fn recv_client_frame_now(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    ) -> Option<RtiToFederate> {
        match transport.1.next().now_or_never() {
            Some(Some(Ok(ProtocolFrame::RtiToFederate(message)))) => Some(message),
            Some(Some(Ok(other))) => panic!("expected RTI-to-federate frame, got {other:?}"),
            Some(Some(Err(error))) => panic!("transport error while polling client frame: {error}"),
            Some(None) => panic!("transport closed while polling client frame"),
            None => None,
        }
    }

    async fn expect_start(transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>) {
        assert_eq!(
            recv_client_frame(transport).await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );
    }

    async fn expect_start_traced(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        target: &FederateId,
        trace: &mut Trace,
    ) {
        assert_eq!(
            recv_client_frame_traced(transport, target, trace).await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );
    }

    async fn expect_tag(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        tag: WireTag,
    ) {
        assert_eq!(
            recv_client_frame(transport).await,
            RtiToFederate::Tag { tag }
        );
    }

    async fn expect_tag_traced(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
        target: &FederateId,
        tag: WireTag,
        trace: &mut Trace,
    ) {
        assert_eq!(
            recv_client_frame_traced(transport, target, trace).await,
            RtiToFederate::Tag { tag }
        );
    }

    fn session_endpoint(
        transport: InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    ) -> RtiSessionEndpoint<
        crate::transport::InMemoryFrameSink<ProtocolFrame>,
        crate::transport::InMemoryFrameStream<ProtocolFrame>,
    > {
        let (sink, stream) = transport;
        RtiSessionEndpoint::new(sink, stream)
    }

    fn spawn_session(topology: FederatedTopology) -> SessionFixture {
        let (source_client, source_rti) = in_memory_transport_pair();
        let (sink_client, sink_rti) = in_memory_transport_pair();
        let mut endpoints = BTreeMap::new();
        endpoints.insert(fed("source"), session_endpoint(source_rti));
        endpoints.insert(fed("sink"), session_endpoint(sink_rti));

        let session = StaticRtiSession::new(topology, endpoints);
        let handle = tokio::spawn(session.run());

        (source_client, sink_client, handle)
    }

    #[tokio::test]
    async fn session_routes_zero_tag_msg_and_unblocks_later_grants() {
        let topology = source_sink_topology();
        let (mut source_client, mut sink_client, session) = spawn_session(topology.clone());
        let source = fed("source");
        let sink = fed("sink");
        let endpoint = endpoint("source.out->sink.in");
        let mut trace = Trace::default();

        send_client_frame_traced(
            &mut source_client,
            FederateToRti::Hello {
                federate_id: source.clone(),
                topology: topology.neighbors_for(&source),
            },
            &mut trace,
        )
        .await;
        send_client_frame_traced(
            &mut sink_client,
            FederateToRti::Hello {
                federate_id: sink.clone(),
                topology: topology.neighbors_for(&sink),
            },
            &mut trace,
        )
        .await;
        expect_start_traced(&mut source_client, &source, &mut trace).await;
        expect_start_traced(&mut sink_client, &sink, &mut trace).await;

        send_client_frame_traced(
            &mut sink_client,
            FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            },
            &mut trace,
        )
        .await;
        send_client_frame_traced(
            &mut source_client,
            FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            },
            &mut trace,
        )
        .await;
        expect_tag_traced(&mut source_client, &source, WireTag::ZERO, &mut trace).await;

        send_client_frame_traced(
            &mut source_client,
            FederateToRti::Msg {
                source: source.clone(),
                target: sink.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello".to_vec(),
            },
            &mut trace,
        )
        .await;
        assert_eq!(
            recv_client_frame_traced(&mut sink_client, &sink, &mut trace).await,
            RtiToFederate::Msg {
                source: source.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello".to_vec(),
            }
        );

        send_client_frame_traced(
            &mut source_client,
            FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 1),
            },
            &mut trace,
        )
        .await;
        expect_tag_traced(
            &mut source_client,
            &source,
            WireTag::finite(0, 1),
            &mut trace,
        )
        .await;
        send_client_frame_traced(
            &mut sink_client,
            FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            },
            &mut trace,
        )
        .await;
        expect_tag_traced(&mut sink_client, &sink, WireTag::ZERO, &mut trace).await;
        send_client_frame_traced(
            &mut sink_client,
            FederateToRti::Ltc {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            },
            &mut trace,
        )
        .await;

        send_client_frame_traced(
            &mut source_client,
            FederateToRti::Stop {
                federate_id: source.clone(),
            },
            &mut trace,
        )
        .await;
        send_client_frame_traced(
            &mut sink_client,
            FederateToRti::Stop {
                federate_id: sink.clone(),
            },
            &mut trace,
        )
        .await;
        assert_eq!(
            recv_client_frame_traced(&mut source_client, &source, &mut trace).await,
            RtiToFederate::Stop
        );
        assert_eq!(
            recv_client_frame_traced(&mut sink_client, &sink, &mut trace).await,
            RtiToFederate::Stop
        );

        use TraceActor::{Client, Rti};
        use TraceMessage::{Hello, Ltc, Msg, MsgAck, Net, Start, Stop, Tag};

        trace.assert_exact(&[
            TraceEvent::new(Client(source.clone()), Rti, Hello),
            TraceEvent::new(Client(sink.clone()), Rti, Hello),
            TraceEvent::new(Rti, Client(source.clone()), Start),
            TraceEvent::new(Rti, Client(sink.clone()), Start),
            TraceEvent::new(Client(sink.clone()), Rti, Net(WireTag::ZERO)),
            TraceEvent::new(Client(source.clone()), Rti, Net(WireTag::ZERO)),
            TraceEvent::new(Rti, Client(source.clone()), Tag(WireTag::ZERO)),
            TraceEvent::new(
                Client(source.clone()),
                Rti,
                Msg {
                    tag: WireTag::ZERO,
                    endpoint: endpoint.clone(),
                },
            ),
            TraceEvent::new(
                Rti,
                Client(sink.clone()),
                Msg {
                    tag: WireTag::ZERO,
                    endpoint: endpoint.clone(),
                },
            ),
            TraceEvent::new(Client(source.clone()), Rti, Net(WireTag::finite(0, 1))),
            TraceEvent::new(Rti, Client(source.clone()), Tag(WireTag::finite(0, 1))),
            TraceEvent::new(Client(sink.clone()), Rti, MsgAck(WireTag::ZERO)),
            TraceEvent::new(Rti, Client(sink.clone()), Tag(WireTag::ZERO)),
            TraceEvent::new(Client(sink.clone()), Rti, Ltc(WireTag::ZERO)),
            TraceEvent::new(Client(source.clone()), Rti, Stop),
            TraceEvent::new(Client(sink.clone()), Rti, Stop),
            TraceEvent::new(Rti, Client(source.clone()), Stop),
            TraceEvent::new(Rti, Client(sink.clone()), Stop),
        ]);

        let source_tag = TracePattern::between(Rti, Client(source.clone()), Tag(WireTag::ZERO));
        let source_msg = TracePattern::between(
            Client(source.clone()),
            Rti,
            Msg {
                tag: WireTag::ZERO,
                endpoint: endpoint.clone(),
            },
        );
        let forwarded_msg = TracePattern::between(
            Rti,
            Client(sink.clone()),
            Msg {
                tag: WireTag::ZERO,
                endpoint: endpoint.clone(),
            },
        );
        let target_ack = TracePattern::between(Client(sink.clone()), Rti, MsgAck(WireTag::ZERO));
        let target_tag = TracePattern::between(Rti, Client(sink.clone()), Tag(WireTag::ZERO));

        trace.assert_before(source_tag, source_msg);
        trace.assert_before(forwarded_msg, target_ack.clone());
        trace.assert_before(target_ack, target_tag);
        trace.assert_before(
            TracePattern::between(Client(source.clone()), Rti, Stop),
            TracePattern::between(Rti, Client(source.clone()), Stop),
        );
        trace.assert_before(
            TracePattern::between(Client(sink.clone()), Rti, Stop),
            TracePattern::between(Rti, Client(sink.clone()), Stop),
        );

        drop(source_client);
        drop(sink_client);
        session.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_blocks_pending_grant_behind_multiple_same_tag_messages() {
        let topology = source_sink_topology();
        let (mut source_client, mut sink_client, session) = spawn_session(topology.clone());
        let source = fed("source");
        let sink = fed("sink");
        let endpoint = endpoint("source.out->sink.in");

        send_client_frame(
            &mut source_client,
            FederateToRti::Hello {
                federate_id: source.clone(),
                topology: topology.neighbors_for(&source),
            },
        )
        .await;
        send_client_frame(
            &mut sink_client,
            FederateToRti::Hello {
                federate_id: sink.clone(),
                topology: topology.neighbors_for(&sink),
            },
        )
        .await;
        expect_start(&mut source_client).await;
        expect_start(&mut sink_client).await;

        send_client_frame(
            &mut sink_client,
            FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::finite(0, 1),
            },
        )
        .await;
        send_client_frame(
            &mut source_client,
            FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            },
        )
        .await;
        expect_tag(&mut source_client, WireTag::ZERO).await;

        for payload in [b"first".to_vec(), b"second".to_vec()] {
            send_client_frame(
                &mut source_client,
                FederateToRti::Msg {
                    source: source.clone(),
                    target: sink.clone(),
                    endpoint: endpoint.clone(),
                    tag: WireTag::ZERO,
                    payload: payload.clone(),
                },
            )
            .await;
            assert_eq!(
                recv_client_frame(&mut sink_client).await,
                RtiToFederate::Msg {
                    source: source.clone(),
                    endpoint: endpoint.clone(),
                    tag: WireTag::ZERO,
                    payload,
                }
            );
        }

        send_client_frame(
            &mut source_client,
            FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 2),
            },
        )
        .await;
        expect_tag(&mut source_client, WireTag::finite(0, 2)).await;
        tokio::task::yield_now().await;
        assert_eq!(recv_client_frame_now(&mut sink_client), None);

        send_client_frame(
            &mut sink_client,
            FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            },
        )
        .await;
        tokio::task::yield_now().await;
        assert_eq!(recv_client_frame_now(&mut sink_client), None);
        send_client_frame(
            &mut sink_client,
            FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            },
        )
        .await;
        expect_tag(&mut sink_client, WireTag::finite(0, 1)).await;

        send_client_frame(
            &mut source_client,
            FederateToRti::Stop {
                federate_id: source,
            },
        )
        .await;
        send_client_frame(&mut sink_client, FederateToRti::Stop { federate_id: sink }).await;
        assert_eq!(
            recv_client_frame(&mut source_client).await,
            RtiToFederate::Stop
        );
        assert_eq!(
            recv_client_frame(&mut sink_client).await,
            RtiToFederate::Stop
        );

        drop(source_client);
        drop(sink_client);
        session.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_sends_protocol_error_for_unexpected_federate_frame() {
        let topology = source_sink_topology();
        let (mut source_client, mut sink_client, session) = spawn_session(topology.clone());
        let source = fed("source");
        let sink = fed("sink");

        send_client_frame(
            &mut source_client,
            FederateToRti::Hello {
                federate_id: source.clone(),
                topology: topology.neighbors_for(&source),
            },
        )
        .await;
        send_client_frame(
            &mut sink_client,
            FederateToRti::Hello {
                federate_id: sink.clone(),
                topology: topology.neighbors_for(&sink),
            },
        )
        .await;
        expect_start(&mut source_client).await;
        expect_start(&mut sink_client).await;

        source_client
            .0
            .send(ProtocolFrame::RtiToFederate(RtiToFederate::Stop))
            .await
            .unwrap();
        assert!(matches!(
            recv_client_frame(&mut source_client).await,
            RtiToFederate::Error { message }
                if message.contains("RTI-to-federate frame")
        ));

        drop(source_client);
        drop(sink_client);
        let error = session.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            SessionError::Protocol {
                federate_id,
                message
            } if federate_id == source && message.contains("RTI-to-federate frame")
        ));
    }
}
