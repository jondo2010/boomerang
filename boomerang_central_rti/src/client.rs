//! Federate-side protocol bridge for one persistent federate.
use std::collections::BTreeMap;
use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration as StdDuration,
};

use crate::RuntimeBridgeError;
use crate::WireTag;
use crate::{
    FederateId, FederateToRti, NeighborStructure, ProtocolFrame, RtiToFederate, TransportError,
};
use futures_util::{Sink, SinkExt, TryStream, TryStreamExt};
use tokio::task::JoinHandle;

#[derive(Debug, thiserror::Error)]
/// Failure while connecting a federate or driving its RTI coordination.
pub enum FederateClientError {
    /// The transport could not carry a protocol frame.
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    /// A runtime value could not be represented by the wire protocol.
    #[error("runtime bridge error: {0}")]
    RuntimeBridge(#[from] RuntimeBridgeError),
    /// The runtime rejected a federated endpoint operation.
    #[error("runtime endpoint error: {0}")]
    RuntimeEndpoint(#[from] boomerang_runtime::FederatedEndpointError),

    /// A peer violated the protocol sequencing or topology contract.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The RTI explicitly rejected a federate operation.
    #[error("RTI error: {message}")]
    RtiError {
        /// Error text supplied by the RTI.
        message: String,
    },

    /// The RTI ended this federate's session.
    #[error("RTI stopped the federate session")]
    RtiStopped,
    /// An earlier failure made later barrier progress unsafe.
    #[error(
        "federated scheduler barrier is terminal after an earlier protocol or admission failure"
    )]
    BarrierFailed,

    /// The protocol client's background tasks are no longer connected.
    #[error("federate protocol client is closed")]
    ClientClosed,
    /// Scheduling an inbound payload lost the scheduler event receiver.
    #[error("scheduler event channel closed after scheduling inbound endpoint `{endpoint}`")]
    SchedulerEventChannelClosed {
        /// Stable endpoint whose scheduling event could not be observed.
        endpoint: crate::EndpointId,
    },
    /// Two lowered routes use the same stable endpoint identity.
    #[error("duplicate federated client route for endpoint `{0}`")]
    DuplicateRoute(crate::EndpointId),
    /// No lowered route owns the referenced stable endpoint identity.
    #[error("unknown federated client route for endpoint `{0}`")]
    UnknownRoute(crate::EndpointId),
    /// A received payload has no runtime endpoint binding.
    #[error("federated client route for endpoint `{0}` has no inbound runtime binding")]
    UnboundInboundRoute(crate::EndpointId),
    /// Lowering attempted to bind an inbound endpoint more than once.
    #[error("federated client route for endpoint `{0}` already has an inbound runtime binding")]
    DuplicateInboundBinding(crate::EndpointId),
    /// A route is used by a federate other than its declared source.
    #[error(
        "route for endpoint `{endpoint}` has source `{route_source}`, expected `{federate_id}`"
    )]
    RouteSourceMismatch {
        /// Stable endpoint carried by the mismatched route.
        endpoint: crate::EndpointId,
        /// Source federate recorded in lowered route metadata.
        route_source: FederateId,
        /// Federate attempting to use the route.
        federate_id: FederateId,
    },
    /// A route is used by a federate other than its declared target.
    #[error(
        "route for endpoint `{endpoint}` has target `{route_target}`, expected `{federate_id}`"
    )]
    RouteTargetMismatch {
        /// Stable endpoint carried by the mismatched route.
        endpoint: crate::EndpointId,
        /// Target federate recorded in lowered route metadata.
        route_target: FederateId,
        /// Federate receiving the route payload.
        federate_id: FederateId,
    },
    /// An inbound payload declares a source different from its route metadata.
    #[error(
        "inbound MSG for endpoint `{endpoint}` came from `{observed_source}`, but route source is `{route_source}`"
    )]
    InboundSourceMismatch {
        /// Stable endpoint carried by the payload.
        endpoint: crate::EndpointId,
        /// Source federate declared in the payload frame.
        observed_source: FederateId,
        /// Source federate recorded in lowered route metadata.
        route_source: FederateId,
    },
    /// An RTI grant did not satisfy the outstanding request.
    #[error("received TAG {received} while waiting for {requested}")]
    UnexpectedTag {
        /// Wire tag requested from the RTI.
        requested: WireTag,
        /// Wire tag received from the RTI.
        received: WireTag,
    },
}

