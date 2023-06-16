use std::{collections::BinaryHeap, time::Duration};

use boomerang_core::keys::PortKey;
use futures::{sink::SinkExt, stream::FusedStream, Sink, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::watch,
    task::JoinHandle,
};
use tokio_util::codec::Framed;

use super::start_time_sync::StartSync;
use crate::{
    util::bincodec, ClockSyncStat, Error, FederateKey, Message, NeighborStructure, RtiMsg, Tag,
    Timestamp,
};

pub struct Initial<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub frame: Framed<S, bincodec::BinCodec<RtiMsg, bincode::DefaultOptions>>,
    pub neighbors: NeighborStructure,
    pub clock_sync: ClockSyncStat,
}

/// Local state per federate.
#[derive(Debug)]
pub struct Federate<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    /// The message sink for the federate.
    sink: Si,
    /// Upstream and downstream neighbors.
    neighbors: NeighborStructure,

    /// Indicates whether clock synchronization is enabled.
    clock_sync: ClockSyncStat,
    /// Start time synchronizer.
    start_time_sync: StartSync,
    /// Record of in-transit messages to this federate that are not yet processed.
    in_transit_message_tags: BinaryHeap<Tag>,

    /// The largest logical tag completed by federate (or [`Timestamp::NEVER`] if no LTC has been received).
    completed: Tag,
    /// The maximum TAG that has been granted so far (or [`Timestamp::NEVER`] if none granted)
    last_granted: Tag,
    /// The maximum PTAG that has been provisionally granted (or [`Timestamp::NEVER`] if none granted)
    last_provisionally_granted: Tag,
    /// Most recent NET received from the federate (or [`Timestamp::NEVER`] if none received).
    next_event: Tag,

    /// Indicates that the federate has requested stop or has replied
    requested_stop: bool,
}

impl<Si> Federate<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn new(
        sink: Si,
        federate_key: FederateKey,
        neighbors: NeighborStructure,
        clock_sync: ClockSyncStat,
        start_time_sync: StartSync,
    ) -> Self {
        Self {
            sink,
            neighbors,
            clock_sync,
            start_time_sync,
            in_transit_message_tags: BinaryHeap::new(),
            completed: Tag::NEVER,
            last_granted: Tag::NEVER,
            last_provisionally_granted: Tag::NEVER,
            next_event: Tag::NEVER,
            requested_stop: false,
        }
    }
}

/// Structure that an RTI instance uses to keep track of its own and its corresponding federates' state.
//#[derive(Debug)]
pub struct Rti<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    // RTI's decided stop tag for federates
    max_stop_tag: Option<Tag>,

    /// Logical time at start of execution.
    start_time: watch::Receiver<Timestamp>,

    /// The federates.
    federates: tinymap::TinySecondaryMap<FederateKey, Federate<Si>>,

    /// The transitive set of upstream neighbors and their delays for each federate.
    transitive_upstream_neighbors:
        tinymap::TinySecondaryMap<FederateKey, Vec<(FederateKey, Duration)>>,
    // Number of federates handling stop
    //num_feds_handling_stop: usize,

    // Boolean indicating that all federates have exited.
    //all_federates_exited: bool,

    // The desired port specified by the user on the command line.
    //user_specified_port: u16,
    // The final port number that the TCP socket server ends up using.
    //final_port_TCP: u16,
    // The final port number that the UDP socket server ends up using. */
    //final_port_UDP: u16,

    // Clock synchronization information
    //clock_sync_global_status: ClockSyncStat,

    // Frequency (period in nanoseconds) between clock sync attempts.
    //clock_sync_period_ns: u64,

    // Number of messages exchanged for each clock sync attempt.
    //clock_sync_exchanges_per_interval: usize,
}

pub struct RtiHandles {
    pub start_time: Timestamp,
    pub rti_handle: JoinHandle<Result<(), Error>>,
    pub listener_handle: JoinHandle<()>,
}

