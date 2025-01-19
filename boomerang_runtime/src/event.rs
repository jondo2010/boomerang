use std::fmt::{Debug, Display};

use crate::{ActionKey, Duration, LevelReactionKey, ReactionGraph, ReactionSet, ReactorData, Tag};

/// `ScheduledEvent` is used internally by the scheduler loop in the event queue. The dependent reactions are already expanded into a single reaction set.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The set of Reactions to be executed at this tag.
    pub(crate) reactions: ReactionSet,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl Display for ScheduledEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L[tag={},terminal={}]", self.tag, self.terminal)
    }
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

/// `AsyncEvent` is used to inject events into the scheduler from outside of the normal event loop.
pub enum AsyncEvent {
    /// A Logical event should execute at the current logical time (+ an optional delay). The current logical time of
    /// the scheduler is not available to the caller, so the scheduler adds this when pulling the event from the
    /// channel.
    Logical {
        /// The delay that should be applied to this event. This will be added to the current logical time to determine
        /// the tag.
        delay: Duration,
        /// The key of the action that triggered this event.
        key: ActionKey,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// A Physical event has its `tag` set to the current physical time (+ an optional delay).
    Physical {
        /// The [`Tag`] at which the reactions in this event should be executed.
        tag: Tag,
        /// The [`ActionKey`] of the action that triggered this event.
        key: ActionKey,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// The scheduler should terminate after processing this event.
    Shutdown {
        /// The [`Tag`] at which the reactions in this event should be executed.
        tag: Tag,
    },
}

impl Debug for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Logical { delay, key, value } => f
                .debug_struct("Logical")
                .field("delay", delay)
                .field("key", key)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Physical { tag, key, value } => f
                .debug_struct("Physical")
                .field("tag", tag)
                .field("key", key)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Shutdown { tag } => f.debug_struct("Shutdown").field("tag", tag).finish(),
        }
    }
}

impl Display for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncEvent::Logical {
                delay,
                key,
                value: _,
            } => {
                write!(
                    f,
                    "AsyncLogical[delay={delay},key={key:?},value=..]",
                    delay = delay.as_seconds_f64()
                )
            }
            AsyncEvent::Physical { tag, key, value: _ } => {
                write!(
                    f,
                    "AsyncPhysical[tag={tag},key={key:?},value=..]",
                    tag = tag,
                    key = key
                )
            }
            AsyncEvent::Shutdown { tag } => {
                write!(f, "AsyncShutdown[tag={tag}]")
            }
        }
    }
}

impl AsyncEvent {
    /// Create a logical event.
    pub(crate) fn logical(key: ActionKey, delay: Duration, value: Box<dyn ReactorData>) -> Self {
        AsyncEvent::Logical { delay, key, value }
    }

    /// Create a physical event.
    pub(crate) fn physical(key: ActionKey, tag: Tag, value: Box<dyn ReactorData>) -> Self {
        AsyncEvent::Physical { tag, key, value }
    }

    /// Create a shutdown event.
    pub(crate) fn shutdown(tag: Tag) -> Self {
        AsyncEvent::Shutdown { tag }
    }

    /// Get an iterator over the downstream reactions of this event.
    pub fn downstream_reactions<'a>(
        &self,
        reaction_graph: &'a ReactionGraph,
    ) -> impl Iterator<Item = LevelReactionKey> + 'a {
        match self {
            AsyncEvent::Logical { key, .. } => reaction_graph.action_triggers[*key].iter().copied(),
            AsyncEvent::Physical { key, .. } => {
                reaction_graph.action_triggers[*key].iter().copied()
            }
            AsyncEvent::Shutdown { .. } => reaction_graph.shutdown_reactions.iter().copied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    #[test]
    fn test_scheduled_event_order() {
        // ScheduledEvent is used in a BinaryHeap, which by design is a max-heap. This means that our implementation of Ord
        // must be reversed to achieve a min-heap behavior.
        // Additionally, we want to ensure that shutdown events are processed last given multiple events with the same tag.
        let mut heap = BinaryHeap::new();
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: ReactionSet::default(),
            terminal: true,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(0), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });

        // The top event should NOT be the shutdown event
        let ev0 = heap.pop().unwrap();
        assert_eq!(ev0.tag.offset(), Duration::seconds(0));
        assert!(!ev0.terminal);
        let ev1 = heap.pop().unwrap();
        assert!(!ev1.terminal);
        assert_eq!(ev1.tag.offset(), Duration::seconds(1));
        let ev2 = heap.pop().unwrap();
        assert!(ev2.terminal);
        assert_eq!(ev2.tag.offset(), Duration::seconds(1));
    }
}
