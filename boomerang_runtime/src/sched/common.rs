use std::time::Duration;

use crate::{ReactionSet, ReactionTriggerCtx, ScheduledEvent};

pub use super::{Receiver, Scheduler, Sender};

use boomerang_core::time::{Tag, Timestamp};

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("env", &self.env)
            .field("event_queue", &self.event_queue)
            .field("start_time", &self.start_time)
            .field("shutdown_tag", &self.shutdown_tag)
            .field("config", &self.config)
            .finish()
    }
}

impl Scheduler {
    /// For all Timers, pump later events onto the queue and create an initial ReactionSet to
    /// process.
    pub(crate) fn initialize_timers(&mut self) -> ReactionSet {
        self.env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_startup_events())
            .flatten()
            .copied()
            .collect()
    }

    #[tracing::instrument(skip(self))]
    pub(crate) fn cleanup(&mut self, current_tag: Tag) {
        for reactor in self.env.reactors.values_mut() {
            reactor.cleanup(current_tag);
        }

        for port in self.env.ports.values_mut() {
            port.cleanup();
        }
    }

    #[tracing::instrument(skip(self))]
    pub(crate) fn shutdown(&mut self, shutdown_tag: Tag, _reactions: Option<ReactionSet>) {
        tracing::info!(tag = %shutdown_tag, "Shutting down.");
        let reaction_set = self
            .env
            .reactors
            .values()
            .flat_map(|reactor| reactor.iter_shutdown_events())
            .flat_map(|downstream_reactions| downstream_reactions.iter().copied())
            .collect();
        self.process_tag(shutdown_tag, reaction_set);

        // If the event queue still has events on it, report that.
        if !self.event_queue.is_empty() {
            tracing::warn!(
                "---- There are {} unprocessed future events on the event queue.",
                self.event_queue.len()
            );
            let event = self.event_queue.peek().unwrap();
            tracing::warn!(
                "---- The first future event has timestamp {:?} after start time.",
                event.tag.get_offset()
            );
        }

        tracing::info!("---- Elapsed logical time: {:?}", shutdown_tag.get_offset());
        // If physical_start_time is 0, then execution didn't get far enough along to initialize this.
        let physical_elapsed = Timestamp::now().checked_duration_since(self.start_time);
        tracing::info!("---- Elapsed physical time: {:?}", physical_elapsed);

        tracing::info!("Scheduler has been shut down.");
    }

    /// Process the reactions at this tag in increasing order of level.
    /// Reactions at a level `N` may trigger further reactions at levels `M`>`N`.
    ///
    /// If the feature `parallel` is enabled, then reactions within each level are executed in
    /// parallel on the Rayon thread pool.
    #[tracing::instrument(skip(self), fields(tag = %tag, reaction_set = ?reaction_set))]
    pub fn process_tag(&mut self, tag: Tag, mut reaction_set: ReactionSet) {
        while let Some((level, reaction_keys)) = reaction_set.next() {
            tracing::info!("Level{level} with {} Reaction(s)", reaction_keys.len());

            #[cfg(feature = "parallel")]
            use rayon::prelude::{ParallelBridge, ParallelIterator};

            #[cfg(feature = "parallel")]
            let iter_ctx = self
                .env
                .iter_reaction_ctx(reaction_keys.iter())
                .par_bridge();

            #[cfg(not(feature = "parallel"))]
            let iter_ctx = self.env.iter_reaction_ctx(reaction_keys.iter());

            let inner_ctxs = iter_ctx
                .map(|trigger_ctx| {
                    let ReactionTriggerCtx {
                        reaction,
                        reactor,
                        inputs,
                        outputs,
                    } = trigger_ctx;

                    let reaction_name = reaction.get_name();
                    let reactor_name = reactor.get_name();
                    tracing::trace!("    Executing {reactor_name}/{reaction_name}.",);

                    //TODO: Plumb these iterators through into the generated reaction code.
                    let inputs = inputs.collect::<Vec<_>>();
                    let mut outputs = outputs.collect::<Vec<_>>();

                    let mut ctx = reaction.trigger(
                        self.start_time,
                        tag,
                        reactor,
                        inputs.as_slice(),
                        outputs.as_mut_slice(),
                        self.event_tx.clone(),
                        #[cfg(feature = "federated")]
                        &self.client,
                    );

                    // Queue downstream reactions triggered by any ports that were set.
                    for port in outputs.into_iter() {
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

        // Insert network-dependent events for input/output ports into the queue
        // enqueue_network_control_reactions()
        #[cfg(feature = "federated")]
        {
            let top_reactor = &self.env.reactors[self.env.top_reactor];
            let reactions = self
                .federate_env
                .input_control_triggers
                .iter()
                .flat_map(|&action_key| top_reactor.action_triggers[action_key].iter().copied())
                .collect();

            self.event_queue.push(ScheduledEvent {
                tag: tag.delay(None::<Duration>),
                reactions,
                terminal: false,
            });
        }

        self.cleanup(tag);
    }
}