enum ClientInput {
    /// A protocol message sent by the RTI.
    Message(RtiToFederate),
    /// A transport failure from a background task.
    Transport(TransportError),
    /// A frame with an invalid direction or phase.
    Protocol(String),
    /// The RTI-side stream ended cleanly.
    Closed,
}

/// Cloneable sender for a federate's single ordered protocol-outbound queue.
#[derive(Debug, Clone)]
pub struct FederateProtocolSender {
    /// Queue consumed by the protocol writer task.
    outgoing: tokio::sync::mpsc::UnboundedSender<FederateToRti>,
}

impl FederateProtocolSender {
    /// Queue a frame while preserving this federate's outbound ordering.
    pub fn send(&self, message: FederateToRti) -> Result<(), FederateClientError> {
        self.outgoing
            .send(message)
            .map_err(|_| FederateClientError::ClientClosed)
    }
}

/// A prebuildable protocol mailbox whose receiver is connected to a transport at execution time.
#[derive(Debug)]
pub struct FederateClientMailbox {
    /// Cloneable handle for producers created during lowering.
    sender: FederateProtocolSender,
    /// Unconnected frame queue later consumed by a protocol writer.
    receiver: tokio::sync::mpsc::UnboundedReceiver<FederateToRti>,
}

impl FederateClientMailbox {
    /// Create an unconnected, ordered federate-to-RTI mailbox.
    pub fn new() -> Self {
        let (outgoing, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            sender: FederateProtocolSender { outgoing },
            receiver,
        }
    }

    /// Clone the producer handle for this mailbox.
    pub fn sender(&self) -> FederateProtocolSender {
        self.sender.clone()
    }

    /// Receive one queued frame without waiting for a producer.
    pub fn try_recv(&mut self) -> Result<Option<FederateToRti>, FederateClientError> {
        match self.receiver.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                Err(FederateClientError::ClientClosed)
            }
        }
    }

    fn into_parts(
        self,
    ) -> (
        FederateProtocolSender,
        tokio::sync::mpsc::UnboundedReceiver<FederateToRti>,
    ) {
        (self.sender, self.receiver)
    }
}

impl Default for FederateClientMailbox {
    fn default() -> Self {
        Self::new()
    }
}

/// A connected protocol client for one persistent federate.
#[derive(Debug)]
pub struct FederateProtocolClient {
    /// Ordered producer for federate-to-RTI frames.
    outgoing: FederateProtocolSender,
    /// Messages and failures forwarded by background transport tasks.
    incoming: mpsc::Receiver<ClientInput>,
    /// Common physical start epoch announced by the RTI.
    start_unix_epoch_ns: i128,
    /// Task reading RTI-to-federate frames.
    reader: JoinHandle<()>,
    /// Task writing queued federate-to-RTI frames.
    writer: JoinHandle<()>,
}

