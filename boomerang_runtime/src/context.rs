use crossbeam_channel::Sender;

use crate::{
    ActionData, ActionKey, ActionRefValue, Duration, Instant, Level, LevelReactionKey,
    PhysicalActionRef, ReactionKey, ReactionSet, ScheduledEvent, Tag,
};

/// Internal state for a context object
#[derive(Debug, Clone)]
pub(crate) struct ContextInternal {
    /// Remaining reactions triggered at this epoch to execute
    pub(crate) reactions: Vec<(Level, ReactionKey)>,
    /// Events scheduled for a future time
    pub(crate) scheduled_events: Vec<ScheduledEvent>,
    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<ScheduledEvent>,
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
        async_tx: Sender<ScheduledEvent>,
    ) -> Self {
        Self {
            start_time,
            tag,
            internal: ContextInternal {
                reactions: Vec::new(),
                scheduled_events: Vec::new(),
                async_tx,
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
            action_triggers: self.action_triggers.clone(),
        }
    }
}

/// SendContext can be shared across threads and allows asynchronous events to be scheduled.
pub struct SendContext {
    /// Physical time the Scheduler was started
    pub start_time: Instant,
    /// Channel for asynchronous events
    pub(crate) async_tx: Sender<ScheduledEvent>,
    /// Downstream reactions triggered by actions
    //TODO: Move this into ActionRef
    action_triggers: tinymap::TinySecondaryMap<ActionKey, Vec<LevelReactionKey>>,
}

impl SendContext {
    /// Schedule a PhysicalAction to trigger at some future time.
    pub fn schedule_action<T: ActionData>(
        &mut self,
        action: &mut PhysicalActionRef<T>,
        value: Option<T>,
        delay: Option<Duration>,
    ) {
        let tag_delay = action.min_delay + delay.unwrap_or_default();
        let new_tag = Tag::absolute(self.start_time, Instant::now() + tag_delay);
        action.set_value(value, new_tag);
        let downstream = self.action_triggers[action.key].iter().copied();
        tracing::info!(action = ?action.key, new_tag = %new_tag, downstream = ?downstream, "Scheduling Physical");
        self.enqueue_async(downstream, new_tag);
    }

    /// Adds new reactions to execute at a later cycle
    #[inline]
    fn enqueue_async(&self, downstream: impl Iterator<Item = (Level, ReactionKey)>, tag: Tag) {
        self.async_tx
            .send(ScheduledEvent {
                tag,
                reactions: ReactionSet::from_iter(downstream),
                terminal: false,
            })
            .unwrap();
    }

    pub fn schedule_shutdown(&self, offset: Option<Duration>) {
        let tag = Tag::absolute(self.start_time, Instant::now() + offset.unwrap_or_default());
        self.async_tx
            .send(ScheduledEvent {
                tag,
                reactions: ReactionSet::default(),
                terminal: true,
            })
            .unwrap();
    }
}
