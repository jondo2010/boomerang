//! This module implements the federate state machine client for a connection to the RTI.

use std::{
    net::SocketAddr,
    time::{self, Duration},
};

use boomerang_core::keys::PortKey;
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc,
};

use tokio_util::codec::Framed;

use crate::{bincodec, FedIds, FederateKey, NeighborStructure, RejectReason, RtiMsg, Timestamp};

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
#[derive(Debug)]
pub struct Config {
    fed_ids: FedIds,
    neighbors: NeighborStructure,
}

impl Config {
    pub fn new(federate: FederateKey, federation_id: &str, neighbors: NeighborStructure) -> Self {
        Config {
            fed_ids: FedIds {
                federate,
                federation_id: federation_id.to_owned(),
            },
            neighbors,
        }
    }
}

pub struct Client {
    start_time: Timestamp,
    sender: mpsc::UnboundedSender<RtiMsg>,
    handle: tokio::task::JoinHandle<()>,
}

impl Client {
    /// Send the specified timestamped message to the specified port in the specified federate via
    /// the RTI or directly to a federate depending on the given socket. The timestamp is calculated
    /// as current_logical_time + additional delay which is greater than or equal to zero.
    ///
    /// The port should be an input port of a reactor in the destination federate.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn send_timed_message<T>(
        &self,
        federate: FederateKey,
        port: PortKey,
        delay: Option<Duration>,
        message: T,
    ) -> Result<(), ClientError>
    where
        T: std::fmt::Debug + Serialize,
    {
        if self.handle.is_finished() {
            panic!("Federate has already shut down.");
        }

        // Apply the additional delay to the current tag and use that as the intended tag of the outgoing message
        //tag_t current_message_intended_tag = _lf_delay_tag(lf_tag(), additional_delay);

        //self.sender.send(RtiMsg::TaggedMessage((), ()));

        Ok(())
    }
}

/// Connect to an RTI and perform initial handshaking.
#[tracing::instrument]
pub async fn connect_to_rti(addr: SocketAddr, config: Config) -> Result<Client, ClientError> {
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
            .send(RtiMsg::FedIds(config.fed_ids))
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
                .send(RtiMsg::NeighborStructure(config.neighbors))
                .await
                .map_err(ClientError::from)?;

            //TODO clock sync / UDP port
            frame
                .send(RtiMsg::UdpPort(crate::ClockSyncStat::Off))
                .await
                .map_err(ClientError::from)?;

            let timestamp = get_start_time_from_rti(&mut frame).await?;
            tracing::debug!("Received start time from RTI: {timestamp:?}");

            let (sender, mut receiver) = mpsc::unbounded_channel::<RtiMsg>();

            let handle = tokio::spawn(async move {
                while let Some(msg) = receiver.recv().await {
                    if let Err(err) = frame.send(msg).await {
                        tracing::error!("Error sending message to RTI: {err:?}");
                    }
                }
            });

            Ok(Client {
                start_time: timestamp,
                sender,
                handle,
            })
        }
        _ => {
            tracing::error!("RTI sent an unexpected message: {msg:?}");
            Err(ClientError::UnexpectedMessage(msg))
        }
    }
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
