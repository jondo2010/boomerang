//! This module provides an implementation of an `ActionStore` for managing actions in a reactor system.
//!
//! The `ActionStore` is a data structure that efficiently stores and retrieves actions based on their
//! associated [`Tag`]s. It uses a binary heap internally to maintain the actions
//! in a priority queue, ensuring that actions can be processed in the correct order.
//!
//! Key features:
//! - Out-of-order insertion and update. Pushing a new value for a tag will semantically replace the old value.
//! - Retrieval follows the monotonically increasing tag order of the scheduler.
//! - Requests for the same current tag will return the same value.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::{ActionData, Tag};

#[derive(Debug)]
struct ActionEntry<T>
where
    T: ActionData,
{
    tag: Tag,
    sequence: usize,
    data: Option<T>,
}

impl<T> Ord for ActionEntry<T>
where
    T: ActionData,
{
    fn cmp(&self, other: &Self) -> Ordering {
        // This is set up so that in ties on the tag, the higher sequence number comes first
        self.tag
            .cmp(&other.tag)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .reverse()
    }
}

impl<T> PartialOrd for ActionEntry<T>
where
    T: ActionData,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Eq for ActionEntry<T> where T: ActionData {}

impl<T> PartialEq for ActionEntry<T>
where
    T: ActionData,
{
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.sequence == other.sequence
    }
}

pub trait BaseActionStore: std::fmt::Debug + Send + Sync + downcast_rs::DowncastSync {
    /// Remove any value at the given Tag
    fn clear_older_than(&mut self, tag: Tag);

    /// Try to serialize a value at the given Tag
    #[cfg(feature = "serde")]
    fn serialize_value(
        &mut self,
        tag: Tag,
        ser: &mut dyn erased_serde::Serializer,
    ) -> Result<(), erased_serde::Error>;

    /// Try to pull a value from the deserializer and store it at the given Tag
    #[cfg(feature = "serde")]
    fn deserialize_value(
        &mut self,
        tag: Tag,
        des: &mut dyn erased_serde::Deserializer<'_>,
    ) -> Result<(), erased_serde::Error>;
}
downcast_rs::impl_downcast!(sync BaseActionStore);

#[derive(Debug)]
pub struct ActionStore<T>
where
    T: ActionData,
{
    heap: BinaryHeap<ActionEntry<T>>,
    counter: usize,
}

impl<T> ActionStore<T>
where
    T: ActionData,
{
    pub fn new() -> Self {
        ActionStore {
            heap: BinaryHeap::new(),
            counter: 0,
        }
    }

    /// Add a new action to the store.
    #[inline]
    pub fn push(&mut self, tag: Tag, data: Option<T>) {
        self.heap.push(ActionEntry {
            tag,
            sequence: self.counter,
            data,
        });
        self.counter += 1;
    }

    pub fn clear_older_than(&mut self, clear_tag: Tag) {
        while let Some(entry) = self.heap.peek() {
            if entry.tag < clear_tag {
                self.heap.pop();
            } else {
                break;
            }
        }
    }

    /// Get the current action data for a given tag.
    ///
    /// This method pops all entries older than `tag` from the store.
    ///
    /// If the store is empty, or only entries newer than `tag` this method returns `None`.
    pub fn get_current(&mut self, tag: Tag) -> Option<&T> {
        if self.heap.is_empty() {
            return None;
        }

        // Remove entries older than the given tag
        self.clear_older_than(tag);

        // Return Some only if the top entry's tag matches the given tag
        self.heap.peek().and_then(|entry| {
            if entry.tag == tag {
                entry.data.as_ref()
            } else {
                None
            }
        })
    }
}

impl<T: ActionData> BaseActionStore for ActionStore<T> {
    fn clear_older_than(&mut self, tag: Tag) {
        self.clear_older_than(tag)
    }

    #[cfg(feature = "serde")]
    fn serialize_value(
        &mut self,
        tag: Tag,
        ser: &mut dyn erased_serde::Serializer,
    ) -> Result<(), erased_serde::Error> {
        let value = self.get_current(tag);
        erased_serde::Serialize::erased_serialize(&value, ser)
    }

