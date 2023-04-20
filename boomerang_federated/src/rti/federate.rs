//! This module implements the RTI's state machine for connections to client federates.

use anyhow::anyhow;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_util::codec::Framed;

use crate::{bincodec, ClockSyncStat, FederateKey, NeighborStructure, RtiMsg, Tag};

use super::{start_time_sync::StartSync, ExecutionMode};

/// State of a federate during execution.
pub enum State {
    /// The federate has not connected.
    NotConnected,
    /// Most recent MSG_TYPE_NEXT_EVENT_TAG has been granted.
    Granted,
    /// Waiting for upstream federates.
    Pending,
}

/// Information about a federate known to the RTI, including its runtime state, mode of execution,
/// and connectivity with other federates.
///
/// The list of upstream and downstream federates does not include those that are connected via a
/// "physical" connection (one denoted with ~>) because those connections do not impose any
/// scheduling constraints.
pub struct Federate {
    /// ID of this federate.
    id: FederateKey,

    /// Start time synchronizer.
    start_time_sync: StartSync,

    /// Indicates whether clock synchronization is enabled.
    clock_sync: ClockSyncStat,

    /// The largest logical tag completed by the federate (or `None` if no LTC has been received).
    completed: Option<Tag>,
    /// The maximum TAG that has been granted so far (or `None` if none granted)
    last_granted: Option<Tag>,
    /// The maximum PTAG that has been provisionally granted (or `None` if none granted)
    last_provisionally_granted: Option<Tag>,
    /// Most recent NET received from the federate (or `None` if none received).
    next_event: Option<Tag>,

    /// Record of in-transit messages to this federate that are not yet processed.
    /// This record is ordered based on the time value of each message for a more efficient access.
    //in_transit_message_record_q_t* in_transit_message_tags;

    /// State of the federate.
    state: State,

    /// The upstream and downstream connections of this federate.
    neighbors: NeighborStructure,

    /// Receiver channel endpoint for receiving messages from other federates.
    receiver: UnboundedReceiver<RtiMsg>,

    /// Sender channel endspoints for sending messages to the neighbors.
    sender_channels: tinymap::TinySecondaryMap<FederateKey, UnboundedSender<RtiMsg>>,

    /// FAST or REALTIME.
    mode: ExecutionMode,
    //char server_hostname[INET_ADDRSTRLEN]; // Human-readable IP address and
    //int32_t server_port;    // port number of the socket server of the federate
    // if it has any incoming direct connections from other federates.
    // The port number will be -1 if there is no server or if the
    // RTI has not been informed of the port number.
    //struct in_addr server_ip_addr; // Information about the IP address of the socket
    // server of the federate.
    /// Indicates that the federate has requested stop or has replied to a request for stop from the
    /// RTI. Used to prevent double-counting a federate when handling lf_request_stop().
    requested_stop: bool,
}

impl Federate {
    pub fn new(
        id: FederateKey,
        start_time_sync: StartSync,
        clock_sync: ClockSyncStat,
        neighbors: NeighborStructure,
        receiver: UnboundedReceiver<RtiMsg>,
        sender_channels: tinymap::TinySecondaryMap<FederateKey, UnboundedSender<RtiMsg>>,
    ) -> Self {
        Federate {
            id,
            start_time_sync,
            clock_sync,
            completed: None,
            last_granted: None,
            last_provisionally_granted: None,
            next_event: None,
            state: State::NotConnected,
            neighbors,
            receiver,
            sender_channels,
            mode: ExecutionMode::RealTime,
            requested_stop: false,
        }
    }

    /// This is the RTI's main loop for each Federate connected to it.
    pub async fn run<T>(
        mut self,
        mut frame: Framed<T, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
    ) where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        loop {
            tokio::select! {
                // Messages from other federates forwarded by the RTI
                msg = self.receiver.recv() => {
                    if let Some(msg) = msg {
                        tracing::debug!(?msg, "Federate {:?} received message from RTI, forwarding to client.", self.id);
                        frame.send(msg).await.unwrap();
                    }
                }
                // Message from the federate over the socket
                msg = frame.next() => {
                    if let Some(msg) = msg {
                        tracing::debug!(?msg, "Federate {:?} received message over TCP.", self.id);

                        match msg {
                            Ok(RtiMsg::Timestamp(ts)) => {
                                let max_start_time = self
                                    .start_time_sync
                                    .propose_start_time(ts)
                                    .await
                                    .expect("TODO: handle error");

                                //TODO:
                                // Record this in-transit message in federate's in-transit message queue.

                                // Send back to the federate the maximum time plus an offset on a TIMESTAMP message.
                                let timestamp = max_start_time.offset(Duration::from_secs(1));
                                frame.send(RtiMsg::Timestamp(timestamp)).await.unwrap();
                            },
                            Ok(RtiMsg::TaggedMessage(tag, msg)) => {
                                let sender = self.sender_channels.get(msg.dest_federate).ok_or(anyhow!("Invalid Federate {:?}", msg.dest_federate)).unwrap();
                                sender.send(RtiMsg::TaggedMessage(tag, msg)).unwrap();
                            }
                            Ok(_) => {
                                tracing::error!(federate_id=?self.id, ?msg, "RTI received from federate an unrecognized TCP message.");
                            }
                            Err(err) => {
                                tracing::error!("Error decoding message from federate: {err}");
                            },
                        }
                    }
                }
            }
        }
    }
}