impl FederateProtocolClient {
    /// Connect a federate transport to the RTI and complete the Hello/Start handshake.
    /// Background reader and writer tasks are spawned for the live session.
    #[tracing::instrument(
        level = "debug",
        skip(federate_id, topology, sink, stream),
        fields(federate = %federate_id)
    )]
    pub async fn connect<S, R>(
        federate_id: FederateId,
        topology: NeighborStructure,
        sink: S,
        stream: R,
    ) -> Result<Self, FederateClientError>
    where
        S: Sink<ProtocolFrame> + Send + Unpin + 'static,
        S::Error: Into<TransportError> + Send + 'static,
        R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
        R::Error: Into<TransportError> + Send + 'static,
    {
        Self::connect_with_mailbox(
            federate_id,
            topology,
            sink,
            stream,
            FederateClientMailbox::new(),
        )
        .await
    }

    /// Connect a transport using an outbound mailbox created during runtime lowering.
    pub async fn connect_with_mailbox<S, R>(
        federate_id: FederateId,
        topology: NeighborStructure,
        mut sink: S,
        mut stream: R,
        mailbox: FederateClientMailbox,
    ) -> Result<Self, FederateClientError>
    where
        S: Sink<ProtocolFrame> + Send + Unpin + 'static,
        S::Error: Into<TransportError> + Send + 'static,
        R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
        R::Error: Into<TransportError> + Send + 'static,
    {
        sink.send(ProtocolFrame::FederateToRti(FederateToRti::Hello {
            federate_id,
            topology,
        }))
        .await
        .map_err(|error| FederateClientError::Transport(error.into()))?;

        let start_unix_epoch_ns = match stream
            .try_next()
            .await
            .map_err(|error| FederateClientError::Transport(error.into()))?
        {
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Start {
                start_unix_epoch_ns,
            })) => start_unix_epoch_ns,
            Some(ProtocolFrame::RtiToFederate(RtiToFederate::Error { message })) => {
                return Err(FederateClientError::RtiError { message });
            }
            Some(frame) => {
                return Err(FederateClientError::Protocol(format!(
                    "expected Start after Hello, got {frame:?}"
                )));
            }
            None => return Err(FederateClientError::Transport(TransportError::Closed)),
        };

        let (outgoing, outgoing_rx) = mailbox.into_parts();
        let (incoming, incoming_rx) = mpsc::channel();
        let reader = spawn_reader(stream, incoming.clone());
        let writer = spawn_writer(sink, outgoing_rx, incoming);

        Ok(Self {
            outgoing,
            incoming: incoming_rx,
            start_unix_epoch_ns,
            reader,
            writer,
        })
    }

    /// Return the RTI-provided physical start epoch from the Start frame.
    pub fn start_unix_epoch_ns(&self) -> i128 {
        self.start_unix_epoch_ns
    }

    /// Send one federate-to-RTI protocol message on the connected transport.
    pub fn send(&self, message: FederateToRti) -> Result<(), FederateClientError> {
        self.outgoing.send(message)
    }

    /// Receive one RTI-to-federate protocol message, waiting up to `timeout`.
    pub fn recv_timeout(
        &self,
        timeout: StdDuration,
    ) -> Result<Option<RtiToFederate>, FederateClientError> {
        match self.incoming.recv_timeout(timeout) {
            Ok(ClientInput::Message(message)) => Ok(Some(message)),
            Ok(ClientInput::Transport(error)) => Err(FederateClientError::Transport(error)),
            Ok(ClientInput::Protocol(message)) => Err(FederateClientError::Protocol(message)),
            Ok(ClientInput::Closed) => Err(FederateClientError::Transport(TransportError::Closed)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(FederateClientError::ClientClosed),
        }
    }
}

impl Drop for FederateProtocolClient {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

/// Spawn the task that converts RTI-to-federate frames into local client inputs.
fn spawn_reader<R>(mut stream: R, incoming: mpsc::Sender<ClientInput>) -> JoinHandle<()>
where
    R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
    R::Error: Into<TransportError> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            let input = match stream.try_next().await {
                Ok(Some(ProtocolFrame::RtiToFederate(message))) => ClientInput::Message(message),
                Ok(Some(frame)) => {
                    ClientInput::Protocol(format!("RTI sent unexpected frame {frame:?}"))
                }
                Ok(None) => ClientInput::Closed,
                Err(error) => ClientInput::Transport(error.into()),
            };
            let should_exit = matches!(
                input,
                ClientInput::Closed | ClientInput::Transport(_) | ClientInput::Protocol(_)
            );
            if incoming.send(input).is_err() || should_exit {
                break;
            }
        }
    })
}

