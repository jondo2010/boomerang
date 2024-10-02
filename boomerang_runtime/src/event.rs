use crate::{ActionKey, LevelReactionKey, ReactionGraph, ReactionSet, Tag};

#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The set of Reactions to be executed at this tag.
    pub(crate) reactions: ReactionSet,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl std::fmt::Display for ScheduledEvent {
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

#[derive(Debug, Clone)]
pub struct PhysicalEvent {
    /// The [`Tag`] at which the reactions in this event should be executed.
    pub(crate) tag: Tag,
    /// The key of the action that triggered this event.
    pub(crate) key: ActionKey,
    /// Whether the scheduler should terminate after processing this event.
    pub(crate) terminal: bool,
}

impl std::fmt::Display for PhysicalEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P[tag={},terminal={}]", self.tag, self.terminal)
    }
}

impl PhysicalEvent {
    /// Create a trigger event.
    pub(crate) fn trigger(key: ActionKey, tag: Tag) -> Self {
        Self {
            tag,
            key,
            terminal: false,
        }
    }

    /// Create a shutdown event.
    pub(crate) fn shutdown(tag: Tag) -> Self {
        Self {
            tag,
            key: ActionKey::default(),
            terminal: true,
        }
    }

    pub fn downstream_reactions<'a>(
        &'a self,
        reaction_graph: &'a ReactionGraph,
    ) -> impl Iterator<Item = LevelReactionKey> + 'a {
        reaction_graph.action_triggers[self.key].iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BinaryHeap, time::Duration};

    #[test]
    fn test_scheduled_event_order() {
        // ScheduledEvent is used in a BinaryHeap, which by design is a max-heap. This means that our implementation of Ord
        // must be reversed to achieve a min-heap behavior.
        // Additionally, we want to ensure that shutdown events are processed last given multiple events with the same tag.
        let mut heap = BinaryHeap::new();
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::from_secs(1), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::from_secs(1), 0),
            reactions: ReactionSet::default(),
            terminal: true,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::from_secs(0), 0),
            reactions: ReactionSet::default(),
            terminal: false,
        });

        // The top event should NOT be the shutdown event
        let ev0 = heap.pop().unwrap();
        assert_eq!(ev0.tag.get_offset(), Duration::from_secs(0).into());
        assert!(!ev0.terminal);
        let ev1 = heap.pop().unwrap();
        assert!(!ev1.terminal);
        assert_eq!(ev1.tag.get_offset(), Duration::from_secs(1).into());
        let ev2 = heap.pop().unwrap();
        assert!(ev2.terminal);
        assert_eq!(ev2.tag.get_offset(), Duration::from_secs(1).into());
    }
}