impl<Si> Rti<Si>
where
    Si: SinkExt<RtiMsg> + Unpin + Clone + Send + 'static,
    <Si as Sink<RtiMsg>>::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn new(
        federates: tinymap::TinySecondaryMap<FederateKey, Federate<Si>>,
        transitive_upstream_neighbors: tinymap::TinySecondaryMap<
            FederateKey,
            Vec<(FederateKey, Duration)>,
        >,
        start_time: watch::Receiver<Timestamp>,
    ) -> Self {
        Self {
            federates,
            transitive_upstream_neighbors,
            max_stop_tag: None,
            start_time,
        }
    }

    /// Find the earliest tag at which the specified federate may experience its next event.
    ///
    /// This is the least next event tag (NET) of the specified federate and (transitively) upstream
    /// federates (with delays of the connections added). For upstream federates, we assume
    /// (conservatively) that federates upstream of those may also send an event. The result will
    /// never be less than the completion time of the federate (which may be NEVER, if the federate
    /// has not yet completed a logical time).
    ///
    /// FIXME: This could be made less conservative by building at code generation time a causality
    /// interface table indicating which outputs can be triggered by which inputs. For now, we
    /// assume any output can be triggered by any input.
    #[tracing::instrument(skip(self, federate_key))]
    fn transitive_next_upstream_event(&self, federate_key: FederateKey) -> Tag {
        // Build the weighted DAG of all upstream edges.
        let upstream_edges = self
            .federates
            .iter()
            .flat_map(|(k, f)| {
                f.neighbors
                    .upstream
                    .iter()
                    .map(move |&(k2, d)| (k2, k, d.as_secs_f64()))
            })
            .inspect(|(k1, k2, d)| {
                //tracing::debug!(federate1 = ?k1, federate2 = ?k2, delay = ?d, "upstream edge");
            });

        let g = petgraph::graph::DiGraph::<(), f64, FederateKey>::from_edges(upstream_edges);
        let paths = petgraph::algo::bellman_ford(&g, federate_key.into()).unwrap();
        g.node_indices()
            .map(|ix| {
                let next_event = &self.federates[FederateKey::from(ix.index())].next_event;
                next_event.delay(Duration::from_secs_f64(paths.distances[ix.index()]))
            })
            .min()
            .unwrap()
    }

    /// Send a tag advance grant (TAG) message to the specified federate.
    ///
    /// Do not send it if a previously sent PTAG was greater or if a previously sent TAG was greater
    /// or equal. This function will keep a record of this TAG in the federate's last_granted field.
    #[tracing::instrument(skip(self, federate_key), fields(tag = ?tag.since(*self.start_time.borrow())))]
    async fn send_tag_advance_grant(&mut self, federate_key: FederateKey, tag: Tag) {
        let fed = &mut self.federates[federate_key];
        if tag <= fed.last_granted || tag < fed.last_provisionally_granted {
            return;
        }

        fed.sink
            .send(RtiMsg::TagAdvanceGrant(tag, false))
            .await
            .unwrap();

        fed.last_granted = tag;
        tracing::debug!("RTI: Sent TAG({tag}) to {federate_key:?}.");
    }

    /// Send a provisional tag advance grant (PTAG) message to the specified federate and possibly
    /// all transitive upstream federates.
    ///
    /// Do not send it if a previously sent PTAG or TAG was greater or equal. This function will
    /// keep a record of this PTAG in the federate's last_provisionally_granted field.
    #[tracing::instrument(skip(self, federate_key), fields(tag = ?tag.since(*self.start_time.borrow())))]
    async fn send_provisional_tag_advance_grant(&mut self, federate_key: FederateKey, tag: Tag) {
        let fed = &mut self.federates[federate_key];
        if tag <= fed.last_granted || tag < fed.last_provisionally_granted {
            return;
        }

        // Send PTAG to all upstream federates, if they have not had a later or equal PTAG or TAG
        // sent previously and if their transitive NET is greater than or equal to the tag.

        // NOTE: This could later be replaced with a TNET mechanism once we have an available
        // encoding of causality interfaces.  That might be more efficient.
        for &(key, delay) in &self.transitive_upstream_neighbors[federate_key] {
            let fed = &mut self.federates[key];

            // If these tags are equal, then a TAG or PTAG should have already been granted, in
            // which case, another will not be sent. But it may not have been already granted.
            if key == federate_key || fed.next_event.delay(delay) >= tag {
                fed.sink
                    .send(RtiMsg::TagAdvanceGrant(tag, true))
                    .await
                    .unwrap();
                fed.last_provisionally_granted = tag;
                tracing::debug!(
                    "RTI: Sent PTAG({tag}) to {key:?}.",
                    tag = tag.since(*self.start_time.borrow())
                );
            }
        }
    }

    /// Determine whether the specified federate fed is eligible for a tag advance grant, (TAG) and, if
    /// so, send it one.
    ///
    /// This is called upon receiving a LTC, NET or resign from an upstream federate. This function
    /// calculates the minimum M over all upstream federates of the "after" delay plus the most recently
    /// received LTC from that federate. If M is greater than the most recently sent TAG to fed or
    /// greater than or equal to the most recently sent PTAG, then send a TAG(M) to fed and return.
    ///
    /// If the above conditions do not result in sending a TAG, then find the minimum M of the earliest
    /// possible future message from upstream federates. This is calculated by transitively looking at
    /// the most recently received NET message from upstream federates. If M is greater than the NET of
    /// the federate fed or the most recently sent PTAG to that federate, then send TAG to the federate
    /// with tag equal to the NET of fed or the PTAG. If M is equal to the NET of the federate, then
    /// send PTAG(M).
    ///
    /// This should be called whenever an immediately upstream federate sends to the RTI an LTC (Logical
    /// Tag Complete), or when a transitive upstream federate sends a NET (Next Event Tag) message. It
    /// is also called when an upstream federate resigns from the federation.
    #[tracing::instrument(skip(self, federate_key))]
    async fn send_advance_grant_if_safe(&mut self, federate_key: FederateKey) -> bool {
        // Find the earliest LTC of upstream federates
        let min_upstream_completed = self.federates[federate_key]
            .neighbors
            .upstream
            .iter()
            .map(|&(upstream_fed_key, upstream_delay)| {
                self.federates[upstream_fed_key]
                    .completed
                    .delay(upstream_delay)
            })
            .min()
            .unwrap();

        tracing::debug!(
            "Minimum upstream LTC for {federate_key:?} is {min_upstream_completed} (adjusted by after delay).",
            min_upstream_completed=min_upstream_completed.since(*self.start_time.borrow())
        );

        if min_upstream_completed > self.federates[federate_key].last_granted
            && min_upstream_completed >= self.federates[federate_key].next_event
        {
            // The federate has to advance its tag
            self.send_tag_advance_grant(federate_key, min_upstream_completed)
                .await;
            return true;
        }

        // Can't make progress based only on upstream LTCs.
        // If all (transitive) upstream federates of the federate have earliest event tags such that
        // the federate can now advance its tag, then send it a TAG message. Find the earliest event
        // time of each such upstream federate, adjusted by delays on the connections.

        // Find the tag of the earliest possible incoming message from upstream federates.
        let t_d = self.transitive_upstream_neighbors[federate_key]
            .iter()
            .map(|(key, delay)| self.federates[*key].next_event.delay(*delay))
            //.chain(std::iter::once(self.federates[federate_key].next_event))
            .min()
            .unwrap_or_else(|| Tag::NEVER);

        tracing::debug!(
            "Earliest next event upstream has tag {}.",
            t_d.since(*self.start_time.borrow())
        );

        let Federate {
            next_event,
            last_provisionally_granted,
            last_granted,
            ..
        } = self.federates[federate_key];

        // The federate has something to do.
        if t_d > next_event
            // The grant is not redundant (equal is important to override any previous PTAGs).
            && t_d >= last_provisionally_granted
            // The grant is not redundant.
            && t_d > last_granted
        {
            // All upstream federates have events with a larger tag than fed, so it is safe to send a TAG.
            tracing::debug!(
                "Earliest upstream message time for {federate_key:?} is {t_d} (adjusted by after delay). Granting tag advance for {next_event}",
                t_d=t_d.since(*self.start_time.borrow()),
                next_event=next_event.since(*self.start_time.borrow()),
            );
            self.send_tag_advance_grant(federate_key, next_event).await;
        } else if t_d == next_event && t_d > last_provisionally_granted && t_d > last_granted {
            // Some upstream federate has an event that has the same tag as fed's next event, so we can only provisionally
            // grant a TAG (via a PTAG).
            tracing::debug!(
                "Earliest upstream message time for {federate_key:?} is {t_d} (adjusted by after delay). Granting provisional tag advance.",
                t_d=t_d.since(*self.start_time.borrow()),
            );
            self.send_provisional_tag_advance_grant(federate_key, t_d)
                .await;
        }

        false
    }

    /// Update the next event tag of federate `federate_key`.
    ///
    /// It will update the recorded next event tag of federate `federate_key` to the minimum of
    /// `next_event_tag` and the minimum tag of in-transit messages (if any) to the federate. Will
    /// try to see if the RTI can grant new TAG or PTAG messages to any downstream federates based
    /// on this new next event tag.
    async fn update_federate_next_event_tag(
        &mut self,
        federate_key: FederateKey,
        next_event_tag: Tag,
    ) {
        let min_in_transit_tag = self.federates[federate_key]
            .in_transit_message_tags
            .peek()
            .copied()
            .unwrap_or(Tag::FOREVER);

        let next_event_tag = next_event_tag.min(min_in_transit_tag);

        self.federates[federate_key].next_event = next_event_tag;

        tracing::debug!(
            "RTI: Updated the recorded next event tag for {federate_key:?} to {next_event_tag}",
            next_event_tag = next_event_tag.since(*self.start_time.borrow()),
        );

        // Check to see whether we can reply now with a tag advance grant.
        // If the federate has no upstream federates, then it does not wait for nor expect a reply.
        // It just proceeds to advance time.
        if !self.federates[federate_key].neighbors.upstream.is_empty() {
            self.send_advance_grant_if_safe(federate_key).await;
        }
        // Check downstream federates to see whether they should now be granted a TAG.
        //self.send_downstream_advance_grants_if_safe(federate_key) .await;
    }

    #[tracing::instrument(
        skip(self),
        fields(
            tag=?tag.since(*self.start_time.borrow())
        )
    )]
    async fn handle_port_absent(
        &mut self,
        federate_key: FederateKey,
        dest_federate_key: FederateKey,
        dest_port_key: PortKey,
        tag: Tag,
    ) {
        // If the destination federate is no longer connected, issue a warning and return.

        tracing::debug!("RTI forwarding port absent message.");

        // Forward the message to destination federate.
        self.federates[dest_federate_key]
            .sink
            .send(RtiMsg::PortAbsent(dest_federate_key, dest_port_key, tag))
            .await
            .unwrap();
    }

    #[tracing::instrument(
        skip(self),
        fields(dest_federate=?msg.dest_federate, dest_port=?msg.dest_port, tag=?tag.since(*self.start_time.borrow()))
    )]
    async fn handle_tagged_message(&mut self, federate_key: FederateKey, tag: Tag, msg: Message) {
        let dest_federate = msg.dest_federate;

        tracing::debug!("RTI received `TaggedMessage`. Forwarding.",);

        // Record this in-transit message in federate's in-transit message queue.
        if self.federates[dest_federate].completed < tag {
            self.federates[dest_federate]
                .in_transit_message_tags
                .push(tag);
            tracing::debug!(
                "RTI: Adding a message with tag {tag} to the list of in-transit messages for federate {dest_fed:?}.",
                tag=tag.since(*self.start_time.borrow()),
                dest_fed=dest_federate
            );
        } else {
            tracing::error!(
                "RTI: Federate {dest_fed:?} has already completed tag {completed}, but there is an in-transit message with tag {tag} from this federate. This is going to cause an STP violation under centralized coordination.",
                dest_fed=dest_federate,
                completed=self.federates[dest_federate].completed,
                tag=tag
            );
        }

        // Forward the message to the destination federate.
        self.federates[dest_federate]
            .sink
            .send(RtiMsg::TaggedMessage(tag, msg))
            .await
            .unwrap();

        self.update_federate_next_event_tag(dest_federate, tag)
            .await;
    }

    #[tracing::instrument(
        skip(self),
        fields(federate_key=?federate_key, tag=?tag.since(*self.start_time.borrow()))
    )]
    async fn handle_logical_tag_complete(&mut self, federate_key: FederateKey, tag: Tag) {
        tracing::debug!("RTI received the Logical Tag Complete (LTC).");

        self.federates[federate_key].completed = tag;

        // Remove any recorded in-transit messages with tags <= tag.
        self.federates[federate_key]
            .in_transit_message_tags
            .retain(|&t| t > tag);

        // Check downstream federates to see whether they should now be granted a TAG.
        let downstream = self.federates[federate_key].neighbors.downstream.clone();
        for &downstream_fed_key in downstream.iter() {
            self.send_advance_grant_if_safe(downstream_fed_key).await;
            //self.send_downstream_advance_grants_if_safe()
        }
    }

    #[tracing::instrument(
        skip(self, federate_key),
        fields(tag=?tag.since(*self.start_time.borrow()))
    )]
    async fn handle_next_event_tag(&mut self, federate_key: FederateKey, tag: Tag) {
        tracing::debug!("RTI received `NextEventTag`");
        // Update the next event tag of the federate.
        self.update_federate_next_event_tag(federate_key, tag).await;
    }

    #[tracing::instrument(skip(self, federate_key))]
    async fn handle_timestamp(
        &mut self,
        federate_key: FederateKey,
        ts: Timestamp,
    ) -> Result<(), Error> {
        let fed = self
            .federates
            .get_mut(federate_key)
            .expect("Invalid federate key");

        tracing::debug!("Proposing start time to RTI");

        let mut sink = fed.sink.clone();
        let mut start_time_sync = fed.start_time_sync.clone();

        tokio::spawn(async move {
            let max_start_time = start_time_sync
                .propose_start_time(ts)
                .await
                .expect("TODO: handle error");

            // Send back to the federate the maximum time plus an offset on a TIMESTAMP message.
            let start_time = max_start_time.offset(Duration::from_secs(1));

            sink.send(RtiMsg::Timestamp(start_time))
                .await
                .map_err(|err| Error::Other(err.into()))
        });

        Ok(())
    }

    /// Handle a message received from a federate.
    #[tracing::instrument(skip(self, msg))]
    async fn handle_message(
        &mut self,
        federate_key: FederateKey,
        msg: RtiMsg,
    ) -> Result<(), Error> {
        match msg {
            RtiMsg::Timestamp(ts) => {
                self.handle_timestamp(federate_key, ts).await?;
            }

            RtiMsg::TaggedMessage(tag, msg) => {
                self.handle_tagged_message(federate_key, tag, msg).await;
            }

            RtiMsg::Resign => {
                self.federates[federate_key].sink.close().await.unwrap();

                // Indicate that there will no further events from this federate.
                self.federates[federate_key].next_event = Tag::FOREVER;

                tracing::info!("Federate {federate_key:?} has resigned.");

                // Check downstream federates to see whether they should now be granted a TAG.
                todo!();
            }

            RtiMsg::NextEventTag(tag) => {
                self.handle_next_event_tag(federate_key, tag).await;
            }

            RtiMsg::LogicalTagComplete(tag) => {
                self.handle_logical_tag_complete(federate_key, tag).await;
            }

            RtiMsg::StopRequest(tag) => {
                todo!();
            }

            RtiMsg::StopRequestReply(tag) => {
                todo!();
            }

            RtiMsg::PortAbsent(dest_federate_key, dest_port_key, tag) => {
                self.handle_port_absent(federate_key, dest_federate_key, dest_port_key, tag)
                    .await;
            }

            _ => {
                tracing::warn!("Unhandled message {msg:?}");
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip(self, stream_results))]
    pub async fn run<St>(mut self, mut stream_results: St) -> Result<(), Error>
    where
        St: StreamExt<Item = Result<(FederateKey, RtiMsg), Error>> + FusedStream + Unpin,
    {
        loop {
            tokio::select! {
                res = stream_results.select_next_some() => {
                    let (federate_key, msg) = res?;
                    tracing::trace!("Received message from federate {federate_key:?}: {:?}", msg);
                    self.handle_message(federate_key, msg).await?;
                }
            }
        }
    }
}
