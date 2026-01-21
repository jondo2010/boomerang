use crate::{
    event::AsyncEvent, keepalive, ActionCommon, ActionKey, ActionRef, BankInfo, Duration,
    EnclaveKey, ReactionGraph, ReactionKey, ReactorData, Tag,
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
    /// The EnclaveId of this context
    enclave_key: EnclaveKey,
    /// Physical time the Scheduler was started
    pub(crate) start_time: std::time::Instant,
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
    /// Get this Enclave ID
    fn enclave_id(&self) -> EnclaveKey;

    /// Get the current physical time
    fn get_physical_time(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool;

    /// Schedule a shutdown event at some future time.
    fn schedule_shutdown(&mut self, offset: Option<Duration>);

    /// Schedule an event externally by sending it to the scheduler through a channel.
    ///
    /// Returns true if the event was successfully scheduled, false if the channel was disconnected.
    fn schedule_external(&self, event: AsyncEvent) -> bool;

    /// Try to schedule an asynchronous event without blocking
    ///
    /// Returns `Some(true)` if the event was successfully scheduled, `Some(false)` if the channel was disconnected, and `None` if the channel would have blocked.
    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool>;

    /// Schedule a new value for this action asynchronously
    ///
    /// Returns true if the event was successfully scheduled, false if the channel was disconnected.
    #[tracing::instrument(skip(self, action, value, delay), fields(logical = action.is_logical()))]
    fn schedule_action_async<T: ReactorData>(
        &self,
        action: &impl ActionCommon<T>,
        value: T,
        delay: Option<Duration>,
    ) -> bool {
        let tag_delay = action.min_delay() + delay.unwrap_or_default();
        let value = Box::new(value) as Box<dyn ReactorData>;

        let event = if action.is_logical() {
            // Logical actions are scheduled at the current logical time + tag_delay
            //tracing::info!(tag_delay = %tag_delay, key = ?action.key(), "Sched");
            //AsyncEvent::logical(action.key(), tag_delay, value)
            todo!("Logical actions are not supported here");
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            let time = self.get_physical_time() + tag_delay;
            tracing::info!(time = ?time, key = ?action.key(), "Sched");
            AsyncEvent::physical(action.key(), time, value)
        };

        self.schedule_external(event)
    }

    fn release_provisional(&self, enclave: EnclaveKey, tag: Tag) -> bool {
        self.schedule_external(AsyncEvent::provisional(enclave, tag))
    }
}

impl Context {
    pub(crate) fn new(
        enclave_key: EnclaveKey,
        start_time: std::time::Instant,
        bank_info: Option<BankInfo>,
        async_tx: crate::Sender<AsyncEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            enclave_key,
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

    /// Get the physical start time of the scheduler
    pub fn get_start_time(&self) -> std::time::Instant {
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
    pub fn get_logical_time(&self) -> std::time::Instant {
        self.tag.to_logical_time(self.start_time)
    }

    /// Get the logical time elapsed since the start of the program.
    pub fn get_elapsed_logical_time(&self) -> Duration {
        self.tag.offset()
    }

    pub fn get_microstep(&self) -> usize {
        self.tag.microstep()
    }

    /// Create a new SendContext that can be shared across threads.
    /// This is used to schedule asynchronous events.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            enclave_key: self.enclave_key,
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
        let tag_delay = action.min_delay() + delay.unwrap_or_default();

        // Compute the base tag for this scheduling request using the existing
        // semantics, then advance the microstep if there are already entries at
        // the same logical offset.
        let base_tag = if action.is_logical() {
            // Logical actions are scheduled at the current logical time + tag_delay
            self.tag.delay(tag_delay)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            Tag::from_physical_time(self.get_start_time(), self.get_physical_time())
                .delay(tag_delay)
        };

        let new_tag = action.next_tag_for_offset(base_tag);

        // Push the new value into the store
        action.set_value(new_tag, value);

        // Schedule the action to trigger at the new tag
        self.trigger_res
            .scheduled_actions
            .push((action.key(), new_tag));
    }
}

impl CommonContext for Context {
    fn enclave_id(&self) -> EnclaveKey {
        self.enclave_key
    }

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }

    #[tracing::instrument]
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let tag = self.tag.delay(offset.unwrap_or_default());

        self.trigger_res.scheduled_shutdown = self
            .trigger_res
            .scheduled_shutdown
            .map_or(Some(tag), |prev| Some(prev.min(tag)));
    }

    /// Schedule an asynchronous event
    #[tracing::instrument(skip(self), fields(enclave = %self.enclave_id(), event = %event))]
    fn schedule_external(&self, event: AsyncEvent) -> bool {
        if self.shutdown_rx.is_shutdwon() {
            return false;
        }
        self.async_tx.send(event).is_ok()
    }

    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool> {
        if self.is_shutdown() {
            return Some(false);
        }

        self.async_tx.try_send(event).map(|_| true).ok()
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
#[derive(Debug)]
pub struct SendContext {
    /// Enclave ID for this context
    pub(crate) enclave_key: EnclaveKey,
    /// Channel for asynchronous events
    pub(crate) async_tx: crate::Sender<AsyncEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,
}

impl CommonContext for SendContext {
    fn enclave_id(&self) -> EnclaveKey {
        self.enclave_key
    }

    /// Has the scheduler already been shutdown?
    fn is_shutdown(&self) -> bool {
        self.shutdown_rx.is_shutdwon()
    }

    /// Schedule a shutdown event at some future time.
    fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let event = AsyncEvent::shutdown(offset.unwrap_or_default());
        self.async_tx.send(event).unwrap();
    }

    /// Send an external event to the scheduler.
    #[tracing::instrument(skip(self), fields(enclave = %self.enclave_id(), event = %event))]
    fn schedule_external(&self, event: AsyncEvent) -> bool {
        if self.is_shutdown() {
            return false;
        }
        self.async_tx.send(event).is_ok()
    }

    fn try_schedule_async(&self, event: AsyncEvent) -> Option<bool> {
        if self.is_shutdown() {
            return Some(false);
        }

        self.async_tx.try_send(event).map(|_| true).ok()
    }
}

/// Build contexts for each reaction
pub fn build_reaction_contexts(
    enclave_key: EnclaveKey,
    reaction_graph: &ReactionGraph,
    start_time: std::time::Instant,
    event_tx: crate::Sender<AsyncEvent>,
    shutdown_rx: keepalive::Receiver,
) -> tinymap::TinySecondaryMap<ReactionKey, Context> {
    reaction_graph
        .reaction_reactors
        .iter()
        .map(|(reaction_key, reactor_key)| {
            let bank_info = &reaction_graph.reactor_bank_infos[*reactor_key];
            let ctx = Context::new(
                enclave_key,
                start_time,
                bank_info.clone(),
                event_tx.clone(),
                shutdown_rx.clone(),
            );
            (reaction_key, ctx)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{action::Action, event::AsyncEvent, ActionKey, BaseAction, DynActionRefMut};

    #[test]
    fn schedule_action_advances_microsteps_for_same_delay() {
        let (async_tx, _async_rx) = kanal::unbounded::<AsyncEvent>();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();

        let mut ctx = Context::new(
            EnclaveKey::from(0),
            std::time::Instant::now(),
            None,
            async_tx,
            shutdown_rx,
        );

        ctx.reset_for_reaction(Tag::ZERO);

        let mut action = Action::<u32>::new("test", ActionKey::from(0), None, true);
        let mut action_ref = ActionRef::<u32>::try_from(DynActionRefMut(
            &mut action as &mut dyn BaseAction,
        ))
        .expect("action ref");

        ctx.schedule_action(&mut action_ref, 1, None);
        ctx.schedule_action(&mut action_ref, 2, None);

        let tags: Vec<Tag> = ctx
            .trigger_res
            .scheduled_actions
            .iter()
            .map(|(_, tag)| *tag)
            .collect();

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].offset(), tags[1].offset());
        assert_eq!(tags[1].microstep(), tags[0].microstep() + 1);
    }
}
