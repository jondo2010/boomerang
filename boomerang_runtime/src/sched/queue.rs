use std::collections::BinaryHeap;

use crate::{key_set::KeySet, Level, ReactionSetLimits, Tag};

#[derive(Debug)]
pub(crate) struct EventQueue<K: tinymap::Key, A: Copy> {
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent<K, A>>,
    /// Recycled key sets that avoid steady-state allocations.
    free_reaction_sets: Vec<KeySet<K>>,
    /// Limits for the reaction sets
    reaction_set_limits: ReactionSetLimits,
}

/// Action value retained while a scope-local event may be rebased.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScheduledActionValue<A> {
    /// Typed action identity.
    pub(super) key: A,
    /// Global tag at which the action value is stored.
    pub(super) stored_tag: Tag,
}

/// One queued event parameterized by the schedule's exact key types.
#[derive(Debug, Clone)]
pub(super) struct ScheduledEvent<K: tinymap::Key, A: Copy> {
    /// Event tag, in global or scope-local time as selected by the manager.
    pub(super) tag: Tag,
    /// Reactions activated by this event.
    pub(super) reactions: KeySet<K>,
    /// Whether processing this event terminates the scheduler.
    pub(super) terminal: bool,
    /// Whether this event includes work other than terminal shutdown processing.
    pub(super) has_nonterminal_work: bool,
    /// Optional action value metadata needed for modal rebasing.
    pub(super) action_value: Option<ScheduledActionValue<A>>,
}

impl<K: tinymap::Key, A: Copy> Eq for ScheduledEvent<K, A> {}

impl<K: tinymap::Key, A: Copy> PartialEq for ScheduledEvent<K, A> {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.terminal == other.terminal
    }
}

impl<K: tinymap::Key, A: Copy> PartialOrd for ScheduledEvent<K, A> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: tinymap::Key, A: Copy> Ord for ScheduledEvent<K, A> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag
            .cmp(&other.tag)
            .then(self.terminal.cmp(&other.terminal))
            .reverse()
    }
}

impl<K: tinymap::Key, A: Copy + PartialEq> EventQueue<K, A> {
    pub(crate) fn new(reaction_set_limits: ReactionSetLimits) -> Self {
        Self {
            event_queue: BinaryHeap::new(),
            free_reaction_sets: Vec::new(),
            reaction_set_limits,
        }
    }

    /// Push an event into the event queue
    ///
    /// A free event is pulled from the `free_events` vector and then modified with the provided function.
    pub(crate) fn push_event<I>(&mut self, tag: Tag, reactions: I, terminal: bool)
    where
        I: IntoIterator<Item = (Level, K)>,
    {
        self.push_event_inner(tag, reactions, terminal, None);
    }

    pub(crate) fn push_action_event<I>(
        &mut self,
        tag: Tag,
        action_value: Option<ScheduledActionValue<A>>,
        reactions: I,
        terminal: bool,
    ) where
        I: IntoIterator<Item = (Level, K)>,
    {
        self.push_event_inner(tag, reactions, terminal, action_value);
    }

    fn push_event_inner<I>(
        &mut self,
        tag: Tag,
        reactions: I,
        terminal: bool,
        action_value: Option<ScheduledActionValue<A>>,
    ) where
        I: IntoIterator<Item = (Level, K)>,
    {
        let can_merge = self.event_queue.peek().is_some_and(|event| {
            event.tag == tag && (event.action_value.is_none() || action_value.is_none())
        });

        if can_merge {
            // If the tag is the same as the next event, merge the reactions
            let mut event = self.event_queue.peek_mut().unwrap();
            event.reactions.extend_above(reactions);
            event.terminal = event.terminal || terminal;
            event.has_nonterminal_work |= !terminal;
            if action_value.is_some() {
                event.action_value = action_value;
            }
        } else {
            // Otherwise, push a new event
            let mut reaction_set = self.next_reaction_set();
            reaction_set.extend_above(reactions);
            let event = ScheduledEvent {
                tag,
                reactions: reaction_set,
                terminal,
                has_nonterminal_work: !terminal,
                action_value,
            };
            self.event_queue.push(event);
        }
    }