/// Spawn the task that serializes this federate's queued outbound frames.
fn spawn_writer<S>(
    mut sink: S,
    mut outgoing: tokio::sync::mpsc::UnboundedReceiver<FederateToRti>,
    incoming: mpsc::Sender<ClientInput>,
) -> JoinHandle<()>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError> + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            if let Err(error) = sink.send(ProtocolFrame::FederateToRti(message)).await {
                let _ = incoming.send(ClientInput::Transport(error.into()));
                break;
            }
        }
    })
}
/// Lowered routing metadata for one cross-federate runtime endpoint.
#[derive(Debug)]
pub struct FederateClientRoute {
    /// Stable protocol endpoint identity for this route.
    pub endpoint: crate::EndpointId,
    /// Federate that emits payloads through this endpoint.
    pub source: FederateId,
    /// Federate that receives payloads through this endpoint.
    pub target: FederateId,
    /// Runtime delivery endpoint attached while lowering the target enclave.
    inbound: Option<boomerang_runtime::FederatedInboundEndpoint>,
}
impl FederateClientRoute {
    /// Create route metadata for one runtime federated endpoint.
    pub fn new(
        endpoint: impl Into<crate::EndpointId>,
        source: impl Into<FederateId>,
        target: impl Into<FederateId>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            source: source.into(),
            target: target.into(),
            inbound: None,
        }
    }

    pub(crate) fn bind_inbound(&mut self, inbound: boomerang_runtime::FederatedInboundEndpoint) {
        debug_assert!(self.inbound.is_none());
        self.inbound = Some(inbound);
    }

    pub(crate) fn inbound(&self) -> Option<&boomerang_runtime::FederatedInboundEndpoint> {
        self.inbound.as_ref()
    }
}