    #[cfg(feature = "serde")]
    fn deserialize_value(
        &mut self,
        tag: Tag,
        des: &mut dyn erased_serde::Deserializer<'_>,
    ) -> Result<(), erased_serde::Error> {
        let value = <Option<T> as serde::Deserialize>::deserialize(des)?;
        self.push(tag, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn build_tags<const N: usize>() -> [Tag; N] {
        (0..N)
            .map(|i| Tag::new(Duration::from_secs(i as u64), 0))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    #[test]
    fn test_action_entry_ordering() {
        let entry1 = ActionEntry::<()> {
            tag: Tag::new(Duration::from_secs(1), 0),
            sequence: 40,
            data: None,
        };
        let entry2 = ActionEntry::<()> {
            tag: Tag::new(Duration::from_secs(1), 0),
            sequence: 41,
            data: None,
        };
        assert!(entry2 > entry1);
    }

    #[test]
    fn test_heap_ordering() {
        let mut store = ActionStore::<u32>::new();

        let tags = build_tags::<5>();
        // The first 3 tags should come out in tag order
        store.push(tags[3], Some(30));
        store.push(tags[1], Some(10));
        store.push(tags[2], Some(20));
        // The last 3 tags with the same value, should come out in reverse push order
        store.push(tags[4], Some(41));
        store.push(tags[4], Some(40));
        store.push(tags[4], Some(42));

        assert_eq!(store.heap.pop().unwrap().data, Some(10));
        assert_eq!(store.heap.pop().unwrap().data, Some(20));
        assert_eq!(store.heap.pop().unwrap().data, Some(30));
        assert_eq!(store.heap.pop().unwrap().data, Some(42));
        assert_eq!(store.heap.pop().unwrap().data, Some(40));
        assert_eq!(store.heap.pop().unwrap().data, Some(41));
    }

    #[test]
    fn test_out_of_order_get_current() {
        let mut store = ActionStore::<u32>::new();

        let tags = build_tags::<6>();
        store.push(tags[3], Some(30));
        store.push(tags[1], Some(10));
        store.push(tags[2], Some(20));

        // We now update the value of tag4 3 times, so the last one should be the one that comes out
        store.push(tags[4], Some(41));
        store.push(tags[4], Some(40));
        store.push(tags[4], Some(42));

        store.push(tags[5], Some(50));

        assert_eq!(store.get_current(tags[0]), None);
        assert_eq!(store.get_current(tags[1]), Some(&10));
        assert_eq!(store.get_current(tags[1]), Some(&10));
        assert_eq!(store.get_current(tags[2]), Some(&20));
        assert_eq!(store.get_current(tags[3]), Some(&30));
        assert_eq!(store.get_current(tags[4]), Some(&42));
        assert_eq!(store.get_current(tags[4]), Some(&42));
        assert_eq!(store.get_current(tags[5]), Some(&50));
        assert_eq!(store.get_current(tags[5]), Some(&50));
        assert_eq!(store.get_current(tags[4]), None);
    }

    #[test]
    fn test_empty_store() {
        let mut store = ActionStore::<u32>::new();
        assert_eq!(store.get_current(Tag::new(Duration::from_secs(1), 0)), None);
    }

    #[cfg(feature = "fixme")]
    #[cfg(feature = "serde")]
    #[test]
    fn test_serialize_deserialize() {
        use serde_json;

        let mut store = ActionStore::<u32>::new();
        let tags = build_tags::<3>();

        store.push(tags[0], Some(10));
        store.push(tags[1], Some(20));
        store.push(tags[2], Some(30));

        // Serialize
        let mut serialized = Vec::new();
        {
            let mut json = serde_json::Serializer::new(&mut serialized);
            let mut ser = Box::new(<dyn erased_serde::Serializer>::erase(&mut json));

            store.serialize_value(tags[1], &mut ser).unwrap();
            store.serialize_value(tags[2], &mut ser).unwrap();
        }

        println!("serialized: {}", String::from_utf8_lossy(&serialized));

        // Deserialize into a new store
        let mut new_store = ActionStore::<u32>::new();
        {
            let mut deserializer = serde_json::Deserializer::from_slice(&serialized);
            let mut des = Box::new(<dyn erased_serde::Deserializer>::erase(&mut deserializer));
            new_store.deserialize_value(tags[1], &mut des).unwrap();
        }

        // Check that the deserialized value is correct
        assert_eq!(new_store.get_current(tags[1]), Some(&20));
    }
}