    /// Pop the next event from the event queue.
    ///
    /// Any subsequent events with the same tag are merged into the returned event.
    pub(crate) fn pop_next_event(
        &mut self,
        action_values: &mut Vec<A>,
    ) -> Option<ScheduledEvent<K, A>> {
        if let Some(mut event) = self.event_queue.pop() {
            if let Some(value) = event.action_value {
                action_values.push(value.key);
            }
            // Merge events with the same tag
            while let Some(next_event) = self.event_queue.peek() {
                if next_event.tag == event.tag {
                    let next_event = self.event_queue.pop().unwrap();
                    if let Some(value) = next_event.action_value {
                        if !action_values.contains(&value.key) {
                            action_values.push(value.key);
                        }
                    }
                    event.reactions.merge(&next_event.reactions);
                    event.terminal = event.terminal || next_event.terminal;
                    event.has_nonterminal_work |= next_event.has_nonterminal_work;

                    self.recycle_reaction_set(next_event.reactions);
                } else {
                    break;
                }
            }

            return Some(event);
        }

        None
    }

    /// Gets a recycled reaction key set or creates one when none are available.
    fn next_reaction_set(&mut self) -> KeySet<K> {
        self.free_reaction_sets
            .pop()
            .unwrap_or_else(|| KeySet::new(&self.reaction_set_limits))
    }

    pub(crate) fn recycle_reaction_set(&mut self, mut reaction_set: KeySet<K>) {
        reaction_set.clear();
        self.free_reaction_sets.push(reaction_set);
    }

    /// Peek the tag of the next event in the queue
    pub(crate) fn peek_tag(&self) -> Option<Tag> {
        self.event_queue.peek().map(|event| event.tag)
    }

    /// Whether any queued event carries work other than terminal shutdown processing.
    pub(crate) fn has_nonterminal_work(&self) -> bool {
        self.event_queue
            .iter()
            .any(|event| event.has_nonterminal_work)
    }

    /// If the event queue still has events on it, report that.
    pub(crate) fn shutdown(&mut self) {
        if !self.event_queue.is_empty() {
            tracing::warn!(
                "---- There are {} unprocessed future events on the event queue.",
                self.event_queue.len()
            );
            let event = self.event_queue.peek().unwrap();
            tracing::warn!(
                "---- The first future event has timestamp {} after start time.",
                event.tag.offset()
            );
        }
    }

    pub(crate) fn clear(&mut self) {
        while let Some(event) = self.event_queue.pop() {
            self.recycle_reaction_set(event.reactions);
        }
    }

    pub(crate) fn rebase_action_values(
        &mut self,
        mut reschedule: impl FnMut(A, Tag, Tag),
        mut map_tag: impl FnMut(Tag) -> Tag,
    ) {
        let mut events = self.event_queue.drain().collect::<Vec<_>>();
        let mut first_move: Option<(A, Tag, Tag)> = None;
        let mut moves: Option<Vec<(A, Tag, Tag)>> = None;
        for event in &mut events {
            let new_tag = map_tag(event.tag);
            if let Some(action_value) = &mut event.action_value {
                let action_move = (action_value.key, action_value.stored_tag, new_tag);
                if let Some(moves) = &mut moves {
                    moves.push(action_move);
                } else if let Some(first_move) = first_move.take() {
                    let collected = vec![first_move, action_move];
                    moves = Some(collected);
                } else {
                    first_move = Some(action_move);
                }
                action_value.stored_tag = new_tag;
            }
        }
        if let Some(mut moves) = moves {
            moves.sort_by(|(_, from_a, _), (_, from_b, _)| from_b.cmp(from_a));
            for (action_key, from, to) in moves {
                reschedule(action_key, from, to);
            }
        } else if let Some((action_key, from, to)) = first_move {
            reschedule(action_key, from, to);
        }
        self.event_queue = events.into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Duration;
    use tinymap::DefaultKey;

    #[test]
    fn scheduled_event_order() {
        // ScheduledEvent is used in a BinaryHeap, which by design is a max-heap. This means
        // that our implementation of Ord must be reversed to achieve min-heap behavior.
        // Additionally, shutdown events must be processed last among events at the same tag.
        let mut heap = BinaryHeap::<ScheduledEvent<DefaultKey, u8>>::new();
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: KeySet::default(),
            terminal: false,
            has_nonterminal_work: true,
            action_value: None,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(1), 0),
            reactions: KeySet::default(),
            terminal: true,
            has_nonterminal_work: false,
            action_value: None,
        });
        heap.push(ScheduledEvent {
            tag: Tag::new(Duration::seconds(0), 0),
            reactions: KeySet::default(),
            terminal: false,
            has_nonterminal_work: true,
            action_value: None,
        });

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