/// Federated scheduler barrier for one federate runtime enclave.
#[derive(Debug)]
pub struct RtiFederatedTimeBarrier {
    /// Stable protocol identity used for outgoing frames and inbound route validation.
    federate_id: FederateId,
    /// Persistent ordered protocol connection to the RTI.
    client: FederateProtocolClient,
    /// Federation route metadata keyed by its stable endpoint identifier.
    routes: BTreeMap<crate::EndpointId, FederateClientRoute>,
    /// Shared terminal fault reported by the runtime endpoint workers, if any.
    faults: boomerang_runtime::FederatedFaultState,
    /// Most recently accepted RTI grant, used to satisfy repeated or older requests locally.
    last_granted: Option<boomerang_runtime::Tag>,
    /// Successfully queued `NET` request still awaiting a sufficient `TAG` response.
    pending_request: Option<WireTag>,
    /// Whether this barrier has entered its terminal stopped state.
    stopped: bool,
    /// Whether an earlier protocol, transport, or admission error made further grants unsafe.
    failed: bool,
    /// Maximum time spent waiting for an RTI frame before checking scheduler events again.
    poll_interval: StdDuration,
}
impl RtiFederatedTimeBarrier {
    /// Create a scheduler barrier for one federate runtime enclave.
    /// Route metadata binds runtime endpoints to source and target federates.
    #[tracing::instrument(
        level = "debug",
        skip(federate_id, client, routes),
        fields(federate = %federate_id)
    )]
    pub fn new(
        federate_id: FederateId,
        client: FederateProtocolClient,
        routes: impl IntoIterator<Item = FederateClientRoute>,
        faults: boomerang_runtime::FederatedFaultState,
    ) -> Result<Self, FederateClientError> {
        let mut route_map = BTreeMap::new();
        for route in routes {
            let endpoint = route.endpoint.clone();
            if route_map.insert(endpoint.clone(), route).is_some() {
                return Err(FederateClientError::DuplicateRoute(endpoint));
            }
        }

        Ok(Self {
            federate_id,
            client,
            routes: route_map,
            faults,
            last_granted: None,
            pending_request: None,
            stopped: false,
            failed: false,
            poll_interval: StdDuration::from_millis(1),
        })
    }

    /// Request and wait for an RTI TAG grant for `tag`.
    /// Inbound MSG frames are scheduled while the scheduler is blocked.
    #[tracing::instrument(
        level = "debug",
        skip(self, tag, event_rx),
        fields(federate = %self.federate_id, tag = %tag)
    )]
    pub fn wait_for_tag(
        &mut self,
        tag: boomerang_runtime::Tag,
        event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    ) -> Result<Option<boomerang_runtime::AsyncEvent>, FederateClientError> {
        if self.stopped {
            return Err(FederateClientError::RtiStopped);
        }
        if self.failed {
            return Err(FederateClientError::BarrierFailed);
        }
        if self
            .last_granted
            .is_some_and(|last_granted| tag <= last_granted)
        {
            return Ok(None);
        }

        if let Err(error) = self.check_runtime_fault() {
            return self.fail(error);
        }

        let requested = crate::wire_tag_from_runtime(tag)?;
        if self.pending_request != Some(requested) {
            if let Err(error) = self.client.send(FederateToRti::Net {
                federate_id: self.federate_id.clone(),
                tag: requested,
            }) {
                return self.fail(error);
            }
            self.pending_request = Some(requested);
        }

        loop {
            if let Ok(Some(event)) = event_rx.try_recv() {
                return Ok(Some(event));
            }

            let message = match self.client.recv_timeout(self.poll_interval) {
                Ok(message) => message,
                Err(error) => {
                    return self.fail(error);
                }
            };
            let Some(message) = message else {
                continue;
            };
            match message {
                RtiToFederate::Tag { tag: granted } => {
                    let runtime_tag = match crate::runtime_tag_from_wire(granted) {
                        Ok(tag) => tag,
                        Err(error) => {
                            return self.fail(error.into());
                        }
                    };
                    self.last_granted = Some(runtime_tag);
                    if self
                        .pending_request
                        .is_some_and(|pending| granted >= pending)
                    {
                        self.pending_request = None;
                    }
                    if granted >= requested {
                        return Ok(None);
                    }
                    continue;
                }
                RtiToFederate::Msg {
                    source,
                    endpoint,
                    tag,
                    payload,
                } => {
                    let result =
                        self.schedule_inbound_msg(source, endpoint, tag, &payload, event_rx);
                    if result.is_err() {
                        self.pending_request = None;
                        self.failed = true;
                    }
                    return result;
                }
                RtiToFederate::Stop => {
                    self.pending_request = None;
                    self.stopped = true;
                    return Err(FederateClientError::RtiStopped);
                }
                RtiToFederate::Error { message } => {
                    return self.fail(FederateClientError::RtiError { message });
                }
                RtiToFederate::Start { .. } => {
                    return self.fail(FederateClientError::Protocol(
                        "unexpected duplicate Start frame".into(),
                    ));
                }
            }
        }
    }

    /// Report LTC after every reaction-emitted MSG has entered the ordered client mailbox.
    #[tracing::instrument(
        level = "debug",
        skip(self, tag),
        fields(federate = %self.federate_id, tag = %tag)
    )]
    pub fn report_logical_tag_complete(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), FederateClientError> {
        if self.failed {
            return Err(FederateClientError::BarrierFailed);
        }
        if let Err(error) = self.check_runtime_fault() {
            return self.fail(error);
        }
        if let Err(error) = self.send_ltc(tag) {
            return self.fail(error);
        }
        Ok(())
    }

    /// Send a final Stop frame for this federate after its scheduler has terminated.
    #[tracing::instrument(
        level = "debug",
        skip(self),
        fields(federate = %self.federate_id)
    )]
    pub fn stop(&mut self) -> Result<(), FederateClientError> {
        if self.stopped {
            return Ok(());
        }

        let fault_result = self.check_runtime_fault();
        let net_result = self.client.send(FederateToRti::Net {
            federate_id: self.federate_id.clone(),
            tag: WireTag::FOREVER,
        });
        let stop_result = self.client.send(FederateToRti::Stop {
            federate_id: self.federate_id.clone(),
        });
        self.pending_request = None;
        self.stopped = true;
        fault_result?;
        net_result?;
        stop_result?;
        Ok(())
    }

    /// Schedule one inbound MSG payload through the handler attached during lowering.
    /// Returns the scheduler wake event produced by that async scheduling operation.
    #[tracing::instrument(
        level = "debug",
        skip(self, source, endpoint, tag, payload, event_rx),
        fields(
            federate = %self.federate_id,
            source = %source,
            endpoint = %endpoint,
            tag = %tag,
            payload_len = payload.len()
        )
    )]
    fn schedule_inbound_msg(
        &mut self,
        source: FederateId,
        endpoint: crate::EndpointId,
        tag: WireTag,
        payload: &[u8],
        event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    ) -> Result<Option<boomerang_runtime::AsyncEvent>, FederateClientError> {
        let route = self.route_for(&endpoint)?;
        if route.target != self.federate_id {
            return Err(FederateClientError::RouteTargetMismatch {
                endpoint: endpoint.clone(),
                route_target: route.target.clone(),
                federate_id: self.federate_id.clone(),
            });
        }
        if route.source != source {
            return Err(FederateClientError::InboundSourceMismatch {
                endpoint: endpoint.clone(),
                observed_source: source,
                route_source: route.source.clone(),
            });
        }

        let runtime_tag = crate::runtime_tag_from_wire(tag)?;
        let inbound = route
            .inbound
            .as_ref()
            .ok_or_else(|| FederateClientError::UnboundInboundRoute(endpoint.clone()))?;
        inbound.schedule(runtime_tag, payload)?;

        event_rx
            .recv()
            .map(Some)
            .map_err(|_| FederateClientError::SchedulerEventChannelClosed { endpoint })
    }

    fn check_runtime_fault(&self) -> Result<(), FederateClientError> {
        match self.faults.get() {
            Some(error) => Err(FederateClientError::RuntimeEndpoint(error)),
            None => Ok(()),
        }
    }

    fn fail<T>(&mut self, error: FederateClientError) -> Result<T, FederateClientError> {
        self.pending_request = None;
        self.failed = true;
        Err(error)
    }

    /// Send LTC for a scheduler tag through the federate protocol client.
    #[tracing::instrument(
        level = "debug",
        skip(self, tag),
        fields(federate = %self.federate_id, tag = %tag)
    )]
    fn send_ltc(&self, tag: boomerang_runtime::Tag) -> Result<(), FederateClientError> {
        self.client.send(FederateToRti::Ltc {
            federate_id: self.federate_id.clone(),
            tag: crate::wire_tag_from_runtime(tag)?,
        })
    }

    fn route_for(
        &self,
        endpoint: &crate::EndpointId,
    ) -> Result<&FederateClientRoute, FederateClientError> {
        self.routes
            .get(endpoint)
            .ok_or_else(|| FederateClientError::UnknownRoute(endpoint.clone()))
    }
}
impl boomerang_runtime::FederatedTimeBarrier for RtiFederatedTimeBarrier {
    fn acquire_tag(
        &mut self,
        tag: boomerang_runtime::Tag,
        event_rx: &boomerang_runtime::Receiver<boomerang_runtime::AsyncEvent>,
    ) -> Result<boomerang_runtime::FederatedBarrierOutcome, boomerang_runtime::FederatedBarrierError>
    {
        self.wait_for_tag(tag, event_rx)
            .map(|event| match event {
                Some(event) => boomerang_runtime::FederatedBarrierOutcome::Interrupted(event),
                None => boomerang_runtime::FederatedBarrierOutcome::Granted,
            })
            .map_err(boomerang_runtime::FederatedBarrierError::from_error)
    }

