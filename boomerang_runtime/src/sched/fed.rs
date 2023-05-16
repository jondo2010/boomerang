//! Federated scheduler implementation.

use std::{collections::BinaryHeap, net::SocketAddr, time::Duration};

use tokio::sync::mpsc;

use boomerang_core::time::{Tag, Timestamp};
use boomerang_federated::client::{self};

use crate::{Env, FederateEnv, ReactionSet, SchedError, ScheduledEvent};

pub use mpsc::UnboundedReceiver as Receiver;
pub use mpsc::UnboundedSender as Sender;

const ADVANCE_MESSAGE_INTERVAL: Duration = Duration::from_millis(10);

/// Scheduler configuration
#[derive(Debug)]
pub struct Config {
    /// Whether to skip wall-clock synchronization
    pub fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events
    pub keep_alive: bool,
    /// The address of the RTI
    pub rti_addr: SocketAddr,
    /// The federate client configuration
    pub client_config: client::Config,
}

impl Config {
    pub fn new_federated(rti_addr: SocketAddr, client_config: client::Config) -> Self {
        Self {
            fast_forward: false,
            keep_alive: false,
            rti_addr,
            client_config,
        }
    }
}

pub struct Scheduler {
    /// The environment state
    pub(super) env: Env,
    /// Asynchronous events sender
    pub(super) event_tx: Sender<ScheduledEvent>,
    /// Asynchronous events receiver
    pub(super) event_rx: Receiver<ScheduledEvent>,
    /// The main event queue, sorted by time
    pub(super) event_queue: BinaryHeap<ScheduledEvent>,
    /// Initial wall-clock time.
    pub(super) start_time: Timestamp,
    /// A shutdown has been scheduled at this time.
    pub(super) shutdown_tag: Option<Tag>,
    /// Scheduler config
    pub(super) config: Config,
    /// Client to the federated runtime
    pub(super) client: client::Client,
    /// Federated environment
    pub(super) federate_env: FederateEnv,
}

