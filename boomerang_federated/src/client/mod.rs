//! Federate-side protocol bridge for one persistent federate.

use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration as StdDuration,
};

use futures_util::{Sink, SinkExt, TryStream, TryStreamExt};
use tokio::task::JoinHandle;

#[cfg(feature = "runtime")]
use crate::RuntimeBridgeError;
#[cfg(feature = "runtime")]
use crate::WireTag;
use crate::{FederateId, FederateToRti, ProtocolFrame, RtiToFederate, TransportError};

#[cfg(all(test, feature = "runtime"))]
mod tests;

#[cfg(feature = "runtime")]
pub(crate) mod coordination;
#[cfg(feature = "runtime")]
pub use coordination::RtiLogicalTimeCoordinator;

#[derive(Debug, thiserror::Error)]
pub enum FederateClientError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    #[cfg(feature = "runtime")]
    #[error("runtime bridge error: {0}")]
    RuntimeBridge(#[from] RuntimeBridgeError),

    #[cfg(feature = "runtime")]
    #[error("runtime endpoint error: {0}")]
    RuntimeEndpoint(#[from] crate::FederatedEndpointError),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("RTI error: {message}")]
    RtiError { message: String },

    #[error("RTI stopped the federate session")]
    RtiStopped,

    #[cfg(feature = "runtime")]
    #[error(
        "RTI logical-time coordinator is terminal after an earlier protocol or admission failure"
    )]
    CoordinationFailed,

    #[error("federate protocol client is closed")]
    ClientClosed,

    #[error("timed out waiting for federate protocol delivery")]
    DeliveryTimeout,

    #[cfg(feature = "runtime")]
    #[error("scheduler event channel closed after scheduling inbound endpoint `{endpoint}`")]
    SchedulerEventChannelClosed { endpoint: crate::EndpointId },

    #[cfg(feature = "runtime")]
    #[error("duplicate federated client route for endpoint `{0}`")]
    DuplicateRoute(crate::EndpointId),

    #[cfg(feature = "runtime")]
    #[error("unknown federated client route for endpoint `{0}`")]
    UnknownRoute(crate::EndpointId),

    #[cfg(feature = "runtime")]
    #[error("federated client route for endpoint `{0}` has no inbound runtime binding")]
    UnboundInboundRoute(crate::EndpointId),

    #[cfg(feature = "runtime")]
    #[error("federated client route for endpoint `{0}` already has an inbound runtime binding")]
    DuplicateInboundBinding(crate::EndpointId),

    #[cfg(feature = "runtime")]
    #[error(
        "route for endpoint `{endpoint}` has source `{route_source}`, expected `{federate_id}`"
    )]
    RouteSourceMismatch {
        endpoint: crate::EndpointId,
        route_source: FederateId,
        federate_id: FederateId,
    },

    #[cfg(feature = "runtime")]
    #[error(
        "route for endpoint `{endpoint}` has target `{route_target}`, expected `{federate_id}`"
    )]
    RouteTargetMismatch {
        endpoint: crate::EndpointId,
        route_target: FederateId,
        federate_id: FederateId,
    },

    #[cfg(feature = "runtime")]
    #[error(
        "inbound MSG for endpoint `{endpoint}` came from `{observed_source}`, but route source is `{route_source}`"
    )]
    InboundSourceMismatch {
        endpoint: crate::EndpointId,
        observed_source: FederateId,
        route_source: FederateId,
    },

    #[cfg(feature = "runtime")]
    #[error("received TAG {received} while waiting for {requested}")]
    UnexpectedTag {
        requested: WireTag,
        received: WireTag,
    },
}

enum ClientInput {
    Message(RtiToFederate),
    Transport(TransportError),
    Protocol(String),
    Closed,
}

#[derive(Debug)]
struct OutboundRequest {
    message: FederateToRti,
    delivered: Option<mpsc::Sender<Result<(), TransportError>>>,
}

/// Cloneable sender for a federate's single ordered protocol-outbound queue.
#[derive(Debug, Clone)]
pub struct FederateProtocolSender {
    outgoing: tokio::sync::mpsc::UnboundedSender<OutboundRequest>,
}

impl FederateProtocolSender {
    pub fn send(&self, message: FederateToRti) -> Result<(), FederateClientError> {
        self.outgoing
            .send(OutboundRequest {
                message,
                delivered: None,
            })
            .map_err(|_| FederateClientError::ClientClosed)
    }

