use crossbeam_channel::{Receiver, RecvTimeoutError};
use derive_more::Display;
// use rayon::iter::{ParallelBridge, ParallelIterator};
use std::{collections::BinaryHeap, time::Duration};
use tracing::{info, trace, warn};

use crate::{Env, Instant, ReactionKey, ReactionSet, ReactionTriggerCtx, Tag};

#[derive(Debug, Display, Clone)]
#[display(fmt = "[tag={},terminal={}]", tag, terminal)]
pub(crate) struct ScheduledEvent {
    pub(crate) tag: Tag,
    pub(crate) reactions: ReactionSet,
    pub(crate) terminal: bool,
}

impl Eq for ScheduledEvent {}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.terminal == other.terminal
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag
            .cmp(&other.tag)
            .then(self.terminal.cmp(&other.terminal))
            .reverse()
    }
}

pub struct Scheduler {
    /// The environment state
    env: Env,
    /// Whether to skip wall-clock synchronization
    fast_forward: bool,
    /// Asynchronous events receiver
    event_rx: Receiver<ScheduledEvent>,

    event_queue: BinaryHeap<ScheduledEvent>,
    /// Initial wall-clock time.
    start_time: Instant,
    /// A shutdown has been scheduled at this time.
    shutdown_tag: Option<Tag>,
}

impl Scheduler {
    pub fn new(env: Env, fast_forward: bool) -> Self {
        let (_event_tx, event_rx) = crossbeam_channel::unbounded();
        Self {
            env,
            fast_forward,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Instant::now(),
            shutdown_tag: None,
        }
    }

    /// Execute startup of the Scheduler.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn startup(&mut self) {
        let tag = Tag::new(Duration::ZERO, 0);

        info!("Starting the execution at {tag}");
        let mut reaction_set = ReactionSet::default();

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to
        // process.
        for (offset, downstream) in self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_startup_events())
        {
            if offset.is_zero() {
                reaction_set.extend_above(downstream.iter().copied(), 0);
            } else {
                let tag = tag.delay(Some(*offset));
                self.event_queue.push(ScheduledEvent {
                    tag,
                    reactions: ReactionSet::from_iter(downstream.iter().copied()),
                    terminal: false,
                });
            }
        }

        self.process_tag(tag, reaction_set);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn cleanup(&mut self, current_tag: Tag) {
        for (period, downstream) in self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_cleanup_events())
        {
            // schedule a periodic timer again
            let tag = current_tag.delay(Some(*period));
            self.event_queue.push(ScheduledEvent {
                tag,
                reactions: ReactionSet::from_iter(downstream.iter().copied()),
                terminal: false,
            });
        }

        for reactor in self.env.reactors.values_mut() {
            reactor.cleanup(current_tag);
        }

        for port in self.env.ports.values_mut() {
            port.cleanup();
        }
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn shutdown(&mut self, shutdown_tag: Tag, _reactions: Option<ReactionSet>) {
        info!("Shutting down at {shutdown_tag}");
        let reaction_set = self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_shutdown_events())
            .flat_map(|downstream_reactions| downstream_reactions.iter().copied())
            .collect();
        self.process_tag(shutdown_tag, reaction_set);
        info!("Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    fn receive_event(&mut self) -> Option<ScheduledEvent> {
        // TODO
        None
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn event_loop(&mut self) {
        self.startup();
        loop {
            // Push pending events into the queue
            for event in self.event_rx.try_iter() {
                self.event_queue.push(event);
            }

            if let Some(event) = self.event_queue.pop() {
                trace!("Handling event {}", event);

                if !self.fast_forward {
                    if let Some(async_event) =
                        self.synchronize_wall_clock(event.tag.to_logical_time(self.start_time))
                    {
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
                trace!("No more events in queue. -> Terminate!");
                break;
            }

            #[cfg(feature = "profiling")]
            profiling::finish_frame!();
        } // loop

        let shutdown_tag = self
            .shutdown_tag
            .unwrap_or_else(|| Tag::now(self.start_time));
        self.shutdown(shutdown_tag, None);
    }

    // Wait until the wall-clock time is reached
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn synchronize_wall_clock(&self, target: Instant) -> Option<ScheduledEvent> {
        let now = Instant::now();

        if now < target {
            let advance = target - now;
            trace!("Need to sleep {}ns", advance.as_nanos());

            match self.event_rx.recv_timeout(advance) {
                Ok(event) => {
                    trace!("Sleep interrupted by async event {}", event.tag);
                    return Some(event);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let remaining = target.duration_since(Instant::now());
                    std::thread::sleep(remaining);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }

        if now > target {
            let delay = now - target;
            warn!("running late by {}ns", delay.as_nanos());
        }

        None
    }

    /// Process the reactions at this tag in increasing order of level.
    /// Reactions at a level N may trigger further reactions at levels M>N
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn process_tag(&mut self, tag: Tag, mut reaction_set: ReactionSet) {
        trace!("Processing tag {tag} with {} levels:", reaction_set.len());

        while let Some((level, reaction_keys)) = reaction_set.next() {
            trace!("  Level {level} with {} Reaction(s)", reaction_keys.len());

            let iter_ctx = self.env.iter_reaction_ctx(reaction_keys.iter());

            let inner_ctxs = iter_ctx
                //.par_bridge()
                .map(|trigger_ctx| {
                    let ReactionTriggerCtx {
                        reaction,
                        reactor,
                        inputs,
                        outputs,
                    } = trigger_ctx;

                    let reaction_name = reaction.get_name();
                    let reactor_name = reactor.get_name();
                    trace!("    Executing {reactor_name}/{reaction_name}.",);

                    let mut ctx = reaction.trigger(self.start_time, tag, reactor, inputs, outputs);

                    // Queue downstream reactions triggered by any ports that were set.
                    for port in outputs.iter() {
                        if port.is_set() {
                            ctx.enqueue_now(port.get_downstream());
                        }
                    }

                    ctx.internal
                })
                .collect::<Vec<_>>();

            for ctx in inner_ctxs.into_iter() {
                reaction_set.extend_above(ctx.reactions.into_iter(), level);

                for evt in ctx.scheduled_events.into_iter() {
                    self.event_queue.push(evt);
                }
            }
        }

        self.cleanup(tag);
    }
}
