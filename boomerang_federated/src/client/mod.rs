//! This module implements the federate state machine client for a connection to the RTI.

use std::net::SocketAddr;

use boomerang_core::{keys::PortKey, time::Tag};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use tokio::{
    net::TcpStream,
    sync::{mpsc, watch},
};

use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::codec::Framed;

use crate::{
    util::bincodec, FedIds, FederateKey, Message, NeighborStructure, RejectReason, RtiMsg,
    Timestamp,
};

mod handler;
use handler::Handler;

/// The error type for the client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Received an unexpected message from the RTI: {0:?}")]
    UnexpectedMessage(RtiMsg),

    #[error("The RTI rejected the federate: {0:?}")]
    Rejected(RejectReason),

    #[error("The RTI unexpectedly closed the connection")]
    UnexpectedClose,

    #[error("Serialization error")]
    Codec(#[from] bincode::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Configuration for a federate client
#[derive(Debug, Clone)]
pub struct Config {
    pub fed_ids: FedIds,
    pub neighbors: NeighborStructure,
}

impl Config {
    pub fn new(federate: FederateKey, federation_id: &str, neighbors: NeighborStructure) -> Self {
        Config {
            fed_ids: FedIds {
                federate_key: federate,
                federation: federation_id.to_owned(),
            },
            neighbors,
        }
    }
}

/// Status of a given port at a given logical time.
///
/// For non-network ports, unknown is unused.
pub enum PortStatus {
    /// The port is absent at the given logical time.
    Absent,
    /// The port is present at the given logical time.
    Present,
    /// It is unknown whether the port is present or absent (e.g., in a distributed application).
    Unknown,
}

#[derive(Debug)]
pub struct Client {
    /// The configuration for this federate.
    config: Config,
    /// Start time watch
    start_time_rx: watch::Receiver<Timestamp>,
    /// Negotiated start time
    start_time: Option<Timestamp>,
    /// The message sender to the RTI.
    sender: mpsc::UnboundedSender<RtiMsg>,

    /// Receiver for tag advance grant messages from the RTI.
    //tag_receiver: mpsc::UnboundedReceiver<TagAdvanceGrant>,
    /// The last Logical Tag Complete (LTC) sent to the RTI.
    last_sent_ltc: Option<Tag>,

    /// Most recent Time Advance Grant received from the RTI, or [`Tag::NEVER`] if none has been received.
    pub last_tag: watch::Receiver<Tag>,

    /// A record of the most recently sent NET (NextEventTag) message.
    last_net: Tag,

    /// `Handler` join handle
    handler_handle: tokio::task::JoinHandle<Result<(), ClientError>>,
}

impl Client {
    /// Get the most recent TAG received from the RTI.
    pub fn last_tag(&self) -> Tag {
        *self.last_tag.borrow()
    }

    /// Wait for the start time to negotiate with the RTI and other federates.
    pub async fn wait_for_start_time(&mut self) -> Result<Timestamp, anyhow::Error> {
        self.start_time_rx.changed().await?;
        self.start_time = Some(*self.start_time_rx.borrow());
        Ok(self.start_time.unwrap())
    }

    //pub async fn block_for_next_tag(&mut self) -> Result<Tag, ClientError> {
    //    self.last_tag.changed().await.map_err(anyhow::Error::from)?;
    //    Ok(*self.last_tag.borrow())
    //}

    /// Check if this federate has any upstream federates.
    pub fn has_upstream(&self) -> bool {
        !self.config.neighbors.upstream.is_empty()
    }

    /// Check if this federate has any downstream federates.
    pub fn has_downstream(&self) -> bool {
        !self.config.neighbors.downstream.is_empty()
    }

    /// Send a [`Tag`] to the RTI using the [`RtiMsg::NextEventTag`] message.
    #[tracing::instrument(skip(self), fields(tag))]
    pub fn send_next_event_tag(&mut self, tag: Tag) -> Result<(), ClientError> {
        if self.sender.is_closed() {
            tracing::error!("RTI connection closed unexpectedly");
            return Err(ClientError::UnexpectedClose);
        }

        self.sender
            .send(RtiMsg::NextEventTag(tag))
            .map_err(|err| ClientError::Other(err.into()))
            .and_then(|ret| {
                self.last_net = tag;
                tracing::debug!(
                    "Sent next event tag (NET) {} to RTI",
                    tag.since(self.start_time.unwrap_or(Timestamp::ZERO))
                );
                Ok(ret)
            })
    }

    /// Send the specified timestamped message to the specified port in the specified federate via
    /// the RTI or directly to a federate depending on the given socket.
    ///
    /// The timestamp is calculated as current_logical_time + additional delay which is greater than or equal to zero.
    ///
    /// The port should be an input port of a reactor in the destination federate.
    #[tracing::instrument(
        skip(self),
        fields(fed_ids=%self.config.fed_ids, federate, port, tag)
    )]
    pub fn send_timed_message<T>(
        &self,
        federate: FederateKey,
        port: PortKey,
        tag: Tag,
        message: T,
    ) -> Result<(), ClientError>
    where
        T: std::fmt::Debug + Serialize,
    {
        if self.sender.is_closed() {
            tracing::error!("RTI connection closed unexpectedly");
            return Err(ClientError::UnexpectedClose);
        }

        tracing::info!(
            "Sending message to federate {federate:?} port {port:?} at tag {}: {message:?}",
            tag.since(self.start_time.unwrap_or(Timestamp::ZERO))
        );

        // Apply the additional delay to the current tag and use that as the intended tag of the outgoing message
        //tag_t current_message_intended_tag = _lf_delay_tag(lf_tag(), additional_delay);

        self.sender
            .send(RtiMsg::TaggedMessage(
                tag,
                Message {
                    dest_port: port,
                    dest_federate: federate,
                    message: bincode::serialize(&message).map_err(ClientError::Codec)?,
                },
            ))
            .map_err(|err| ClientError::Other(err.into()))
    }

    /// Send a logical tag complete (LTC) message to the RTI unless an equal or later LTC has
    /// previously been sent.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids, federate, port, tag))]
    pub fn send_logical_tag_complete(&mut self, tag: Tag) {
        todo!();
    }

    /// Send a port absent message to federate, informing it that the current federate will not
    /// produce an event on this network port at the current logical time.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids))]
    pub fn send_port_absent_to_federate(
        &self,
        federate: FederateKey,
        port: PortKey,
        tag: Tag,
    ) -> Result<(), ClientError> {
        if self.sender.is_closed() {
            tracing::error!("RTI connection closed unexpectedly");
            return Err(ClientError::UnexpectedClose);
        }

        self.sender
            .send(RtiMsg::PortAbsent(federate, port, tag))
            .map_err(|err| ClientError::Other(err.into()))
    }
}

