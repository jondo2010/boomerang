//! This module implements the federate state machine client for a connection to the RTI.

use std::{cmp::Ordering, net::SocketAddr};

use anyhow::anyhow;
use boomerang_core::{keys::PortKey, time::Tag};
use futures::{stream::FusedStream, Sink, SinkExt, StreamExt, TryStreamExt};
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{mpsc, watch},
};

use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::codec::Framed;

use crate::{
    util::{bincodec, mpsc_sink::UnboundedSenderSink},
    FedIds, FederateKey, Message, NeighborStructure, RejectReason, RtiMsg, Timestamp,
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

#[derive(Debug)]
pub struct Client {
    start_time: Timestamp,
    config: Config,

    /// The message sender to the RTI.
    sender: mpsc::UnboundedSender<RtiMsg>,

    /// Receiver for tag advance grant messages from the RTI.
    //tag_receiver: mpsc::UnboundedReceiver<TagAdvanceGrant>,
    /// The last Logical Tag Complete (LTC) sent to the RTI.
    last_sent_ltc: Option<Tag>,
    /// Most recent Time Advance Grant received from the RTI, or `None` if never received.
    last_tag: Option<Tag>,
}

#[derive(Debug)]
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

    /// Returns a tag that is less than or equal to the specified tag when, as far as the federation
    /// is concerned, it is safe to commit to advancing to the returned tag. That is, all incoming
    /// network messages with tags less than the returned tag have been received.
    ///
    /// If this federate depends on upstream federates or sends data to downstream federates, then
    /// send to the RTI a NET, which will give the tag of the earliest event on the event queue, or,
    /// if the queue is empty, the timeout time, or, if there is no timeout, FOREVER.
    ///
    /// If there are network outputs that depend on physical actions, then insert a dummy event to
    /// ensure this federate advances its tag so that downstream federates can make progress.
    ///
    /// A NET is a promise saying that, absent network inputs, this federate will not produce an
    /// output message with tag earlier than the NET value.
    ///
    /// If there are upstream federates, then after sending a NET, this will block until either the
    /// RTI grants the advance to the requested time or the wait for the response from the RTI is
    /// interrupted by a change in the event queue (e.g., a physical action triggered or a network
    /// message arrived). If there are no upstream federates, then it will not wait for a TAG (which
    /// won't be forthcoming anyway) and returns the earliest tag on the event queue.
    ///
    /// If the federate has neither upstream nor downstream federates, then this returns the
    /// specified tag immediately without sending anything to the RTI.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids, federate, port, tag))]
    pub fn send_next_event_tag(&self, tag: Tag) -> Result<Tag, ClientError> {
        if self.sender.is_closed() {
            tracing::error!("RTI connection closed unexpectedly");
            return Err(ClientError::UnexpectedClose);
        }

        if self.config.neighbors.upstream.is_empty() && self.config.neighbors.downstream.is_empty()
        {
            // No upstream or downstream federates, so no need to send a NET
            tracing::debug!("Granted tag {tag} because the federate has neither upstream nor downstream federates.");
            return Ok(tag);
        }

        if let Some(last_tag) = self.last_tag {
            if last_tag >= tag {
                // The requested tag is less than or equal to the last tag, so no need to send a NET
                tracing::debug!("Granted tag {tag} because the requested tag is less than or equal to the last tag.", tag=last_tag);
                return Ok(last_tag);
            }
        }

        //TODO: tag_bounded_by_physical_time

        self.sender
            .send(RtiMsg::NextEventTag(tag))
            .map_err(|err| ClientError::Other(err.into()))?;

        Ok(tag)
    }

    /// Send a logical tag complete (LTC) message to the RTI unless an equal or later LTC has
    /// previously been sent.
    #[tracing::instrument(level = "debug", skip(self), fields(fed_ids=%self.config.fed_ids, federate, port, tag))]
    pub fn send_logical_tag_complete(&mut self, tag: Tag) {
        todo!();
    }

    /// Enqueue network control reactions.
    ///
    /// that will send an [`RtiMsg::PortAbsent`] message to downstream federates if a given network port is not present.
    pub fn enqueue_network_control_reactions(&self) {
        if self.config.neighbors.downstream.is_empty() {
            // No downstream federates, so no need to enqueue control reactions
            return;
        }
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

    //pub fn block_for_next_tag(&self) -> Result<Tag, ClientError> {
    //self.tag_receiver.blocking_recv()
    //}
}

/// Connect to an RTI and perform initial handshaking.
#[tracing::instrument(skip(config), fields(fed_ids=%config.fed_ids))]
pub async fn connect_to_rti(
    addr: SocketAddr,
    config: Config,
) -> Result<(Client, tokio::task::JoinHandle<Result<(), ClientError>>), ClientError> {
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

            let start_time = get_start_time_from_rti(&mut frame).await?;
            tracing::debug!("Received start time from RTI: {start_time:?}");

            //let (tag_sender, tag_receiver) = mpsc::unbounded_channel::<TagAdvanceGrant>();

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

            // Spawn a `ClientAsync` to handle messages received from the RTI.
            let client_handle = tokio::spawn(
                ClientAsync {
                    sink: UnboundedSenderSink::from(sender.clone()),
                    last_known_port_tag: tinymap::TinySecondaryMap::new(),
                    start_time,
                    current_tag: Tag::NEVER,
                    last_tag: Tag::NEVER,
                    is_last_tag_provisional: false,
                    stop_tag: Tag::FOREVER,
                }
                .run(
                    frame_stream
                        .fuse()
                        .map_err(|err| ClientError::Other(err.into())),
                ),
            );

            Ok((
                Client {
                    start_time,
                    config,
                    sender,
                    //tag_receiver,
                    last_sent_ltc: None,
                    last_tag: None,
                },
                client_handle,
            ))
        }
        _ => {
            tracing::error!("RTI sent an unexpected message: {msg:?}");
            Err(ClientError::UnexpectedMessage(msg))
        }
    }
}

