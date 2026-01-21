//! This module provides an implementation of an `ActionStore` for managing actions in a reactor
//! system.
//!
//! The [`ActionStore`] is a data structure that efficiently stores and retrieves actions based on
//! their associated [`Tag`]s. It uses a binary heap internally to maintain the actions
//! in a priority queue, ensuring that actions can be processed in the correct order.
//!
//! Key features:
//! - Out-of-order insertion and update. Pushing a new value for a tag will semantically replace the
//!   old value.
//! - Retrieval follows the monotonically increasing tag order of the scheduler.
//! - Requests for the same current tag will return the same value.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::Mutex;
use std::sync::Arc;

use downcast_rs::Downcast;

use crate::{Duration, ReactorData, Tag};

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct OffsetBucket<T: ReactorData> {
    base_microstep: usize,
    actions: VecDeque<Option<T>>,
    next_microstep: usize,
    occupied: usize,
}

impl<T: ReactorData> OffsetBucket<T> {
    fn new() -> Self {
        Self {
            base_microstep: 0,
            actions: VecDeque::new(),
            next_microstep: 0,
            occupied: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.occupied == 0
    }

    fn insert(&mut self, microstep: usize, data: T) {
        if microstep < self.base_microstep {
            let missing = self.base_microstep - microstep;
            for _ in 0..missing {
                self.actions.push_front(None);
            }
            self.base_microstep = microstep;
        }

        let index = microstep - self.base_microstep;
        while index >= self.actions.len() {
            self.actions.push_back(None);
        }

        let slot = &mut self.actions[index];
        if slot.is_none() {
            self.occupied += 1;
        }
        *slot = Some(data);
        self.next_microstep = self.next_microstep.max(microstep.saturating_add(1));
    }

    fn get(&self, microstep: usize) -> Option<&T> {
        if microstep < self.base_microstep {
            return None;
        }

        let index = microstep - self.base_microstep;
        self.actions.get(index).and_then(|value| value.as_ref())
    }

    fn remove_before(&mut self, microstep: usize) {
        if microstep <= self.base_microstep {
            return;
        }

        let drop_count = (microstep - self.base_microstep).min(self.actions.len());
        for _ in 0..drop_count {
            if let Some(entry) = self.actions.pop_front() {
                if entry.is_some() {
                    self.occupied = self.occupied.saturating_sub(1);
                }
            }
        }
        self.base_microstep += drop_count;
    }
}

pub trait BaseActionStore: Debug + Downcast + Send + Sync {
    /// Remove any value at the given Tag
    fn clear_older_than(&mut self, tag: Tag);

    /// Convert a Boxed store into an Arc-Mutex-protected version
    fn boxed_to_mutex(self: Box<Self>) -> Arc<Mutex<dyn BaseActionStore>>;
}

downcast_rs::impl_downcast!(BaseActionStore);

pub struct ActionStore<T: ReactorData> {
    offsets: BTreeMap<Duration, OffsetBucket<T>>,
}

impl<T: ReactorData> Debug for ActionStore<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionStore").finish()
    }
}

impl<T: ReactorData> ActionStore<T> {
    pub fn new() -> Self {
        ActionStore {
            offsets: BTreeMap::new(),
        }
    }

    /// Add a new action to the store.
    #[inline]
    pub fn push(&mut self, tag: Tag, data: T) {
        let bucket = self
            .offsets
            .entry(tag.offset())
            .or_insert_with(OffsetBucket::new);
        bucket.insert(tag.microstep(), data);
    }

    /// Compute the next microstep for a given logical offset, ensuring it is at
    /// least `min_microstep` and greater than any existing entry at the same
    /// offset.
    #[inline]
    pub fn next_microstep_for_offset(&self, offset: Duration, min_microstep: usize) -> usize {
        self.offsets
            .get(&offset)
            .map(|bucket| bucket.next_microstep)
            .unwrap_or(min_microstep)
            .max(min_microstep)
    }

    pub fn clear_older_than(&mut self, clear_tag: Tag) {
        let clear_offset = clear_tag.offset();
        self.offsets = self.offsets.split_off(&clear_offset);

        if let Some(bucket) = self.offsets.get_mut(&clear_offset) {
            bucket.remove_before(clear_tag.microstep());
            if bucket.is_empty() {
                self.offsets.remove(&clear_offset);
            }
        }
    }

    /// Get the current action data for a given tag.
    ///
    /// This method pops all entries older than `tag` from the store.
    ///
    /// If the store is empty, or only entries newer than `tag` this method returns `None`.
    pub fn get_current(&mut self, tag: Tag) -> Option<&T> {
        // Remove entries older than the given tag
        self.clear_older_than(tag);

        // Return Some only if the top entry's tag matches the given tag
        self.offsets
            .get(&tag.offset())
            .and_then(|bucket| bucket.get(tag.microstep()))
    }
}

impl<T: ReactorData> Default for ActionStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ReactorData> BaseActionStore for ActionStore<T> {
    fn clear_older_than(&mut self, tag: Tag) {
        self.clear_older_than(tag)
    }

    fn boxed_to_mutex(self: Box<Self>) -> Arc<Mutex<dyn BaseActionStore>> {
        Arc::new(Mutex::new(*self)) as _
    }
}

#[cfg(test)]
mod tests {
    use crate::Duration;

    use super::*;

    fn build_tags<const N: usize>() -> [Tag; N] {
        (0..N)
            .map(|i| Tag::new(Duration::seconds(i as _), 0))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    #[test]
    fn test_out_of_order_get_current_prunes() {
        let mut store = ActionStore::<u32>::new();

        let tags = build_tags::<4>();
        store.push(tags[2], 20);
        store.push(tags[0], 0);
        store.push(tags[1], 10);

        assert_eq!(store.get_current(tags[0]), Some(&0));
        assert_eq!(store.get_current(tags[1]), Some(&10));
        assert_eq!(store.get_current(tags[2]), Some(&20));
        assert_eq!(store.get_current(tags[1]), None);
        assert_eq!(store.get_current(tags[3]), None);
    }

    #[test]
    fn test_replace_same_tag() {
        let mut store = ActionStore::<u32>::new();
        let tag = Tag::new(Duration::seconds(1), 0);

        store.push(tag, 1);
        store.push(tag, 2);

        assert_eq!(store.get_current(tag), Some(&2));
    }

    #[test]
    fn test_next_microstep_prunes_offset_state() {
        let mut store = ActionStore::<u32>::new();
        let offset = Duration::seconds(1);

        assert_eq!(store.next_microstep_for_offset(offset, 0), 0);

        store.push(Tag::new(offset, 2), 20);
        assert_eq!(store.next_microstep_for_offset(offset, 0), 3);

        store.clear_older_than(Tag::new(Duration::seconds(2), 0));
        assert_eq!(store.next_microstep_for_offset(offset, 5), 5);
    }

    #[test]
    fn test_clear_older_than_microstep() {
        let mut store = ActionStore::<u32>::new();
        let offset = Duration::seconds(1);
        let tag0 = Tag::new(offset, 0);
        let tag1 = Tag::new(offset, 1);
        let tag2 = Tag::new(offset, 2);

        store.push(tag0, 10);
        store.push(tag1, 11);
        store.push(tag2, 12);

        store.clear_older_than(tag1);

        assert_eq!(store.get_current(tag0), None);
        assert_eq!(store.get_current(tag1), Some(&11));
        assert_eq!(store.get_current(tag2), Some(&12));
    }
}