    #[cfg(feature = "runtime")]
    fn send_confirmed(
        &self,
        message: FederateToRti,
        timeout: StdDuration,
    ) -> Result<(), FederateClientError> {
        let (delivered, delivery) = mpsc::channel();
        self.outgoing
            .send(OutboundRequest {
                message,
                delivered: Some(delivered),
            })
            .map_err(|_| FederateClientError::ClientClosed)?;
        match delivery.recv_timeout(timeout) {
            Ok(result) => result.map_err(FederateClientError::Transport),
            Err(RecvTimeoutError::Timeout) => Err(FederateClientError::DeliveryTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(FederateClientError::ClientClosed),
        }
    }
}

/// A prebuildable protocol mailbox whose receiver is connected to a transport at execution time.
#[derive(Debug)]
pub struct FederateClientMailbox {
    sender: FederateProtocolSender,
    receiver: tokio::sync::mpsc::UnboundedReceiver<OutboundRequest>,
}

impl FederateClientMailbox {
    pub fn new() -> Self {
        let (outgoing, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            sender: FederateProtocolSender { outgoing },
            receiver,
        }
    }

    pub fn sender(&self) -> FederateProtocolSender {
        self.sender.clone()
    }

    pub fn try_recv(&mut self) -> Result<Option<FederateToRti>, FederateClientError> {
        match self.receiver.try_recv() {
            Ok(request) => Ok(Some(request.message)),
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
        tokio::sync::mpsc::UnboundedReceiver<OutboundRequest>,
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
    outgoing: FederateProtocolSender,
    incoming: mpsc::Receiver<ClientInput>,
    start_unix_epoch_ns: i128,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl FederateProtocolClient {
    /// Connect a federate transport to the RTI and complete the Hello/Start handshake.
    /// Background reader and writer tasks are spawned for the live session.
    #[cfg_attr(feature = "runtime", tracing::instrument(
        level = "debug",
        skip(federate_id, sink, stream),
        fields(federate = %federate_id)
    ))]
    pub async fn connect<S, R>(
        federate_id: FederateId,
        sink: S,
        stream: R,
    ) -> Result<Self, FederateClientError>
    where
        S: Sink<ProtocolFrame> + Send + Unpin + 'static,
        S::Error: Into<TransportError> + Send + 'static,
        R: TryStream<Ok = ProtocolFrame> + Send + Unpin + 'static,
        R::Error: Into<TransportError> + Send + 'static,
    {
        Self::connect_with_mailbox(federate_id, sink, stream, FederateClientMailbox::new()).await
    }

    /// Connect a transport using an outbound mailbox created during runtime lowering.
    pub async fn connect_with_mailbox<S, R>(
        federate_id: FederateId,
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

    #[cfg(feature = "runtime")]
    pub(crate) fn send_confirmed(
        &self,
        message: FederateToRti,
        timeout: StdDuration,
    ) -> Result<(), FederateClientError> {
        self.outgoing.send_confirmed(message, timeout)
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

fn spawn_writer<S>(
    mut sink: S,
    mut outgoing: tokio::sync::mpsc::UnboundedReceiver<OutboundRequest>,
    incoming: mpsc::Sender<ClientInput>,
) -> JoinHandle<()>
where
    S: Sink<ProtocolFrame> + Send + Unpin + 'static,
    S::Error: Into<TransportError> + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(request) = outgoing.recv().await {
            let result = sink
                .send(ProtocolFrame::FederateToRti(request.message))
                .await
                .map_err(Into::into);
            match result {
                Ok(()) => {
                    if let Some(delivered) = request.delivered {
                        let _ = delivered.send(Ok(()));
                    }
                }
                Err(error) => {
                    if let Some(delivered) = request.delivered {
                        let _ = delivered.send(Err(error.clone()));
                    }
                    let _ = incoming.send(ClientInput::Transport(error));
                    break;
                }
            }
        }
    })
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
pub struct FederateClientRoute {
    pub endpoint: crate::EndpointId,
    pub source: FederateId,
    pub target: FederateId,
    inbound: Option<crate::FederatedInboundEndpoint>,
}

#[cfg(feature = "runtime")]
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

    pub(crate) fn bind_inbound(&mut self, inbound: crate::FederatedInboundEndpoint) {
        debug_assert!(self.inbound.is_none());
        self.inbound = Some(inbound);
    }

    pub(crate) fn inbound(&self) -> Option<&crate::FederatedInboundEndpoint> {
        self.inbound.as_ref()
    }
}
