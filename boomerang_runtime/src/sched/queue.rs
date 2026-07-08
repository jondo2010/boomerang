use std::{collections::BinaryHeap, pin::Pin};

use crate::{
    event::{ScheduledActionValue, ScheduledEvent},
    store::Store,
    Level, ReactionKey, ReactionSet, ReactionSetLimits, Tag,
};

#[derive(Debug)]
pub(crate) struct EventQueue {
    /// Current event queue
    event_queue: BinaryHeap<ScheduledEvent>,
    /// Recycled ReactionSets to avoid allocations
    free_reaction_sets: Vec<ReactionSet>,
    /// Limits for the reaction sets
    reaction_set_limits: ReactionSetLimits,
}

impl EventQueue {
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
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.push_event_inner(tag, reactions, terminal, None);
    }

    pub(crate) fn push_action_event<I>(
        &mut self,
        tag: Tag,
        action_value: Option<ScheduledActionValue>,
        reactions: I,
        terminal: bool,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        self.push_event_inner(tag, reactions, terminal, action_value);
    }

    fn push_event_inner<I>(
        &mut self,
        tag: Tag,
        reactions: I,
        terminal: bool,
        action_value: Option<ScheduledActionValue>,
    ) where
        I: IntoIterator<Item = (Level, ReactionKey)>,
    {
        let can_merge = self.event_queue.peek().is_some_and(|event| {
            event.tag == tag && (event.action_value.is_none() || action_value.is_none())
        });

        if can_merge {
            // If the tag is the same as the next event, merge the reactions
            let mut event = self.event_queue.peek_mut().unwrap();
            event.reactions.extend_above(reactions);
            event.terminal = event.terminal || terminal;
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
                action_value,
            };
            self.event_queue.push(event);
        }
    }

    /// Pop the next event from the event queue.
    ///
    /// Any subsequent events with the same tag are merged into the returned event.
    pub(crate) fn pop_next_event(&mut self) -> Option<ScheduledEvent> {
        if let Some(mut event) = self.event_queue.pop() {
            // Merge events with the same tag
            while let Some(next_event) = self.event_queue.peek() {
                if next_event.tag == event.tag {
                    let next_event = self.event_queue.pop().unwrap();
                    event.reactions.merge(&next_event.reactions);
                    event.terminal = event.terminal || next_event.terminal;

                    self.recycle_reaction_set(next_event.reactions);
                } else {
                    break;
                }
            }

            return Some(event);
        }

        None
    }

    /// Get a free [`ReactionSet`] or create a new one if none are available.
    fn next_reaction_set(&mut self) -> ReactionSet {
        self.free_reaction_sets
            .pop()
            .unwrap_or_else(|| ReactionSet::new(&self.reaction_set_limits))
    }

    pub(crate) fn recycle_reaction_set(&mut self, mut reaction_set: ReactionSet) {
        reaction_set.clear();
        self.free_reaction_sets.push(reaction_set);
    }

    /// Peek the tag of the next event in the queue
    pub(crate) fn peek_tag(&self) -> Option<Tag> {
        self.event_queue.peek().map(|event| event.tag)
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
        store: &mut Pin<Box<Store>>,
        mut map_tag: impl FnMut(Tag) -> Tag,
    ) {
        let mut events = self.event_queue.drain().collect::<Vec<_>>();
        let mut first_move: Option<(crate::ActionKey, Tag, Tag)> = None;
        let mut moves: Option<Vec<(crate::ActionKey, Tag, Tag)>> = None;
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
                store.reschedule_action_value(action_key, from, to);
            }
        } else if let Some((action_key, from, to)) = first_move {
            store.reschedule_action_value(action_key, from, to);
        }
        self.event_queue = events.into_iter().collect();
    }
}
