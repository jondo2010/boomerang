//! This module implements the federate state machine client for a connection to the RTI.

use std::{borrow::Cow, net::SocketAddr};

use boomerang_core::{keys::PortKey, time::Tag};
use futures::{Sink, SinkExt, Stream, StreamExt};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc,
};

use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::codec::Framed;

use crate::{
    bincodec, FedIds, FederateKey, Message, NeighborStructure, PortAbsent, RejectReason, RtiMsg,
    Timestamp,
};

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
    fed_ids: FedIds,
    neighbors: NeighborStructure,
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

// Status of a given port at a given logical time.
//
// For non-network ports, unknown is unused.
pub enum PortStatus {
    /// The port is absent at the given logical time.
    Absent,
    /// The port is present at the given logical time.
    Present,
    /// It is unknown whether the port is present or absent (e.g., in a distributed application).
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Client {
    start_time: Timestamp,
    config: Config,
    sender: mpsc::UnboundedSender<RtiMsg>,
}

pub struct Handles {
    inbound_handle: tokio::task::JoinHandle<Result<(), ClientError>>,
    outbound_handle: tokio::task::JoinHandle<Result<(), ClientError>>,
}

impl Client {
    /// Send the specified timestamped message to the specified port in the specified federate via
    /// the RTI or directly to a federate depending on the given socket.
    ///
    /// The timestamp is calculated as current_logical_time + additional delay which is greater than or equal to zero.
    ///
    /// The port should be an input port of a reactor in the destination federate.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids, federate, port, tag))]
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

    /// In a federated execution with centralized coordination, this function returns a tag that is
    /// less than or equal to the specified tag when, as far as the federation is concerned, it is
    /// safe to commit to advancing to the returned tag. That is, all incoming network messages
    /// with tags less than the returned tag have been received.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids, federate, port, tag))]
    pub fn send_next_event_tag(&self, tag: Tag) -> Result<Tag, ClientError> {
        if self.sender.is_closed() {
            tracing::error!("RTI connection closed unexpectedly");
            return Err(ClientError::UnexpectedClose);
        }

        self.sender
            .send(RtiMsg::NextEventTag(tag))
            .map_err(|err| ClientError::Other(err.into()))?;

        Ok(tag)
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
            .send(RtiMsg::PortAbsent(PortAbsent {
                federate,
                port,
                tag,
            }))
            .map_err(|err| ClientError::Other(err.into()))
    }
}

/// Connect to an RTI and perform initial handshaking.
#[tracing::instrument]
pub async fn connect_to_rti(
    addr: SocketAddr,
    config: Config,
) -> Result<(Client, Handles), ClientError> {
    tracing::info!("Connecting to RTI..");

    let client = TcpStream::connect(&addr)
        .await
        .map_err(|err| ClientError::Other(err.into()))?;

    let mut frame = Framed::new(client, bincodec::create::<RtiMsg>());

    // Have connected to an RTI, but not sure it's the right RTI.
    // Send a `FedIds` message and wait for a reply.
    // Notify the RTI of the ID of this federate and its federation.

    if cfg!(feature = "authentication") {
        tracing::info!(
            "Connected to an RTI. Performing HMAC-based authentication using federation ID."
        );
        //perform_hmac_authentication(_fed.socket_TCP_RTI);
        todo!();
    } else {
        tracing::info!("Connected to an RTI. Sending federation ID for authentication.");

        frame
            .send(RtiMsg::FedIds(config.fed_ids.clone()))
            .await
            .map_err(ClientError::from)?;
    }

    tracing::debug!("Waiting for response to federation ID from the RTI.");

    let msg = frame
        .next()
        .await
        .ok_or(ClientError::UnexpectedClose)?
        .map_err(ClientError::from)?;

    match msg {
        RtiMsg::Reject(reason) => {
            tracing::error!("RTI rejected federate: {reason:?}");
            Err(ClientError::Rejected(reason))
        }
        RtiMsg::Ack => {
            tracing::debug!("Received acknowledgment from the RTI.");
            // Send neighbor information to the RTI.
            frame
                .send(RtiMsg::NeighborStructure(config.neighbors.clone()))
                .await
                .map_err(ClientError::from)?;

            //TODO clock sync / UDP port
            frame
                .send(RtiMsg::UdpPort(crate::ClockSyncStat::Off))
                .await
                .map_err(ClientError::from)?;

            let timestamp = get_start_time_from_rti(&mut frame).await?;
            tracing::debug!("Received start time from RTI: {timestamp:?}");

            let (sender, receiver) = mpsc::unbounded_channel::<RtiMsg>();
            let (frame_sink, frame_stream) = frame.split();

            let outbound_handle =
                tokio::spawn(outbound(UnboundedReceiverStream::new(receiver), frame_sink));
            let inbound_handle = tokio::spawn(inbound(sender.clone(), frame_stream));

            Ok((
                Client {
                    start_time: timestamp,
                    config,
                    sender,
                },
                Handles {
                    inbound_handle,
                    outbound_handle,
                },
            ))
        }
        _ => {
            tracing::error!("RTI sent an unexpected message: {msg:?}");
            Err(ClientError::UnexpectedMessage(msg))
        }
    }
}

/// Handle piping messages from the internal `receiver` channel to the RTI via `frame_sink`.
async fn outbound<R, S>(receiver: R, mut frame_sink: S) -> Result<(), ClientError>
where
    R: Stream<Item = RtiMsg> + Unpin,
    S: Sink<RtiMsg> + Unpin,
    <S as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    receiver
        .map(|msg| Ok(msg))
        .forward(&mut frame_sink)
        .await
        .map_err(|err| ClientError::Other(err.into()))
}