    fn logical_tag_complete(
        &mut self,
        tag: boomerang_runtime::Tag,
    ) -> Result<(), boomerang_runtime::FederatedBarrierError> {
        self.report_logical_tag_complete(tag)
            .map_err(boomerang_runtime::FederatedBarrierError::from_error)
    }
}
#[cfg(test)]
mod tests {
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
        connect_client_with_fake_rti_and_mailbox(federate_id, FederateClientMailbox::new(), rti)
            .await
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
        boomerang_runtime::FederatedInboundEndpoint,
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
        let endpoint = boomerang_runtime::FederatedInboundEndpoint::new(
            context,
            action_ref,
            Box::new(|bytes: &[u8]| {
                std::str::from_utf8(bytes)
                    .map_err(|error| {
                        boomerang_runtime::FederatedEndpointError::codec(error.to_string())
                    })?
                    .parse::<u32>()
                    .map_err(|error| {
                        boomerang_runtime::FederatedEndpointError::codec(error.to_string())
                    })
            }),
        )
        .unwrap();
        (endpoint, enclave.event_rx, action_key, enclave.shutdown_tx)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bridge_sends_net_outbound_msg_and_ltc_frames() {
        boomerang_util::test_tracing::init_with_directive("boomerang_federated=debug");

        let mut connections =
            crate::FederatedRuntimeConnections::new([fed("source"), fed("sink")], [route()])
                .unwrap();
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
                send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO })
                    .await;
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("source"),
            client,
            [route()],
            boomerang_runtime::FederatedFaultState::default(),
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
            .send(boomerang_runtime::FederatedOutboundCommand::Msg(
                boomerang_runtime::FederatedOutboundMessage {
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("sink"),
            client,
            [inbound_route],
            boomerang_runtime::FederatedFaultState::default(),
        )
        .unwrap();

        let event = barrier
            .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
            .unwrap()
            .expect("inbound MSG should interrupt the barrier wait");
        match event {
            boomerang_runtime::AsyncEvent::Logical { tag, target, value } => {
                assert_eq!(tag, boomerang_runtime::Tag::ZERO);
                assert_eq!(
                    target,
                    boomerang_runtime::AsyncEventTarget::Action(action_key)
                );
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("sink"),
            client,
            [inbound_route],
            boomerang_runtime::FederatedFaultState::default(),
        )
        .unwrap();

        for expected in [41, 42] {
            let event = barrier
                .wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx)
                .unwrap()
                .expect("each preceding MSG must interrupt before TAG");
            let boomerang_runtime::AsyncEvent::Logical { tag, target, value } = event else {
                panic!("expected logical async event");
            };
            assert_eq!(tag, boomerang_runtime::Tag::ZERO);
            assert_eq!(
                target,
                boomerang_runtime::AsyncEventTarget::Action(action_key)
            );
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
    async fn inbound_admission_failure_makes_the_barrier_terminal_before_later_tag() {
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("sink"),
            client,
            [inbound_route],
            boomerang_runtime::FederatedFaultState::default(),
        )
        .unwrap();

        assert!(matches!(
            barrier.wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx),
            Err(FederateClientError::RuntimeEndpoint(_))
        ));
        assert!(barrier.failed);
        assert!(matches!(
            barrier.wait_for_tag(boomerang_runtime::Tag::ZERO, &event_rx),
            Err(FederateClientError::BarrierFailed)
        ));
        assert!(matches!(
            barrier.report_logical_tag_complete(boomerang_runtime::Tag::ZERO),
            Err(FederateClientError::BarrierFailed)
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
                send_rti_to_federate(&mut transport, RtiToFederate::Tag { tag: WireTag::ZERO })
                    .await;
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("sink"),
            client,
            [inbound_route],
            boomerang_runtime::FederatedFaultState::default(),
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
        let (client, rti) =
            connect_client_with_fake_rti(fed("source"), |mut transport| async move {
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
        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("source"),
            client,
            [route()],
            boomerang_runtime::FederatedFaultState::default(),
        )
        .unwrap();

        assert!(matches!(
            boomerang_runtime::FederatedTimeBarrier::acquire_tag(
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
        let (client, rti) =
            connect_client_with_fake_rti(fed("source"), |mut transport| async move {
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

        let mut barrier = RtiFederatedTimeBarrier::new(
            fed("source"),
            client,
            [route()],
            boomerang_runtime::FederatedFaultState::default(),
        )
        .unwrap();

        barrier.stop().unwrap();
        assert_eq!(barrier.pending_request, None);

        rti.await.unwrap();
    }
}
