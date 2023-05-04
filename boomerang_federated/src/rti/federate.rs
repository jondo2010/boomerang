//! This module implements the RTI's state machine for connections to client federates.

use anyhow::anyhow;
use boomerang_core::time::Timestamp;
use futures::{SinkExt, StreamExt};
use std::{
    cmp::{min, Reverse},
    collections::BinaryHeap,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, watch},
};
use tokio_util::codec::Framed;

use crate::{bincodec, ClockSyncStat, FederateKey, Message, NeighborStructure, RtiMsg, Tag};

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

/// Update the next event tag of federate `federate_id`.
///
/// It will update the recorded next event tag of federate `federate_id` to the minimum of
/// `next_event_tag` and the minimum tag of in-transit messages (if any) to the federate. Will
/// try to see if the RTI can grant new TAG or PTAG messages to any downstream federates based
/// on this new next event tag.
///
/// federate_id – The id of the federate that needs to be updated.
/// next_event_tag – The next event tag for `federate_id`.
async fn next_event_tag_task(
    mut in_transit_receiver: mpsc::UnboundedReceiver<Tag>,
    mut next_event_receiver: mpsc::UnboundedReceiver<Tag>,
) {
    let mut in_transit_message_tags = BinaryHeap::new();

    loop {
        tokio::select! {
            biased;

            in_transit_tag = in_transit_receiver.recv() => {
                if let Some(in_transit_tag) = in_transit_tag {
                    in_transit_message_tags.push(Reverse(in_transit_tag));
                }
            }

            next_event_tag = next_event_receiver.recv() => {
                if let Some(next_event_tag) = next_event_tag {
                    // ported from update_federate_next_event_tag_locked()

                    let next_event_tag = match in_transit_message_tags.peek() {
                        Some(min_in_transit) => min_in_transit.0.min(next_event_tag),
                        _ => next_event_tag,
                    };

                }
            }
        }
    }
}

/// Channel endpoints for sending and receiving messages to/from a neighbor federate.
struct NeighborEndpoint {
    /// The largest logical tag completed by "neighbor" federate (or `None` if no LTC has been received).
    completed: watch::Receiver<Option<Tag>>,

    /// Record of in-transit messages to this federate that are not yet processed.
    in_transit_message_tags: mpsc::UnboundedSender<Tag>,

    /// Sender channel endpoint for sending messages to the "neighbor" federate.
    sender: mpsc::UnboundedSender<RtiMsg>,
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

    /// Record of in-transit messages to this federate that are not yet processed.
    /// This record is ordered based on the time value of each message for a more efficient access.
    //in_transit_message_record_q_t* in_transit_message_tags;

    /// State of the federate.
    state: State,

    /// The upstream and downstream connections of this federate.
    neighbors: NeighborStructure,

    neighbor_feds: tinymap::TinySecondaryMap<FederateKey, NeighborEndpoint>,

    /// Receiver channel endpoint for receiving messages from other federates.
    receiver: mpsc::UnboundedReceiver<RtiMsg>,

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
        let (tx, mut rx): (watch::Sender<Option<Tag>>, watch::Receiver<Option<Tag>>) =
            watch::channel(None);
        tx.send(Some(Tag::now(Timestamp::now()))).unwrap();

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
    #[tracing::instrument(skip(self, frame), fields(federate = ?self.id))]
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
                        tracing::debug!(?msg, "Received message from RTI, forwarding to client.");
                        frame.send(msg).await.unwrap();
                    }
                }
                // Message from the federate over the socket
                msg = frame.next() => {
                    if let Some(msg) = msg {
                        tracing::debug!(?msg, "Received message over TCP.");

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
                                self.handle_tagged_message(tag, msg).await;
                            }

                            Ok(RtiMsg::PortAbsent(port_absent)) => {
                                tracing::debug!(
                                    "RTI forwarding port absent message for port {port:?} to federate {federate:?}.",
                                    port=port_absent.port,
                                    federate=port_absent.federate
                                );

                                let sender = self
                                    .sender_channels
                                    .get(port_absent.federate)
                                    .ok_or(anyhow!("Invalid Federate {:?}", port_absent.federate))
                                    .unwrap();

                                sender.send(RtiMsg::PortAbsent(port_absent)).unwrap();
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