/// State of the async side of the client.
struct ClientAsync<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    /// The message sink to the RTI.
    sink: Si,

    /// Last know status tag for each port.
    last_known_port_tag: tinymap::TinySecondaryMap<PortKey, watch::Sender<Tag>>,

    start_time: Timestamp,

    current_tag: Tag,

    /// Most recent `TimeAdvanceGrant` received from the RTI, or [`Tag::NEVER`] if none has been received.
    /// This is used to communicate between the listen_to_rti_TCP thread and the main federate thread.
    /// TODO should be channel?
    last_tag: Tag,

    /// Indicates whether the last TAG received is provisional or an ordinary TAG.
    /// If the last TAG has been provisional, network control reactions must be inserted.
    is_last_tag_provisional: bool,

    /// The tag at which the program should stop.
    stop_tag: Tag,
}

impl<Si> ClientAsync<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    /// Update the last known status tag of a network input port to the value of "tag". This is the largest tag at which the status (present or absent) of the port was known.
    fn update_last_known_status_on_input_port(
        &mut self,
        tag: Tag,
        port: PortKey,
    ) -> Result<(), ClientError> {
        let tag_sender = self
            .last_known_port_tag
            .get_mut(port)
            .ok_or(ClientError::Other(anyhow!(
                "Received port absent message for unknown port: {port:?}"
            )))?;

        match tag.cmp(&*tag_sender.borrow()) {
            Ordering::Less => {
                tracing::warn!("Attempt to update the last known status tag of network input port to an earlier tag was ignored.");
            }
            Ordering::Equal => {
                // If the intended tag for an input port is equal to the last known status, we need to increment the microstep.
                let tag = tag.delay(None);
                tracing::debug!(
                    "Updating the last known status tag to {tag}",
                    tag = tag.since(self.start_time)
                );
                tag_sender.send(tag).unwrap();
            }
            Ordering::Greater => {
                tracing::debug!(
                    "Updating the last known status tag to {tag}",
                    tag = tag.since(self.start_time)
                );
                tag_sender.send(tag).unwrap();
            }
        }

        Ok(())
    }

    /// Handle a time advance grant (TAG) message from the RTI.
    ///
    /// This updates the last known status tag for each network input port, and broadcasts a signal,
    /// which may cause a blocking control reaction to unblock.
    ///
    /// In addition, this updates the last known TAG/PTAG and broadcasts a notification of this
    /// update, which may unblock whichever worker thread is trying to advance time.
    ///
    /// @note This function is very similar to handle_provisinal_tag_advance_grant() except that it
    /// sets last_TAG_was_provisional to false.
    #[tracing::instrument(skip(self), fields(tag = %tag.since(self.start_time)))]
    async fn handle_tag_advance_grant(&mut self, tag: Tag) -> Result<(), ClientError> {
        // Update the last known status tag of all network input ports to the TAG received from the
        // RTI. Here we assume that the RTI knows the status of network ports up to and including
        // the granted tag, so by extension, we assume that the federate can safely rely on the RTI
        // to handle port statuses up until the granted tag.
        //self.update_last_known_status_on_input_ports(tag);

        // It is possible for this federate to have received a PTAG earlier with the same tag as this TAG.
        if tag >= self.last_tag {
            self.last_tag = tag;
            self.is_last_tag_provisional = false;
            tracing::debug!(
                "Received Time Advance Grant (TAG): {tag}.",
                tag = tag.since(self.start_time)
            );
        } else {
            tracing::error!("Received a TAG {tag} that wasn't larger than the previous TAG or PTAG {last_tag}. Ignoring the TAG.",
                tag = tag.since(self.start_time),
                last_tag = self.last_tag.since(self.start_time));
        }

        //self.waiting_for_TAG = false

        Ok(())
    }

    /// Handle a provisional tag advance grant (PTAG) message from the RTI.
    ///
    /// This updates the last known TAG/PTAG and broadcasts a notification of this update, which may unblock whichever worker thread is trying to advance time.
    /// If current_time is less than the specified PTAG, then this will also insert into the event_q a dummy event with the specified tag.
    /// This will ensure that the federate advances time to the specified tag and, for centralized coordination, inserts blocking reactions and null-message-sending output reactions at that tag.
    ///
    /// @note This function is similar to handle_tag_advance_grant() except that it sets last_TAG_was_provisional to true and also it does not update the last known tag for input ports.
    #[tracing::instrument(skip(self), fields(tag = %tag.since(self.start_time)))]
    async fn handle_provisional_tag_advance_grant(&mut self, tag: Tag) -> Result<(), ClientError> {
        // Sanity check
        if tag < self.last_tag || tag == self.last_tag && !self.is_last_tag_provisional {
            tracing::error!("Received a PTAG {tag} that is equal or earlier than an already received TAG {last_tag}.",
                tag = tag.since(self.start_time),
                last_tag = self.last_tag.since(self.start_time));
        }

        self.last_tag = tag;
        self.is_last_tag_provisional = true;
        tracing::debug!(
            "At tag {current_tag}, received Provisional Tag Advance Grant (PTAG): {tag}.",
            current_tag = self.current_tag.since(self.start_time),
            tag = tag.since(self.start_time),
        );

        //TODO handle the rest of this function

        Ok(())
    }

    /// Handle a port absent message received from a remote federate.
    ///
    /// This just sets the last known status tag of the port specified in the message.
    #[tracing::instrument(skip(self), fields(?federate, ?port, tag = %tag.since(self.start_time)))]
    async fn handle_port_absent(
        &mut self,
        federate: FederateKey,
        port: PortKey,
        tag: Tag,
    ) -> Result<(), ClientError> {
        tracing::debug!("Handling PortAbsent.");
        self.update_last_known_status_on_input_port(tag, port)
    }

    /// Handle a timed message being received from a remote federate via the RTI or directly from other federates.
    ///
    /// This will read the tag encoded in the header and calculate an offset to pass to the schedule function.
    ///
    /// Instead of holding the mutex lock, this function calls _lf_increment_global_tag_barrier with the
    /// tag carried in the message header as an argument. This ensures that the current tag will not
    /// advance to the tag of the message if it is in the future, or the tag will not advance at all if
    /// the tag of the message is now or in the past.
    #[tracing::instrument(skip(self), fields(tag = %tag.since(self.start_time)))]
    async fn handle_tagged_message(
        &mut self,
        tag: Tag,
        message: Message,
    ) -> Result<(), ClientError> {
        tracing::debug!("Handling tagged message from RTI");

        // Record the physical time of arrival of the message
        //TODO: action->trigger->physical_time_of_arrival = lf_time_physical();

        // In centralized coordination, a TAG message from the RTI can set the last_known_status_tag
        // to a future tag where messages have not arrived yet.
        self.update_last_known_status_on_input_port(tag, message.dest_port)?;

        // Check whether reactions need to be inserted directly into the reaction queue or a call to
        // schedule is needed. This checks if the intended tag of the message is for the current tag
        // or a tag that is already passed and if any control reaction is waiting on this port (or
        // the execution hasn't even started).

        // If the tag is intended for a tag that is passed, the control reactions would need to exit
        // because only one message can be processed per tag, and that message is going to be a
        // tardy message.
        // The actual tardiness handling is done inside _lf_insert_reactions_for_trigger.

        // To prevent multiple processing of messages per tag, we also need to check the port status.
        // For example, there could be a case where current tag is 10 with a control reaction
        // waiting, and a message has arrived with intended_tag 8.  This message will eventually
        // cause the control reaction to exit, but before that, a message with intended_tag of 9
        // could arrive before the control reaction has had a chance to exit. The port status is on
        // the other hand changed in this thread, and thus, can be checked in this scenario without
        // this race condition. The message with intended_tag of 9 in this case needs to wait one
        // microstep to be processed.

        if tag <= self.current_tag
        // The event is meant for the current or a previous tag.
        /*
        (is_a_control_reaction_waiting && // Check if a control reaction is waiting and
         trigger_status == unknown) ||             // if the status of the port is still unknown.
         _lf_execution_started == false)         // Or, execution hasn't even started, so it's safe to handle this event.
         */
        {
            // Since the message is intended for the current tag and a control reaction was waiting
            // for the message, trigger the corresponding reactions for this message.
            tracing::debug!(
                "Inserting reactions directly at tag {current_tag}. Intended tag: {intended_tag}.",
                current_tag = self.current_tag.since(self.start_time),
                intended_tag = tag.since(self.start_time)
            );
            //action->trigger->intended_tag = intended_tag;
            //_lf_insert_reactions_for_trigger(action->trigger, message_token);

            // Set the status of the port as present here to inform the network input control
            // reactions know that they no longer need to block. The reason for that is because the
            // network receiver reaction is now in the reaction queue keeping the precedence order
            // intact.
            //set_network_port_status(port_id, present);
        } else {
            // If no control reaction is waiting for this message, or if the intended tag is in the future, use schedule functions to process the message.

            // Before that, if the current time >= stop time, discard the message.  But only if the stop time is not equal to the start time!
            if tag >= self.stop_tag {
                tracing::warn!(
                    "Received message too late. Already at stop tag.\n Current tag is {current_tag} and intended tag is {intended_tag}.\n Discarding message.",
                    current_tag = self.current_tag.since(self.start_time),
                    intended_tag = tag.since(self.start_time)
                );
                //return;
            }
            tracing::debug!(
                "Calling schedule with tag {intended_tag}.",
                intended_tag = tag.since(self.start_time)
            );
            //schedule_message_received_from_network_already_locked(action->trigger, intended_tag, message_token);
        }

        Ok(())
    }

    /// Handle a message received from the RTI
    #[tracing::instrument(skip(self, msg))]
    async fn handle_message(&mut self, msg: RtiMsg) -> Result<(), ClientError> {
        match msg {
            RtiMsg::TaggedMessage(tag, message) => {
                self.handle_tagged_message(tag, message).await?;
            }
            RtiMsg::TagAdvanceGrant(tag, provisional) => {
                if provisional {
                    self.handle_provisional_tag_advance_grant(tag).await?;
                } else {
                    self.handle_tag_advance_grant(tag).await?;
                }
            }
            RtiMsg::StopRequest(tag) => {
                todo!();
            }
            RtiMsg::StopGranted(tag) => {
                todo!();
            }
            RtiMsg::PortAbsent(dest_federate, dest_port, tag) => {
                self.handle_port_absent(dest_federate, dest_port, tag)
                    .await?;
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
        Ok(())
    }

    #[tracing::instrument(skip(self, stream_results))]
    async fn run<St>(mut self, mut stream_results: St) -> Result<(), ClientError>
    where
        St: StreamExt<Item = Result<RtiMsg, ClientError>> + FusedStream + Unpin,
    {
        loop {
            tokio::select! {
                res = stream_results.select_next_some() => {
                    let msg = res?;
                    tracing::trace!("Received message from RTI {msg:?}");
                    match self.handle_message(msg).await {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::error!("Error handling message from RTI: {e}");
                            return Err(e);
                        }
                    }
                }
            }
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
    tracing::debug!("Sending Timestamp::now() to RTI.");
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
