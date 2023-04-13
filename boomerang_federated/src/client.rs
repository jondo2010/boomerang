//! This module implements the federate state machine client for a connection to the RTI.

use std::{net::SocketAddr, time};

use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc,
};
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tokio_util::codec::Framed;

use crate::{
    bincodec, Error, FedIds, FederateId, NeighborStructure, RejectReason, RtiMsg, Timestamp,
};

/// Configuration for a federate client
#[derive(Debug)]
pub struct Config {
    fed_ids: FedIds,
    neighbors: NeighborStructure,
}

impl Config {
    pub fn new(federate_id: FederateId, federation_id: &str, neighbors: NeighborStructure) -> Self {
        Config {
            fed_ids: FedIds {
                federate_id,
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
    /// Send the specified timestamped message to the specified port in the specified federate via the RTI or directly to a federate depending on the given socket. The timestamp is calculated as current_logical_time + additional delay which is greater than or equal to zero.  The port should be an input port of a reactor in the destination federate. This version does include the timestamp in the message.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn send_timed_message(&self) {}
}

#[tracing::instrument]
/// Connect to an RTI and perform initial handshaking.
pub async fn connect_to_rti(addr: SocketAddr, config: Config) -> Result<Client, Error> {
    tracing::info!("Connecting to RTI..");
    let client = TcpStream::connect(&addr)
        .await
        .map_err(|err| Error::Other(err.into()))?;
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
            .map_err(|err| Error::Other(err.into()))?;
    }

    tracing::debug!("Waiting for response to federation ID from the RTI.");

    match frame.next().await.ok_or(Error::HangUp)? {
        Ok(RtiMsg::Reject(reason)) => {
            tracing::error!("RTI rejected federate: {reason:?}");
            Err(Error::Reject(reason))
        }
        Ok(RtiMsg::Ack) => {
            tracing::debug!("Received acknowledgment from the RTI.");
            // Send neighbor information to the RTI.
            frame
                .send(RtiMsg::NeighborStructure(config.neighbors))
                .await
                .map_err(|err| Error::Other(err.into()))?;

            //TODO clock sync / UDP port
            frame
                .send(RtiMsg::UdpPort(crate::ClockSyncStat::Off))
                .await
                .map_err(|err| Error::Other(err.into()))?;

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
        Ok(msg) => {
            tracing::error!("RTI sent an unexpected message: {msg:?}");
            Err(Error::Reject(RejectReason::UnexpectedMessage))
        }
        Err(err) => {
            tracing::error!("Error in message from RTI: {err:?}");
            Err(Error::Other(err.into()))
        }
    }
}

#[tracing::instrument(level = "debug", skip(frame))]
async fn get_start_time_from_rti<T>(
    frame: &mut Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
) -> Result<Timestamp, Error>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    frame
        .send(RtiMsg::Timestamp(Timestamp::now()))
        .await
        .map_err(|err| Error::Other(err.into()))?;

    let msg = match frame.next().await {
        Some(Ok(msg)) => Ok(msg),
        _ => {
            tracing::warn!("RTI did not receive a message from federate.");
            Err(RejectReason::UnexpectedMessage)
        }
    }?;

    match msg {
        RtiMsg::Timestamp(start_time) => Ok(start_time),
        _ => {
            tracing::warn!("RTI did not return a timestamp.");
            Err(Error::Other(anyhow::anyhow!(
                "RTI did not return a timestamp."
            )))
        }
    }
}