/// Handle receiving messages from the RTI via `frame_stream`
async fn inbound<S>(
    mut sender: mpsc::UnboundedSender<RtiMsg>,
    mut frame_stream: S,
) -> Result<(), ClientError>
where
    S: StreamExt<Item = Result<RtiMsg, bincode::Error>> + Unpin,
{
    while let Some(msg) = frame_stream.next().await {
        let msg = msg.map_err(ClientError::from)?;

        match msg {
            RtiMsg::TaggedMessage(tag, message) => {
                tracing::debug!("Received tagged message from RTI: {tag:?}");
                todo!();
            }
            RtiMsg::TagAdvanceGrant(tag) => {}
            RtiMsg::ProvisionalTagAdvanceGrant(tag) => {}
            RtiMsg::StopRequest(tag) => {}
            RtiMsg::StopGranted(tag) => {}
            RtiMsg::PortAbsent(port_absent) => {
                //handle_port_absent_message(_fed.socket_TCP_RTI, -1);
            }
            RtiMsg::ClockSyncT1 | RtiMsg::ClockSyncT4 => {
                tracing::error!(
                    "Federate {:?} received unexpected clock sync message from RTI on TCP socket.",
                    FederateKey::from(0)
                );
            }
            _ => {
                tracing::error!(
                    "Federate {:?} received unexpected message from RTI on TCP socket: {msg:?}",
                    FederateKey::from(0)
                );
            }
        }
    }

    Ok(())
}

#[tracing::instrument(level = "debug", skip(frame))]
async fn get_start_time_from_rti<T>(
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<Timestamp, ClientError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Send a `Timestamp` message and wait for a reply.
    frame
        .send(RtiMsg::Timestamp(Timestamp::now()))
        .await
        .map_err(ClientError::from)?;

    let msg = frame
        .next()
        .await
        .ok_or(ClientError::UnexpectedClose)?
        .map_err(ClientError::from)?;

    match msg {
        RtiMsg::Timestamp(start_time) => Ok(start_time),
        _ => {
            tracing::warn!("RTI did not return a timestamp.");
            Err(ClientError::UnexpectedMessage(msg))
        }
    }
}

/// Handle a timed message being received from a remote federate via the RTI or directly from other federates.
///
/// This will read the tag encoded in the header and calculate an offset to pass to the schedule function.
///
/// Instead of holding the mutex lock, this function calls _lf_increment_global_tag_barrier with the
/// tag carried in the message header as an argument. This ensures that the current tag will not
/// advance to the tag of the message if it is in the future, or the tag will not advance at all if
/// the tag of the message is now or in the past.
#[tracing::instrument(level = "debug")]
async fn handle_tagged_message(message: Message, federate: FederateKey) {}
