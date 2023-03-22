use tracing::debug;

use crate::{
    Action, ActionKey, ActionMut, Duration, Instant, Level, LevelReactionKey, PortData,
    ReactionKey, ReactionSet, ScheduledEvent, Tag,
};

/// Internal state for a context object
#[derive(Debug, Clone)]
pub(crate) struct ContextInternal {
    /// Remaining reactions triggered at this epoch to execute
    pub(crate) reactions: Vec<(Level, ReactionKey)>,
    /// Events scheduled for a future time
    pub(crate) scheduled_events: Vec<ScheduledEvent>,
}

/// Scheduler context passed into reactor functions.
#[derive(Debug)]
pub struct Context<'a> {
    /// Physical time the Scheduler was started
    pub start_time: Instant,

    /// Logical time of the currently executing epoch
    pub tag: Tag,

    pub current_level: Level,

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
    ) -> Self {
        Self {
            start_time,
            tag,
            current_level: 0,
            internal: ContextInternal {
                reactions: Vec::new(),
                scheduled_events: Vec::new(),
            },
            action_triggers,
        }
    }

    pub fn get_start_time(&self) -> Instant {
        self.start_time
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
    pub fn get_action_mut<'action, T: PortData>(
        &self,
        action: &'action ActionMut<T>,
    ) -> Option<&'action T> {
        action.values.get_value(self.tag)
    }

    pub fn get_action<'action, T: PortData>(
        &self,
        action: &'action Action<T>,
    ) -> Option<&'action T> {
        action.values.get_value(self.tag)
    }

    /// Schedule the Action to trigger at some future time.
    pub fn schedule_action<T: PortData>(
        &mut self,
        action: &mut ActionMut<T>,
        value: Option<T>,
        delay: Option<Duration>,
    ) {
        // TODO
        let tag_delay = delay.map_or(*action.min_delay, |delay| delay + *action.min_delay);
        let new_tag = self.tag.delay(Some(tag_delay));
        action.values.set_value(value, new_tag);
        let downstream = self.action_triggers[action.key].iter().copied();
        self.enqueue_later(downstream, new_tag);
    }

    /// Adds new reactions to execute within this cycle
    pub fn enqueue_now<'b>(&mut self, downstream: impl Iterator<Item = &'b LevelReactionKey>) {
        // Merge all ReactionKeys from `downstream` into the todo reactions
        self.internal.reactions.extend(downstream);
    }

    /// Adds new reactions to execute at a later cycle
    pub fn enqueue_later(&mut self, downstream: impl Iterator<Item = LevelReactionKey>, tag: Tag) {
        let event = ScheduledEvent {
            tag,
            reactions: ReactionSet::from_iter(downstream),
            terminal: false,
        };
        self.internal.scheduled_events.push(event);
    }

    pub fn schedule_shutdown(&mut self, offset: Option<Duration>) {
        debug!("Scheduling shutdown");
        let event = ScheduledEvent {
            tag: self.tag.delay(offset),
            reactions: ReactionSet::default(),
            terminal: true,
        };
        self.internal.scheduled_events.push(event);
    }
}
