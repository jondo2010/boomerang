use std::time::Duration;

use crate::{
    event::AsyncEvent, keepalive, ActionCommon, ActionKey, ActionRef, BankInfo, ReactionGraph,
    ReactionKey, ReactorData, Tag, Timestamp,
};

/// Result from a reaction trigger
#[derive(Debug, Clone)]
pub(crate) struct TriggerRes {
    /// Actions that have been scheduled to trigger at a future time
    pub scheduled_actions: Vec<(ActionKey, Tag)>,
    /// A shutdown was scheduled
    pub scheduled_shutdown: Option<Tag>,
}

/// Scheduler context passed into reactor functions.
#[derive(Debug)]
pub struct Context {
    /// Physical time the Scheduler was started
    pub(crate) start_time: Timestamp,
    /// Logical time of the currently executing epoch
    pub(crate) tag: Tag,
    /// Bank index and node count for a multi-bank reactor
    pub(crate) bank_info: Option<BankInfo>,

    /// Channel for asynchronous events
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,

    /// Trigger result
    pub(crate) trigger_res: TriggerRes,
}

/// Common methods for both `Context` and `SendContext`
pub trait CommonContext {
    /// Get the current physical time
    fn get_physical_time(&self) -> Timestamp {
        Timestamp::now()
    }

    fn schedule_shutdown(&mut self, offset: Option<Duration>);

    /// Schedule an asynchronous event
    ///
    /// Returns true if the event was successfully scheduled, false if the channel was disconnected.
    fn schedule_async(&self, event: AsyncEvent) -> bool;

    /// Schedule a new value for this action asynchronously
    fn schedule_action_async<T: ReactorData>(
        &self,
        action: &impl ActionCommon<T>,
        value: T,
        delay: Option<Duration>,
    ) {
        let tag_delay = action.min_delay() + delay.unwrap_or_default();
        let value = Box::new(value) as Box<dyn ReactorData>;

        let event = if action.is_logical() {
            // Logical actions are scheduled at the current logical time + tag_delay
            tracing::info!(tag_delay = ?tag_delay, key = ?action.key(), "Scheduling Async LogicalAction");
            AsyncEvent::logical(action.key(), tag_delay, value)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            let time = self.get_physical_time().offset(tag_delay);
            tracing::info!(time = ?time, key = ?action.key(), "Scheduling Async PhysicalAction");
            AsyncEvent::physical(action.key(), time, value)
        };

        assert!(self.schedule_async(event), "Failed to send async event");
    }
}

impl Context {
    pub(crate) fn new(
        start_time: Timestamp,
        bank_info: Option<BankInfo>,
        async_tx: crate::Sender<AsyncEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            start_time,
            tag: Tag::new(start_time, 0),
            bank_info,
            async_tx,
            shutdown_rx,
            trigger_res: TriggerRes {
                scheduled_actions: Vec::new(),
                scheduled_shutdown: None,
            },
        }
    }

    pub(crate) fn reset_for_reaction(&mut self, tag: Tag) {
        self.tag = tag;
        self.trigger_res.scheduled_actions.clear();
        self.trigger_res.scheduled_shutdown = None;
    }

    /// Get the start time of the scheduler
    pub fn get_start_time(&self) -> Timestamp {
        self.start_time
    }

    /// Get the bank index for a multi-bank reactor
    pub fn get_bank_index(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { idx, .. }| *idx)
    }

    /// Get the number of nodes in a multi-bank reactor
    pub fn get_bank_total(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { total, .. }| *total)
    }

    pub fn get_tag(&self) -> Tag {
        self.tag
    }

    /// Get the current logical time, frozen during the execution of a reaction.
    pub fn get_logical_time(&self) -> Timestamp {
        self.tag.to_logical_time(self.start_time)
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.get_logical_time() - self.get_start_time()
    }

    /// Create a new SendContext that can be shared across threads.
    /// This is used to schedule asynchronous events.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            async_tx: self.async_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }

    /// Get value for an action at the current logical time
    pub fn get_action_value<'a, T: ReactorData>(
        &self,
        action: &'a mut ActionRef<T>,
    ) -> Option<&'a T> {
        action.get_value_at(self.tag)
    }

    /// Schedule a new value for this action
    pub fn schedule_action<T: ReactorData>(
        &mut self,
        action: &mut ActionRef<T>,
        value: T,
        delay: Option<Duration>,
    ) {
        //let action = &mut self.0;

        let tag_delay = action.min_delay() + delay.unwrap_or_default();

        let new_tag = if action.is_logical() {
            // Logical actions are scheduled at the current logical time + tag_delay
            self.tag.delay(tag_delay)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            Tag::from_physical_time(self.get_start_time(), self.get_physical_time())
                .delay(tag_delay)
        };

        // Push the new value into the store
        action.set_value(new_tag, value);

        // Schedule the action to trigger at the new tag
        self.trigger_res
            .scheduled_actions
            .push((action.key(), new_tag));
    }
}

impl CommonContext for Context {
    #[tracing::instrument]
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let tag = self.tag.delay(offset.unwrap_or_default());

        self.trigger_res.scheduled_shutdown = self
            .trigger_res
            .scheduled_shutdown
            .map_or(Some(tag), |prev| Some(prev.min(tag)));
    }

    /// Schedule an asynchronous event
    fn schedule_async(&self, event: AsyncEvent) -> bool {
        self.async_tx.send(event).is_ok()
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
pub struct SendContext {
    /// Channel for asynchronous events
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,
}

impl SendContext {
    /// Has the scheduler already been shutdown?
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }
}

impl CommonContext for SendContext {
    /// Schedule a shutdown event at some future time.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let event = AsyncEvent::shutdown(offset.unwrap_or_default());
        self.async_tx.send(event).unwrap();
    }

    /// Send an asynchronous event to the scheduler.
    fn schedule_async(&self, event: AsyncEvent) -> bool {
        self.async_tx.send(event).is_ok()
    }
}

/// Build contexts for each reaction
pub fn build_reaction_contexts(
    reaction_graph: &ReactionGraph,
    start_time: Timestamp,
    event_tx: crate::Sender<AsyncEvent>,
    shutdown_rx: keepalive::Receiver,
) -> tinymap::TinySecondaryMap<ReactionKey, Context> {
    reaction_graph
        .reaction_reactors
        .iter()
        .map(|(reaction_key, reactor_key)| {
            let bank_info = &reaction_graph.reactor_bank_infos[*reactor_key];
            let ctx = Context::new(
                start_time,
                bank_info.clone(),
                event_tx.clone(),
                shutdown_rx.clone(),
            );
            (reaction_key, ctx)
        })
        .collect()
}
