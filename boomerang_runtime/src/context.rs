use crossbeam_channel::Sender;

use crate::{
    event::PhysicalEvent, keepalive, ActionData, ActionKey, ActionRefValue, BankInfo, Duration,
    Instant, PhysicalActionRef, ReactionGraph, ReactionKey, Tag,
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
    pub(crate) start_time: Instant,
    /// Logical time of the currently executing epoch
    pub(crate) tag: Tag,
    /// Bank index and node count for a multi-bank reactor
    pub(crate) bank_info: Option<BankInfo>,

    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<PhysicalEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,

    /// Trigger result
    pub(crate) trigger_res: TriggerRes,
}

impl Context {
    pub(crate) fn new(
        start_time: Instant,
        bank_info: Option<BankInfo>,
        async_tx: Sender<PhysicalEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            start_time,
            tag: Tag::now(start_time),
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

    /// Get the bank index for a multi-bank reactor
    pub fn get_bank_index(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { idx, .. }| *idx)
    }

    /// Get the number of nodes in a multi-bank reactor
    pub fn get_bank_total(&self) -> Option<usize> {
        self.bank_info.as_ref().map(|BankInfo { total, .. }| *total)
    }

    pub fn get_start_time(&self) -> Instant {
        self.start_time
    }

    pub fn get_tag(&self) -> Tag {
        self.tag
    }

    /// Get the current logical time, frozen during the execution of a reaction.
    pub fn get_logical_time(&self) -> Instant {
        self.tag.to_logical_time(self.start_time)
    }

    /// Get the current physical time
    pub fn get_physical_time(&self) -> Instant {
        Instant::now()
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.get_logical_time() - self.get_start_time()
    }

    /// Get the physical time elapsed since the start of the program.
    pub fn get_elapsed_physical_time(&self) -> Duration {
        self.get_physical_time() - self.get_start_time()
    }

    /// Get the value of an Action at the current Tag
    pub fn get_action<T: ActionData, A: ActionRefValue<T>>(&self, action: &mut A) -> Option<T> {
        action.get_value(self.tag)
    }

    /// Schedule the Action to trigger at some future time.
    #[tracing::instrument(skip(self, value), fields(action = ?action.get_key()))]
    pub fn schedule_action<T: ActionData, A: ActionRefValue<T>>(
        &mut self,
        action: &mut A,
        value: Option<T>,
        delay: Option<Duration>,
    ) {
        let tag_delay = action.get_min_delay() + delay.unwrap_or_default();
        let new_tag = self.tag.delay(Some(tag_delay));
        tracing::trace!(new_tag = %new_tag, "Scheduling Logical");
        action.set_value(value, new_tag);
        self.trigger_res
            .scheduled_actions
            .push((action.get_key(), new_tag));
    }

    #[tracing::instrument]
    pub fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let tag = self.tag.delay(offset);

        self.trigger_res.scheduled_shutdown = self
            .trigger_res
            .scheduled_shutdown
            .map_or(Some(tag), |prev| Some(prev.min(tag)));
    }

    /// Create a new SendContext that can be shared across threads.
    /// This is used to schedule asynchronous events.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            start_time: self.start_time,
            async_tx: self.async_tx.clone(),
            shutdown_rx: self.shutdown_rx.clone(),
        }
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
pub struct SendContext {
    /// Physical time the Scheduler was started
    pub start_time: Instant,
    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<PhysicalEvent>,
    /// Shutdown channel
    shutdown_rx: keepalive::Receiver,
}

impl SendContext {
    /// Schedule a PhysicalAction to trigger at some future time.
    #[tracing::instrument(skip(self, action, value, delay))]
    pub fn schedule_action<T: ActionData>(
        &mut self,
        action: &mut PhysicalActionRef<T>,
        value: Option<T>,
        delay: Option<Duration>,
    ) {
        let tag_delay = action.min_delay + delay.unwrap_or_default();
        let new_tag = Tag::absolute(self.start_time, Instant::now() + tag_delay);
        action.set_value(value, new_tag);
        tracing::info!(new_tag = %new_tag, key = ?action.key, "Scheduling Physical");
        let event = PhysicalEvent::trigger(action.key, new_tag);
        self.async_tx.send(event).unwrap();
    }

    /// Schedule a shutdown event at some future time.
    pub fn schedule_shutdown(&self, offset: Option<Duration>) {
        let tag = Tag::absolute(self.start_time, Instant::now() + offset.unwrap_or_default());
        let event = PhysicalEvent::shutdown(tag);
        self.async_tx.send(event).unwrap();
    }

    /// Has the scheduler already been shutdown?
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }
}

/// Build contexts for each reaction
pub fn build_reaction_contexts(
    reaction_graph: &ReactionGraph,
    start_time: Instant,
    event_tx: crossbeam_channel::Sender<PhysicalEvent>,
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
