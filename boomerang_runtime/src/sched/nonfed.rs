use crossbeam_channel::RecvTimeoutError;
use std::collections::BinaryHeap;

use crate::{Env, ReactionSet, ReactionTriggerCtx, Tag, Timestamp};

use super::ScheduledEvent;

pub use crossbeam_channel::{Receiver, Sender};

/// Scheduler configuration
#[derive(Debug)]
pub struct Config {
    /// Whether to skip wall-clock synchronization
    pub fast_forward: bool,
    /// Whether to keep the scheduler alive for any possible asynchronous events
    pub keep_alive: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fast_forward: true,
            keep_alive: false,
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
    /// Config
    pub(super) config: Config,
}

/// Non-federated scheduler impls
#[cfg(not(feature = "federated"))]
impl Scheduler {
    pub fn new(env: Env, config: Config) -> Self {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        Self {
            env,
            event_tx,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Timestamp::now(),
            shutdown_tag: None,
            config,
        }
    }

    /// Execute startup of the Scheduler.
    #[tracing::instrument(skip(self))]
    pub(crate) fn startup(&mut self) {
        let tag = Tag::new(Duration::ZERO, 0);
        let initial_reaction_set = self.initialize_timers();
        tracing::info!(tag = %tag, ?initial_reaction_set, "Starting the execution.");
        self.process_tag(tag, initial_reaction_set);
    }

    #[tracing::instrument(skip(self))]
    pub fn event_loop(&mut self) {
        self.start_time = Timestamp::now();
        self.startup();

        loop {
            // Push pending events into the queue
            for event in self.event_rx.try_iter() {
                self.event_queue.push(event);
            }

            if let Some(event) = self.event_queue.pop() {
                tracing::debug!(event = %event, reactions = ?event.reactions, "Handling event");

                if !self.config.fast_forward {
                    let target = event.tag.to_logical_time(self.start_time);
                    if let Some(async_event) = self.synchronize_wall_clock(target) {
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
            } else if let Some(event) = self.receive_event() {
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
    fn synchronize_wall_clock(&self, target: Timestamp) -> Option<ScheduledEvent> {
        let now = Timestamp::now();

        if now < target {
            let advance = target - now;
            tracing::debug!(advance = ?advance, "Need to sleep");

            match self.event_rx.recv_timeout(advance.into()) {
                Ok(event) => {
                    tracing::debug!(event = %event, "Sleep interrupted by async event");
                    return Some(event);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let remaining = target.checked_duration_since(Timestamp::now());
                    if let Some(remaining) = remaining {
                        tracing::debug!(remaining = ?remaining,
                            "Sleep interrupted disconnect, sleeping for remaining",
                        );
                        std::thread::sleep(remaining);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }

        if now > target {
            let delay = now - target;
            tracing::warn!(delay = ?delay, "running late");
        }

        None
    }

    /// Try to receive an asynchronous event
    #[tracing::instrument(skip(self))]
    fn receive_event(&mut self) -> Option<ScheduledEvent> {
        if let Some(shutdown) = self.shutdown_tag {
            let abs = shutdown.to_logical_time(self.start_time);
            if let Some(timeout) = abs.checked_duration_since(Timestamp::now()) {
                tracing::debug!(timeout = ?timeout, "Waiting for async event.");
                self.event_rx.recv_timeout(timeout).ok()
            } else {
                tracing::debug!("Cannot wait, already past programmed shutdown time...");
                None
            }
        } else if self.config.keep_alive {
            tracing::debug!("Waiting indefinitely for async event.");
            self.event_rx.recv().ok()
        } else {
            None
        }
    }
}
