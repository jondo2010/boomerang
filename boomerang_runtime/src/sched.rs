use crossbeam_channel::{Receiver, RecvTimeoutError};
use derive_more::Display;
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::{collections::BinaryHeap, time::Duration};
use tracing::{info, trace, warn};

use crate::{
    Context, DepInfo, Env, Instant, InternalAction, ReactionKey, ReactionSet, ReactionTriggerCtx,
    Tag, ValuedAction,
};

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
        Some(self.cmp(&other))
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
    env: Env,

    /// Dependency information
    dep_info: DepInfo,

    /// Whether to skip wall-clock synchronization
    fast_forward: bool,

    /// Asynchronous events receiver
    event_rx: Receiver<ScheduledEvent>,

    event_queue: BinaryHeap<ScheduledEvent>,

    /// Initial wall-clock time.
    start_time: Instant,

    shutdown_tag: Option<Tag>,
}

impl Scheduler {
    pub fn new(env: Env, dep_info: DepInfo, fast_forward: bool) -> Self {
        let (_event_tx, event_rx) = crossbeam_channel::unbounded();
        Self {
            env,
            dep_info,
            fast_forward,
            event_rx,
            event_queue: BinaryHeap::new(),
            start_time: Instant::now(),
            shutdown_tag: None,
        }
    }

    /// Execute startup of the Scheduler.
    fn startup(&mut self) {
        trace!("Starting the execution");
        let tag = Tag::new(Duration::ZERO, 0);
        let mut reaction_set = ReactionSet::new();

        // For all Timers, pump later events onto the queue and create an initial ReactionSet to
        // process.
        for action in self.env.actions.values() {
            if let InternalAction::Timer { key, offset, .. } = action {
                let downstream = self.dep_info.triggered_by_action(*key);
                if offset.is_zero() {
                    reaction_set.extend_above(downstream, 0);
                } else {
                    let tag = tag.delay(Some(*offset));
                    self.event_queue.push(ScheduledEvent {
                        tag,
                        reactions: ReactionSet::from_iter(downstream),
                        terminal: false,
                    });
                }
            }
        }

        self.process_tag(tag, reaction_set);
    }

    fn cleanup(&mut self, current_tag: Tag) {
        for action in self.env.actions.values_mut() {
            match action {
                InternalAction::Timer { key, period, .. } if !period.is_zero() => {
                    // schedule a periodic timer again
                    let downstream = self.dep_info.triggered_by_action(*key);
                    let tag = current_tag.delay(Some(*period));
                    self.event_queue.push(ScheduledEvent {
                        tag,
                        reactions: ReactionSet::from_iter(downstream),
                        terminal: false,
                    });
                }
                InternalAction::Valued(ValuedAction { values, .. }) => {
                    // Clear action values at the current tag
                    values.remove(current_tag);
                }
                _ => {}
            }
        }

        for port in self.env.ports.values_mut() {
            port.cleanup();
        }
    }

    fn shutdown(&mut self, shutdown_tag: Tag, _reactions: Option<ReactionSet>) {
        info!("Shutting down at {shutdown_tag}");
        let mut reaction_set = ReactionSet::new();
        for action in self.env.actions.values() {
            if let InternalAction::Shutdown { key, .. } = action {
                let downstream = self.dep_info.triggered_by_action(*key);
                reaction_set.extend_above(downstream, 0);
            }
        }
        self.process_tag(shutdown_tag, reaction_set);
        info!("Scheduler has been shut down.");
    }

    /// Try to receive an asynchronous event
    fn receive_event(&mut self) -> Option<ScheduledEvent> {
        // TODO
        None
    }

    pub fn event_loop(mut self) {
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
        } // loop

        let shutdown_tag = self
            .shutdown_tag
            .unwrap_or_else(|| Tag::now(self.start_time));
        self.shutdown(shutdown_tag, None);
    }

    // Wait until the wall-clock time is reached
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
    pub fn process_tag(&mut self, tag: Tag, mut reaction_set: ReactionSet) {
        let dep_info = &self.dep_info;
        trace!("Processing tag {tag} with {} levels:", reaction_set.len());

        while let Some((level, reaction_keys)) = reaction_set.next() {
            trace!("  Level {level} with {} Reaction(s)", reaction_keys.len());

            let reaction_keys: Box<[ReactionKey]> = reaction_keys.collect();

            let iter_ctx = self.env.iter_reaction_ctx(dep_info, reaction_keys.iter());

            let inner_ctxs = iter_ctx
                .par_bridge()
                .map(|trigger_ctx| {
                    let ReactionTriggerCtx {
                        reaction,
                        reactor,
                        inputs,
                        outputs,
                        actions,
                        schedulable_actions,
                    } = trigger_ctx;

                    trace!("    Executing {}.", reaction.get_name());

                    let mut ctx = Context::new(dep_info, self.start_time, tag);
                    reaction.trigger(
                        &mut ctx,
                        reactor,
                        inputs,
                        outputs,
                        actions,
                        schedulable_actions,
                    );

                    // Queue downstream reactions triggered by any ports that were set.
                    for port in outputs.iter() {
                        if port.is_set() {
                            ctx.enqueue_now(dep_info.triggered_by_port(port.get_key()));
                        }
                    }

                    ctx.internal
                })
                .collect::<Vec<_>>();

            for ctx in inner_ctxs.into_iter() {
                reaction_set.extend_above(ctx.reactions.into_iter(), level);

                for evt in ctx.events.into_iter() {
                    self.event_queue.push(evt);
                }
            }
        }

        self.cleanup(tag);
    }
}

#[cfg(feature = "disabled")]
fn test<'a>(
    writable_actions: &'a mut [&'a mut ValuedAction],
    actions: &'a [&'a InternalAction],
    ctx: &mut Context,
) {
    match actions {
        &[&InternalAction::Valued(ref action)] => {}
        _ => panic!(),
    }
    // let [y]: &[_;1]=std::convert::TryInto::try_into(actions).unwrap();

    let [y]: &mut [_; 1usize] = ::std::convert::TryInto::try_into(writable_actions).unwrap();
    let mut a = crate::ActionMut::<u32>::from(y);
    ctx.schedule_action(&mut a, Some(0), None);
    let v = ctx.get_action(&a);
}
