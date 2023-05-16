//! Client state and methods for handling async messages from the RTI.

use std::cmp::Ordering;

use boomerang_core::{
    keys::PortKey,
    time::{Tag, Timestamp},
};
use futures::{stream::FusedStream, StreamExt};
use tokio::sync::{mpsc, watch};

use crate::{FederateKey, Message, RtiMsg};

use super::ClientError;

/// `Handler` is the state for a spawned task that handles receiving messages from the RTI.
pub struct Handler {
    /// The configuration for this federate.
    config: super::Config,
    /// The sender end of the channel to send messages to the RTI.
    sender: mpsc::UnboundedSender<RtiMsg>,
    /// Last know status tag for each port.
    last_known_port_tag: tinymap::TinySecondaryMap<PortKey, watch::Sender<Tag>>,
    /// Negotiate start time with RTI
    start_time: watch::Sender<Timestamp>,

    current_tag: Tag,

    /// Most recent `TimeAdvanceGrant` received from the RTI, or [`Tag::NEVER`] if none has been received.
    /// This is used to communicate between the listen_to_rti_TCP thread and the main federate thread.
    /// TODO should be channel?
    last_tag: watch::Sender<Tag>,

    /// Indicates whether the last TAG received is provisional or an ordinary TAG.
    /// If the last TAG has been provisional, network control reactions must be inserted.
    is_last_tag_provisional: bool,

    /// The tag at which the program should stop.
    stop_tag: Tag,
}