impl Scheduler {
    pub async fn new(
        env: Env,
        federate_env: crate::FederateEnv,
        config: Config,
    ) -> Result<Self, SchedError> {
        let client = client::connect_to_rti(config.rti_addr, &config.client_config).await?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Ok(Self {
            env,
            event_tx,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Timestamp::now(),
            shutdown_tag: None,
            config,
            client,
            federate_env,
        })
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    pub async fn startup(&mut self) {
        self.start_time = self.client.wait_for_start_time().await.unwrap();

        match self.start_time.checked_duration_since(Timestamp::now()) {
            Some(duration) => {
                tracing::info!("Sleeping for {duration:?} to synchronize startup.");
                async_timer::new_timer(duration).await;
            }
            None => {
                tracing::error!("Negotiated start time should be in the future!");
            }
        }

        let tag = Tag::new(Duration::ZERO, 0);
        let initial_reaction_set = self.initialize_timers();
        tracing::info!(tag = %tag, ?initial_reaction_set, "Starting the execution.");
        self.process_tag(tag, initial_reaction_set);
    }

    #[tracing::instrument(skip(self))]
    pub async fn event_loop(&mut self) {
        self.startup().await;

        loop {
            // Push pending events into the queue
            while let Ok(event) = self.event_rx.try_recv() {
                self.event_queue.push(event);
            }

            let granted_tag = self.send_next_event_tag(true).await.unwrap();
            tracing::debug!(
                "Granted next event tag {}",
                granted_tag.since(self.start_time),
            );

            if let Some(event) = self.event_queue.pop() {
                tracing::debug!(event = %event, reactions = ?event.reactions, "Handling event");

                if !self.config.fast_forward {
                    let target = event.tag.to_logical_time(self.start_time);
                    if let Some(async_event) = self.synchronize_wall_clock(target).await {
                        // Woken up by async event
                        if async_event.tag < event.tag {
                            // Re-insert both events to order them
                            self.event_queue.push(event);
                            self.event_queue.push(async_event);
                            continue;
                        } else {
                            self.event_queue.push(async_event);
                        }
                    }
                }

                if self
                    .shutdown_tag
                    .as_ref()
                    .map(|shutdown_tag| shutdown_tag == &event.tag)
                    .unwrap_or(event.terminal)
                {
                    self.shutdown_tag = Some(event.tag);
                    break;
                }
                self.process_tag(event.tag, event.reactions);
            } else if let Some(event) = self.receive_event().await {
                self.event_queue.push(event);
            } else {
                tracing::trace!("No more events in queue. -> Terminate!");
                break;
            }
        } // loop

        let shutdown_tag = self
            .shutdown_tag
            .unwrap_or_else(|| Tag::now(self.start_time));
        self.shutdown(shutdown_tag, None);
    }

    // Wait until the wall-clock time is reached
    #[tracing::instrument(skip(self), fields(target = ?target))]
    async fn synchronize_wall_clock(&self, target: Timestamp) -> Option<ScheduledEvent> {
        todo!()
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    async fn receive_event(&mut self) -> Option<ScheduledEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(Timestamp::now()) {
                tracing::debug!(timeout = ?timeout, "Waiting for async event.");
                tokio::time::timeout(timeout, self.event_rx.recv())
                    .await
                    .unwrap_or_default()
            } else {
                tracing::debug!("Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.config.keep_alive {
            tracing::debug!("Waiting indefinitely for async event.");
            self.event_rx.recv().await
        } else {
            None
        }
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
    #[tracing::instrument(skip(self), fields(tag))]
    pub(crate) async fn send_next_event_tag(
        &mut self,
        wait_for_reply: bool,
    ) -> Result<Tag, SchedError> {
        loop {
            let tag = self
                .event_queue
                .peek()
                .map(|event| event.tag)
                .expect("Empty event queue");

            if !self.client.has_upstream() && !self.client.has_downstream() {
                // No upstream or downstream federates, so no need to send a NET
                tracing::debug!(
                "Granted tag {tag} because the federate has neither upstream nor downstream federates."
            );
                return Ok(tag);
            }

            // If time advance (TAG or PTAG) has already been granted for this tag or a larger tag,
            // then return immediately.
            let last_tag = self.client.last_tag();
            if last_tag >= tag {
                // The requested tag is less than or equal to the last tag, so no need to send a NET
                tracing::debug!(
                "Granted tag {tag} because the requested tag is less than or equal to the last tag.",
                tag=last_tag.since(self.start_time)
            );
                return Ok(last_tag);
            }

            //TODO: tag_bounded_by_physical_time
            let tag_bounded_by_physical_time = false;

            // What we do next depends on whether the NET has been bounded by physical time or by an
            // event on the event queue.
            if !tag_bounded_by_physical_time {
                self.client.send_next_event_tag(tag)?;

                if !wait_for_reply {
                    tracing::debug!("Not waiting for reply to NET.");
                    return Ok(tag);
                }

                // If there are no upstream federates, return immediately, without waiting for a reply.
                // This federate does not need to wait for any other federate.
                // NOTE: If fast execution is being used, it may be necessary to throttle upstream federates.
                if !self.client.has_upstream() {
                    tracing::debug!(
                        "Not waiting for reply to NET {} because I have no upstream federates.",
                        tag.since(self.start_time)
                    );
                    return Ok(tag);
                }

                // Wait until there is a TAG received from the RTI or an event on the event queue.
                tokio::select! {
                    last_tag_changed = self.client.last_tag.changed() => {
                        last_tag_changed.unwrap();
                        tracing::debug!("Received TAG while waiting for reply to NET.");
                        return Ok(*self.client.last_tag.borrow());
                    }
                    event = self.event_rx.recv() => {
                        tracing::debug!("Received event while waiting for reply to NET.");
                        todo!();
                        //let event = event.ok_or(ClientError::UnexpectedClose)?;
                        // Check whether the new event on the event queue requires sending a new NET.
                    }
                    //event = self.receive_event() => {
                    //    //tracing::debug!("Received event while waiting for reply to NET.");
                    //    //let event = event.ok_or(ClientError::UnexpectedClose)?;
                    //    // Check whether the new event on the event queue requires sending a new NET.
                    //}
                }
            }

            if tag != Tag::FOREVER {
                // Create a dummy event that will force this federate to advance time and subsequently
                // enable progress for downstream federates.
                self.push_dummy_events(tag.offset, 1);
            }

            if !wait_for_reply {
                tracing::debug!("Not waiting physical time to advance further.");
                return Ok(tag);
            }

            // This federate should repeatedly advance its tag to ensure downstream federates can make
            // progress. Before advancing to the next tag, we need to wait some time so that we don't
            // overwhelm the network and the RTI. That amount of time will be no greater than
            // ADVANCE_MESSAGE_INTERVAL in the future.
            tracing::debug!("Waiting for physical time to elapse or an event on the event queue.");

            let wait_until_time = /*_lf_last_reported_unadjusted_physical_time_ns +*/
            ADVANCE_MESSAGE_INTERVAL
            // Regardless of the ADVANCE_MESSAGE_INTERVAL, do not let this wait exceed the time of the next tag.
            .min(tag.get_offset());

            tracing::debug!("Wait finished or interrupted.");

            // Either the timeout expired or the wait was interrupted by an event being
            // put onto the event queue. In either case, we can just loop around.
            // The next iteration will determine whether another
            // NET should be sent or not.

            //tag = get_next_event_tag();
        }
    }

    /// Push `count` super-dense dummy events at `time` as spacers into the event queue.
    pub(crate) fn push_dummy_events(&mut self, time: Timestamp, count: usize) {
        tracing::debug!(
            "Inserted {count} dummy event(s) for tag {:?}",
            time.checked_duration_since(self.start_time),
        );
        let dummy_events = (0..count).map(move |i| ScheduledEvent {
            tag: Tag::new(time, i),
            reactions: ReactionSet::new(),
            terminal: false,
        });
        self.event_queue.extend(dummy_events);
    }
}
