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
    ) -> Result<Self, SessionError> {
        Ok(Self {
            rti: RtiState::new(topology)?,
            endpoints,
            start_unix_epoch_ns: 0,
        })
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
                let valid_route = self.rti.contains_route(source, target, endpoint);
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
    use futures_util::{SinkExt, StreamExt};

    use super::*;
    use crate::test_trace::{FramePattern, RecordingClientTransport, Trace, TracePattern};
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

    fn positive_delay_cycle_topology() -> FederatedTopology {
        let a = fed("a");
        let b = fed("b");
        let delay = WireDelay::from_nanos(10);
        FederatedTopology::with_edges(
            [a.clone(), b.clone()],
            [
                TopologyEdge::new(a.clone(), b.clone(), endpoint("a.out->b.in"), delay),
                TopologyEdge::new(b, a, endpoint("b.out->a.in"), delay),
            ],
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

    async fn recv_client_frame(
        transport: &mut InMemoryTransport<ProtocolFrame, ProtocolFrame>,
    ) -> RtiToFederate {
        match transport.1.next().await.unwrap().unwrap() {
            ProtocolFrame::RtiToFederate(message) => message,
            other => panic!("expected RTI-to-federate frame, got {other:?}"),
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
        spawn_two_federate_session(topology, fed("source"), fed("sink"))
    }

    fn spawn_two_federate_session(
        topology: FederatedTopology,
        first: FederateId,
        second: FederateId,
    ) -> SessionFixture {
        let (source_client, source_rti) = in_memory_transport_pair();
        let (sink_client, sink_rti) = in_memory_transport_pair();
        let mut endpoints = BTreeMap::new();
        endpoints.insert(first, session_endpoint(source_rti));
        endpoints.insert(second, session_endpoint(sink_rti));

        let session = StaticRtiSession::new(topology, endpoints).expect("valid test topology");
        let handle = tokio::spawn(session.run());

        (source_client, sink_client, handle)
    }

    #[tokio::test]
    async fn session_positive_delay_cycle_progresses_with_bounded_control_trace() {
        const ADVANCE_COUNT: usize = 3;
        const FEDERATE_COUNT: usize = 2;
        const CONTROL_EVENTS_PER_ADVANCE: usize = 3; // NET request, TAG grant, and LTC.
        const FIXED_PROTOCOL_EVENTS: usize = 8; // Two each of Hello, Start, and both Stop directions.

        let topology = positive_delay_cycle_topology();
        let a = fed("a");
        let b = fed("b");
        let (a_client, b_client, session) =
            spawn_two_federate_session(topology.clone(), a.clone(), b.clone());
        let trace = Trace::default();
        let mut a_client = RecordingClientTransport::new(a_client, a.clone(), trace.clone());
        let mut b_client = RecordingClientTransport::new(b_client, b.clone(), trace.clone());

        a_client
            .send(FederateToRti::Hello {
                federate_id: a.clone(),
                topology: topology.neighbors_for(&a),
            })
            .await;
        b_client
            .send(FederateToRti::Hello {
                federate_id: b.clone(),
                topology: topology.neighbors_for(&b),
            })
            .await;
        assert_eq!(
            a_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );
        assert_eq!(
            b_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );

        let requested_tags = [
            WireTag::ZERO,
            WireTag::finite(10, 0),
            WireTag::finite(20, 0),
        ];
        for tag in requested_tags {
            a_client
                .send(FederateToRti::Net {
                    federate_id: a.clone(),
                    tag,
                })
                .await;
            b_client
                .send(FederateToRti::Net {
                    federate_id: b.clone(),
                    tag,
                })
                .await;
            assert_eq!(a_client.recv().await, RtiToFederate::Tag { tag });
            assert_eq!(b_client.recv().await, RtiToFederate::Tag { tag });
            a_client
                .send(FederateToRti::Ltc {
                    federate_id: a.clone(),
                    tag,
                })
                .await;
            b_client
                .send(FederateToRti::Ltc {
                    federate_id: b.clone(),
                    tag,
                })
                .await;
        }

        a_client
            .send(FederateToRti::Stop {
                federate_id: a.clone(),
            })
            .await;
        b_client
            .send(FederateToRti::Stop {
                federate_id: b.clone(),
            })
            .await;
        assert_eq!(a_client.recv().await, RtiToFederate::Stop);
        assert_eq!(b_client.recv().await, RtiToFederate::Stop);

        use FramePattern::{Ltc, Net, Tag};
        for tag in requested_tags {
            for federate in [&a, &b] {
                trace.assert_count(TracePattern::client_to_rti(federate.clone(), Net(tag)), 1);
                trace.assert_count(TracePattern::rti_to_client(federate.clone(), Tag(tag)), 1);
                trace.assert_count(TracePattern::client_to_rti(federate.clone(), Ltc(tag)), 1);
                trace.assert_before(
                    TracePattern::client_to_rti(federate.clone(), Net(tag)),
                    TracePattern::rti_to_client(federate.clone(), Tag(tag)),
                );
                trace.assert_before(
                    TracePattern::rti_to_client(federate.clone(), Tag(tag)),
                    TracePattern::client_to_rti(federate.clone(), Ltc(tag)),
                );
            }
        }
        trace.assert_len(
            ADVANCE_COUNT * FEDERATE_COUNT * CONTROL_EVENTS_PER_ADVANCE + FIXED_PROTOCOL_EVENTS,
        );

        drop(a_client);
        drop(b_client);
        session.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_routes_zero_tag_msg_and_unblocks_later_grants() {
        let topology = source_sink_topology();
        let (source_client, sink_client, session) = spawn_session(topology.clone());
        let source = fed("source");
        let sink = fed("sink");
        let endpoint = endpoint("source.out->sink.in");
        let trace = Trace::default();
        let mut source_client =
            RecordingClientTransport::new(source_client, source.clone(), trace.clone());
        let mut sink_client =
            RecordingClientTransport::new(sink_client, sink.clone(), trace.clone());

        source_client
            .send(FederateToRti::Hello {
                federate_id: source.clone(),
                topology: topology.neighbors_for(&source),
            })
            .await;
        sink_client
            .send(FederateToRti::Hello {
                federate_id: sink.clone(),
                topology: topology.neighbors_for(&sink),
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );
        assert_eq!(
            sink_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );

        sink_client
            .send(FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );

        source_client
            .send(FederateToRti::Msg {
                source: source.clone(),
                target: sink.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello".to_vec(),
            })
            .await;
        assert_eq!(
            sink_client.recv().await,
            RtiToFederate::Msg {
                source: source.clone(),
                endpoint: endpoint.clone(),
                tag: WireTag::ZERO,
                payload: b"hello".to_vec(),
            }
        );

        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 1),
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Tag {
                tag: WireTag::finite(0, 1),
            }
        );
        sink_client
            .send(FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        assert_eq!(
            sink_client.recv().await,
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );
        sink_client
            .send(FederateToRti::Ltc {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .await;

        source_client
            .send(FederateToRti::Stop {
                federate_id: source.clone(),
            })
            .await;
        sink_client
            .send(FederateToRti::Stop {
                federate_id: sink.clone(),
            })
            .await;
        assert_eq!(source_client.recv().await, RtiToFederate::Stop);
        assert_eq!(sink_client.recv().await, RtiToFederate::Stop);

        use FramePattern::{Hello, Ltc, Msg, MsgAck, Net, Start, Stop, Tag};

        trace.assert_exact(&[
            TracePattern::client_to_rti(source.clone(), Hello),
            TracePattern::client_to_rti(sink.clone(), Hello),
            TracePattern::rti_to_client(source.clone(), Start),
            TracePattern::rti_to_client(sink.clone(), Start),
            TracePattern::client_to_rti(sink.clone(), Net(WireTag::ZERO)),
            TracePattern::client_to_rti(source.clone(), Net(WireTag::ZERO)),
            TracePattern::rti_to_client(source.clone(), Tag(WireTag::ZERO)),
            TracePattern::client_to_rti(
                source.clone(),
                Msg {
                    tag: WireTag::ZERO,
                    endpoint: endpoint.clone(),
                },
            ),
            TracePattern::rti_to_client(
                sink.clone(),
                Msg {
                    tag: WireTag::ZERO,
                    endpoint: endpoint.clone(),
                },
            ),
            TracePattern::client_to_rti(source.clone(), Net(WireTag::finite(0, 1))),
            TracePattern::rti_to_client(source.clone(), Tag(WireTag::finite(0, 1))),
            TracePattern::client_to_rti(sink.clone(), MsgAck(WireTag::ZERO)),
            TracePattern::rti_to_client(sink.clone(), Tag(WireTag::ZERO)),
            TracePattern::client_to_rti(sink.clone(), Ltc(WireTag::ZERO)),
            TracePattern::client_to_rti(source.clone(), Stop),
            TracePattern::client_to_rti(sink.clone(), Stop),
            TracePattern::rti_to_client(source.clone(), Stop),
            TracePattern::rti_to_client(sink.clone(), Stop),
        ]);

        let source_tag = TracePattern::rti_to_client(source.clone(), Tag(WireTag::ZERO));
        let source_msg = TracePattern::client_to_rti(
            source.clone(),
            Msg {
                tag: WireTag::ZERO,
                endpoint: endpoint.clone(),
            },
        );
        let forwarded_msg = TracePattern::rti_to_client(
            sink.clone(),
            Msg {
                tag: WireTag::ZERO,
                endpoint: endpoint.clone(),
            },
        );
        let target_ack = TracePattern::client_to_rti(sink.clone(), MsgAck(WireTag::ZERO));
        let target_tag = TracePattern::rti_to_client(sink.clone(), Tag(WireTag::ZERO));

        trace.assert_before(source_tag, source_msg);
        trace.assert_before(forwarded_msg, target_ack.clone());
        trace.assert_before(target_ack, target_tag);
        trace.assert_before(
            TracePattern::client_to_rti(source.clone(), Stop),
            TracePattern::rti_to_client(source.clone(), Stop),
        );
        trace.assert_before(
            TracePattern::client_to_rti(sink.clone(), Stop),
            TracePattern::rti_to_client(sink.clone(), Stop),
        );

        drop(source_client);
        drop(sink_client);
        session.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn session_blocks_pending_grant_behind_multiple_same_tag_messages() {
        let topology = source_sink_topology();
        let (source_client, sink_client, session) = spawn_session(topology.clone());
        let source = fed("source");
        let sink = fed("sink");
        let endpoint = endpoint("source.out->sink.in");
        let trace = Trace::default();
        let mut source_client =
            RecordingClientTransport::new(source_client, source.clone(), trace.clone());
        let mut sink_client =
            RecordingClientTransport::new(sink_client, sink.clone(), trace.clone());

        source_client
            .send(FederateToRti::Hello {
                federate_id: source.clone(),
                topology: topology.neighbors_for(&source),
            })
            .await;
        sink_client
            .send(FederateToRti::Hello {
                federate_id: sink.clone(),
                topology: topology.neighbors_for(&sink),
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );
        assert_eq!(
            sink_client.recv().await,
            RtiToFederate::Start {
                start_unix_epoch_ns: 0,
            }
        );

        sink_client
            .send(FederateToRti::Net {
                federate_id: sink.clone(),
                tag: WireTag::finite(0, 1),
            })
            .await;
        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Tag { tag: WireTag::ZERO }
        );

        for payload in [b"first".to_vec(), b"second".to_vec()] {
            source_client
                .send(FederateToRti::Msg {
                    source: source.clone(),
                    target: sink.clone(),
                    endpoint: endpoint.clone(),
                    tag: WireTag::ZERO,
                    payload: payload.clone(),
                })
                .await;
            assert_eq!(
                sink_client.recv().await,
                RtiToFederate::Msg {
                    source: source.clone(),
                    endpoint: endpoint.clone(),
                    tag: WireTag::ZERO,
                    payload,
                }
            );
        }

        source_client
            .send(FederateToRti::Net {
                federate_id: source.clone(),
                tag: WireTag::finite(0, 2),
            })
            .await;
        assert_eq!(
            source_client.recv().await,
            RtiToFederate::Tag {
                tag: WireTag::finite(0, 2),
            }
        );
        tokio::task::yield_now().await;
        assert_eq!(sink_client.recv_now(), None);

        let target_ack =
            TracePattern::client_to_rti(sink.clone(), FramePattern::MsgAck(WireTag::ZERO));
        let target_grant =
            TracePattern::rti_to_client(sink.clone(), FramePattern::Tag(WireTag::finite(0, 1)));

        sink_client
            .send(FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        tokio::task::yield_now().await;
        assert_eq!(sink_client.recv_now(), None);
        trace.assert_count(target_ack.clone(), 1);
        trace.assert_absent(target_grant.clone());

        sink_client
            .send(FederateToRti::MsgAck {
                federate_id: sink.clone(),
                tag: WireTag::ZERO,
            })
            .await;
        assert_eq!(
            sink_client.recv().await,
            RtiToFederate::Tag {
                tag: WireTag::finite(0, 1),
            }
        );

        source_client
            .send(FederateToRti::Stop {
                federate_id: source.clone(),
            })
            .await;
        sink_client
            .send(FederateToRti::Stop {
                federate_id: sink.clone(),
            })
            .await;
        assert_eq!(source_client.recv().await, RtiToFederate::Stop);
        assert_eq!(sink_client.recv().await, RtiToFederate::Stop);

        let source_message = TracePattern::client_to_rti(
            source,
            FramePattern::Msg {
                tag: WireTag::ZERO,
                endpoint: endpoint.clone(),
            },
        );
        let forwarded_message = TracePattern::rti_to_client(
            sink,
            FramePattern::Msg {
                tag: WireTag::ZERO,
                endpoint,
            },
        );

        trace.assert_count(source_message.clone(), 2);
        trace.assert_count(forwarded_message.clone(), 2);
        trace.assert_count(target_ack.clone(), 2);
        trace.assert_count(target_grant.clone(), 1);
        trace.assert_before(source_message, forwarded_message.clone());
        trace.assert_before(forwarded_message, target_ack.clone());
        trace.assert_before(target_ack, target_grant);

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