/// Connect to an RTI.
///
/// This will attempt to connect to the RTI at the specified address and will await for a response.
///
/// Once connected, a Tokio task is spawned with an [`AsyncClient`] that handles initial handshaking
/// and communication with the RTI.
///
/// A [`Client`] is returned that allows synchronous communication with the RTI through the `AsyncClient`.
#[tracing::instrument()]
pub async fn connect_to_rti<'a>(
    addr: SocketAddr,
    config: &'a Config,
) -> Result<Client, ClientError> {
    tracing::info!("Connecting to RTI..");

    let client = TcpStream::connect(&addr)
        .await
        .map_err(|err| ClientError::Other(err.into()))?;

    let frame = Framed::new(client, bincodec::create::<RtiMsg>());

    // Split the frame into a stream and a sink.
    let (frame_sink, frame_stream) = frame.split();

    // Wrap the sink in an unbounded channel so we can have multiple senders.
    let sender = {
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(
            // Forward messages from the internal `receiver` channel to the RTI via `frame_sink`.
            UnboundedReceiverStream::new(receiver)
                .map(Ok)
                .forward(frame_sink),
        );
        sender
    };

    let (start_time_tx, start_time_rx) = watch::channel(Timestamp::ZERO);
    let (last_tag_tx, last_tag_rx) = watch::channel(Tag::NEVER);

    let async_client = Handler::new(config, start_time_tx, last_tag_tx, sender.clone())?;

    // Spawn a `ClientAsync` to handle messages received from the RTI.
    let handler_handle = tokio::spawn(async move {
        let ret = async_client
            .run(
                frame_stream
                    .fuse()
                    .map_err(|err| ClientError::Other(err.into())),
            )
            .await;
        tracing::info!("Client exiting.");
        ret
    });

    Ok(Client {
        config: config.clone(),
        start_time_rx,
        start_time: None,
        sender,
        //tag_receiver,
        last_sent_ltc: None,
        last_tag: last_tag_rx,
        last_net: Tag::NEVER,
        handler_handle,
    })
}
