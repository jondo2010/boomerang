use crossbeam_channel::Sender;

use crate::{
    event::AsyncEvent, keepalive, ActionKey, BankInfo, Duration, ReactionGraph, ReactionKey, Tag,
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
    pub(crate) start_time: std::time::Instant,
    /// Logical time of the currently executing epoch
    pub(crate) tag: Tag,
    /// Bank index and node count for a multi-bank reactor
    pub(crate) bank_info: Option<BankInfo>,

    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,

    /// Trigger result
    pub(crate) trigger_res: TriggerRes,
}

pub trait ContextCommon {
    /// Get the start time of the scheduler
    fn get_start_time(&self) -> std::time::Instant;

    /// Get the current physical time
    fn get_physical_time(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn schedule_shutdown(&mut self, offset: Option<Duration>);
}

impl Context {
    pub(crate) fn new(
        start_time: std::time::Instant,
        bank_info: Option<BankInfo>,
        async_tx: Sender<AsyncEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            start_time,
            tag: Tag::NEVER,
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

    pub fn get_tag(&self) -> Tag {
        self.tag
    }

    /// Get the current logical time, frozen during the execution of a reaction.
    pub fn get_logical_time(&self) -> std::time::Instant {
        self.tag.to_logical_time(self.start_time)
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.tag.offset()
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

impl ContextCommon for Context {
    fn get_start_time(&self) -> std::time::Instant {
        self.start_time
    }

    #[tracing::instrument]
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let tag = self.tag.delay(offset.unwrap_or_default());

        self.trigger_res.scheduled_shutdown = self
            .trigger_res
            .scheduled_shutdown
            .map_or(Some(tag), |prev| Some(prev.min(tag)));
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
pub struct SendContext {
    /// Physical time the Scheduler was started
    pub start_time: std::time::Instant,
    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<AsyncEvent>,
    /// Shutdown channel
    shutdown_rx: keepalive::Receiver,
}

impl SendContext {
    /// Has the scheduler already been shutdown?
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }
}

impl ContextCommon for SendContext {
    fn get_start_time(&self) -> std::time::Instant {
        self.start_time
    }

    /// Schedule a shutdown event at some future time.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let tag = Tag::from_physical_time(self.start_time, std::time::Instant::now())
            .delay(offset.unwrap_or_default());
        let event = AsyncEvent::shutdown(tag);
        self.async_tx.send(event).unwrap();
    }
}

/// Build contexts for each reaction
pub fn build_reaction_contexts(
    reaction_graph: &ReactionGraph,
    start_time: std::time::Instant,
    event_tx: crossbeam_channel::Sender<AsyncEvent>,
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