impl Handler {
    pub fn new(
        config: &super::Config,
        start_time: watch::Sender<Timestamp>,
        last_tag: watch::Sender<Tag>,
        sender: mpsc::UnboundedSender<RtiMsg>,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            config: config.clone(),
            sender,
            last_known_port_tag: tinymap::TinySecondaryMap::new(),
            start_time,
            current_tag: Tag::NEVER,
            last_tag,
            is_last_tag_provisional: false,
            stop_tag: Tag::FOREVER,
        })
    }

    /// Update the last known status tag of a network input port to the value of "tag". This is the largest tag at which the status (present or absent) of the port was known.
    fn update_last_known_status_on_input_port(
        &mut self,
        tag: Tag,
        port: PortKey,
    ) -> Result<(), ClientError> {
        let tag_sender = self
            .last_known_port_tag
            .get_mut(port)
            .ok_or(ClientError::Other(anyhow::anyhow!(
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
                    tag = tag.since(*self.start_time.borrow())
                );
                tag_sender.send(tag).unwrap();
            }
            Ordering::Greater => {
                tracing::debug!(
                    "Updating the last known status tag to {tag}",
                    tag = tag.since(*self.start_time.borrow())
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
    #[tracing::instrument(skip(self), fields(tag = %tag.since(*self.start_time.borrow())))]
    async fn handle_tag_advance_grant(&mut self, tag: Tag) -> Result<(), ClientError> {
        // Update the last known status tag of all network input ports to the TAG received from the
        // RTI. Here we assume that the RTI knows the status of network ports up to and including
        // the granted tag, so by extension, we assume that the federate can safely rely on the RTI
        // to handle port statuses up until the granted tag.
        //self.update_last_known_status_on_input_ports(tag);

        // It is possible for this federate to have received a PTAG earlier with the same tag as
        // this TAG.
        let last_tag = *self.last_tag.borrow();
        if tag >= last_tag {
            self.last_tag.send_replace(tag);
            self.is_last_tag_provisional = false;
            tracing::debug!(
                "Received Time Advance Grant (TAG): {tag}.",
                tag = tag.since(*self.start_time.borrow())
            );
        } else {
            tracing::error!(
                "Received a TAG {tag} that wasn't larger than the previous TAG or PTAG {last_tag}. Ignoring the TAG.",
                tag = tag.since(*self.start_time.borrow()),
                last_tag = last_tag.since(*self.start_time.borrow())
            );
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
    #[tracing::instrument(skip(self), fields(tag = %tag.since(*self.start_time.borrow())))]
    async fn handle_provisional_tag_advance_grant(&mut self, tag: Tag) -> Result<(), ClientError> {
        // Sanity check
        let last_tag = *self.last_tag.borrow();
        if tag < last_tag || tag == last_tag && !self.is_last_tag_provisional {
            tracing::error!(
                "Received a PTAG {tag} that is equal or earlier than an already received TAG {last_tag}.",
                tag = tag.since(*self.start_time.borrow()),
                last_tag = last_tag.since(*self.start_time.borrow())
            );
            panic!();
        }

        self.last_tag.send_replace(tag);
        self.is_last_tag_provisional = true;
        tracing::debug!(
            "At tag {current_tag}, received Provisional Tag Advance Grant (PTAG): {tag}.",
            current_tag = self.current_tag.since(*self.start_time.borrow()),
            tag = tag.since(*self.start_time.borrow()),
        );

        //TODO handle the rest of this function

        Ok(())
    }

    /// Handle a port absent message received from a remote federate.
    ///
    /// This just sets the last known status tag of the port specified in the message.
    #[tracing::instrument(
        skip(self),
        fields(?federate, ?port, tag = %tag.since(*self.start_time.borrow()))
    )]
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
    #[tracing::instrument(
        skip(self),
        fields(tag = %tag.since(*self.start_time.borrow()))
    )]
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
                current_tag = self.current_tag.since(*self.start_time.borrow()),
                intended_tag = tag.since(*self.start_time.borrow())
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
                current_tag = self.current_tag.since(*self.start_time.borrow()),
                intended_tag = tag.since(*self.start_time.borrow())
            );
                //return;
            }
            tracing::debug!(
                "Calling schedule with tag {intended_tag}.",
                intended_tag = tag.since(*self.start_time.borrow())
            );
            //schedule_message_received_from_network_already_locked(action->trigger, intended_tag, message_token);
        }

        Ok(())
    }

    /// Handle a message received from the RTI
    #[tracing::instrument(skip(self, msg))]
    async fn handle_message(&mut self, msg: RtiMsg) -> Result<(), ClientError> {
        match msg {
            RtiMsg::Ack => {
                tracing::debug!("Received acknowledgment from the RTI.");

                // Send neighbor information to the RTI.
                self.sender
                    .send(RtiMsg::NeighborStructure(self.config.neighbors.clone()))
                    .map_err(anyhow::Error::from)?;

                //TODO clock sync / UDP port
                self.sender
                    .send(RtiMsg::UdpPort(crate::ClockSyncStat::Off))
                    .map_err(anyhow::Error::from)?;

                // Send a `Timestamp` message and wait for a reply.
                tracing::debug!("Sending Timestamp::now() to RTI.");
                self.sender
                    .send(RtiMsg::Timestamp(Timestamp::now()))
                    .map_err(anyhow::Error::from)?;

                Ok(())
            }
            RtiMsg::Timestamp(start_time) => {
                tracing::debug!("Received start time from RTI: {start_time:?}");
                self.start_time.send_replace(start_time);
                Ok(())
            }
            RtiMsg::Reject(reason) => {
                tracing::error!("RTI rejected federate: {reason:?}");
                Err(ClientError::Rejected(reason))
            }
            RtiMsg::TaggedMessage(tag, message) => self.handle_tagged_message(tag, message).await,
            RtiMsg::TagAdvanceGrant(tag, provisional) => {
                if provisional {
                    self.handle_provisional_tag_advance_grant(tag).await
                } else {
                    self.handle_tag_advance_grant(tag).await
                }
            }
            RtiMsg::StopRequest(tag) => {
                todo!();
            }
            RtiMsg::StopGranted(tag) => {
                todo!();
            }
            RtiMsg::PortAbsent(dest_federate, dest_port, tag) => {
                self.handle_port_absent(dest_federate, dest_port, tag).await
            }
            RtiMsg::ClockSyncT1 | RtiMsg::ClockSyncT4 => {
                Err(ClientError::UnexpectedMessage(msg))
                //tracing::error!( "Federate {:?} received unexpected clock sync message from RTI on TCP socket.", FederateKey::from(0));
            }
            _ => {
                Err(ClientError::UnexpectedMessage(msg))
                //tracing::error!( "Federate {:?} received unexpected message from RTI on TCP socket: {msg:?}", FederateKey::from(0));
            }
        }
    }

    #[tracing::instrument(skip(self, stream_results), fields(%self.config.fed_ids))]
    pub async fn run<St>(mut self, mut stream_results: St) -> Result<(), ClientError>
    where
        St: StreamExt<Item = Result<RtiMsg, ClientError>> + FusedStream + Unpin,
    {
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

            self.sender
                .send(RtiMsg::FedIds(self.config.fed_ids.clone()))
                .map_err(anyhow::Error::from)?;
        }

        tracing::debug!("Waiting for response to federation ID from the RTI.");

        loop {
            tokio::select! {
                res = stream_results.select_next_some() => {
                    let msg = res?;
                    //tracing::trace!("Received message from RTI {msg:?}");
                    match self.handle_message(msg).await {
                        Ok(()) => {
                            Ok(())
                        }
                        Err(e) => {
                            tracing::error!("Error handling message from RTI: {e}");
                            Err(e)
                        }
                    }
                }
            }?
        }
    }
}
