//! This module implements the federate state machine client for a connection to the RTI.

use std::net::SocketAddr;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use crate::{bincodec, Error, FedIds, FederateId, NeighborStructure, RejectReason, RtiMsg};

/// A federate client
pub struct Federate {
    fed_ids: FedIds,
    neighbors: NeighborStructure,
}

impl Federate {
    pub fn new(federate_id: FederateId, federation_id: &str, neighbors: NeighborStructure) -> Self {
        Federate {
            fed_ids: FedIds {
                federate_id,
                federation_id: federation_id.to_owned(),
            },
            neighbors,
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn connect_to_rti(&self, addr: SocketAddr) -> Result<(), Error> {
        tracing::info!("Connecting to RTI..");
        let client = TcpStream::connect(&addr)
            .await
            .map_err(|err| Error::Other(err.into()))?;
        let mut frame = Framed::new(client, bincodec::create::<RtiMsg>());

        // Have connected to an RTI, but not sure it's the right RTI.
        // Send a MSG_TYPE_FED_IDS message and wait for a reply.
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
                .send(RtiMsg::FedIds(self.fed_ids.clone()))
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
                    .send(RtiMsg::NeighborStructure(self.neighbors.clone()))
                    .await
                    .map_err(|err| Error::Other(err.into()))?;

                //TODO clock sync / UDP port
                frame
                    .send(RtiMsg::UdpPort(crate::ClockSyncStat::Off))
                    .await
                    .map_err(|err| Error::Other(err.into()))?;

                Ok(())
            }
            Ok(msg) => {
                tracing::error!("RTI sent an unexpected message: {msg:?}");
                Err(Error::Reject(RejectReason::UnexpectedMessage))
            }
            Err(err) => {
                tracing::error!("Error in message from RTI: {err:?}");
                Err(Error::Other(err.into()))
            }
        }?;

        Ok(())
    }
}
