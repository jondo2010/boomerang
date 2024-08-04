use crossbeam_channel::Sender;

use crate::{
    keepalive, ActionData, ActionKey, ActionRefValue, Duration, Instant, Level, LevelReactionKey,
    PhysicalActionRef, PhysicalEvent, ReactionKey, ReactionSet, ScheduledEvent, Tag,
};

/// Internal state for a context object
#[derive(Debug, Clone)]
pub(crate) struct ContextInternal {
    /// Remaining reactions triggered at this epoch to execute
    pub(crate) reactions: Vec<(Level, ReactionKey)>,
    /// Events scheduled for a future time
    pub(crate) scheduled_events: Vec<ScheduledEvent>,
    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<PhysicalEvent>,
    /// Shutdown channel
    pub(crate) shutdown_rx: keepalive::Receiver,
}

/// Scheduler context passed into reactor functions.
#[derive(Debug)]
pub struct Context<'a> {
    /// Physical time the Scheduler was started
    pub(crate) start_time: Instant,
    /// Logical time of the currently executing epoch
    pub(crate) tag: Tag,
    /// Internal state
    pub(crate) internal: ContextInternal,
    /// Downstream reactions triggered by actions
    action_triggers: &'a tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl<'a> Context<'a> {
    pub(crate) fn new(
        start_time: Instant,
        tag: Tag,
        action_triggers: &'a tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
        async_tx: Sender<PhysicalEvent>,
        shutdown_rx: keepalive::Receiver,
    ) -> Self {
        Self {
            start_time,
            tag,
            internal: ContextInternal {
                reactions: Vec::new(),
                scheduled_events: Vec::new(),
                async_tx,
                shutdown_rx,
            },
            action_triggers,
        }
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
    pub fn get_action<T: ActionData, A: ActionRefValue<T>>(&self, action: &A) -> Option<T> {
        action.get_value(self.tag)
    }

    /// Schedule the Action to trigger at some future time.
    pub fn schedule_action<T: ActionData, A: ActionRefValue<T>>(
        &mut self,
        action: &mut A,
        value: Option<T>,
        delay: Option<Duration>,
    ) {
        let tag_delay = action.get_min_delay() + delay.unwrap_or_default();
        let new_tag = self.tag.delay(Some(tag_delay));
        tracing::info!(action = ?action.get_key(), new_tag = %new_tag, "Scheduling Logical");
        action.set_value(value, new_tag);
        let downstream = self.action_triggers[action.get_key()].iter().copied();
        self.enqueue_later(downstream, new_tag);
    }

    /// Adds new reactions to execute within this cycle
    pub fn enqueue_now<'b>(&mut self, downstream: impl Iterator<Item = &'b LevelReactionKey>) {
        // Merge all ReactionKeys from `downstream` into the todo reactions
        self.internal.reactions.extend(downstream);
    }

    /// Adds new reactions to execute at a later cycle
    pub fn enqueue_later(
        &mut self,
        downstream: impl Iterator<Item = (Level, ReactionKey)>,
        tag: Tag,
    ) {
        let event = ScheduledEvent {
            tag,
            reactions: ReactionSet::from_iter(downstream),
            terminal: false,
        };
        self.internal.scheduled_events.push(event);
    }

    #[tracing::instrument]
    pub fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        let event = ScheduledEvent {
            tag: self.tag.delay(offset),
            reactions: ReactionSet::default(),
            terminal: true,
        };
        self.internal.scheduled_events.push(event);
    }

    /// Create a new SendContext that can be shared across threads.
    /// This is used to schedule asynchronous events.
    pub fn make_send_context(&self) -> SendContext {
        SendContext {
            start_time: self.start_time,
            async_tx: self.internal.async_tx.clone(),
            shutdown_rx: self.internal.shutdown_rx.clone(),
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
