//! This module provides an implementation of an `ActionStore` for managing actions in a reactor system.
//!
//! The [`ActionStore`] is a data structure that efficiently stores and retrieves actions based on their
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

    /// Create a new Arrow ArrayBuilder for the data stored in this store
    #[cfg(feature = "serde")]
    fn new_builder(&self) -> Result<serde_arrow::ArrayBuilder, crate::RuntimeError>;

    /// Serialize the latest value in the store to the given `ArrayBuilder`.
    #[cfg(feature = "serde")]
    fn build_value_at(
        &mut self,
        builder: &mut serde_arrow::ArrayBuilder,
        tag: Tag,
    ) -> Result<(), crate::RuntimeError>;
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

#[cfg(feature = "serde")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct TaggedActionRecord<T> {
    tag: Tag,
    value: Option<T>,
}

impl<T: ActionData> BaseActionStore for ActionStore<T> {
    fn clear_older_than(&mut self, tag: Tag) {
        self.clear_older_than(tag)
    }

    #[cfg(feature = "serde")]
    fn new_builder(&self) -> Result<serde_arrow::ArrayBuilder, crate::RuntimeError> {
        use arrow::datatypes::Field;
        use serde_arrow::schema::{SchemaLike, SerdeArrowSchema, TracingOptions};
        let fields = Vec::<Field>::from_type::<TaggedActionRecord<T>>(
            TracingOptions::default().allow_null_fields(true),
        )?;
        let schema = SerdeArrowSchema::from_arrow_fields(fields.as_slice())?;
        serde_arrow::ArrayBuilder::new(schema).map_err(crate::RuntimeError::from)
    }

    #[cfg(feature = "serde")]
    fn build_value_at(
        &mut self,
        builder: &mut serde_arrow::ArrayBuilder,
        tag: Tag,
    ) -> Result<(), crate::RuntimeError> {
        let value = self.get_current(tag);
        builder
            .push(&TaggedActionRecord { tag, value })
            .map_err(crate::RuntimeError::from)
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

    #[cfg(feature = "serde")]
    #[test]
    fn test_arrow() {
        #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
        struct TestStruct {
            name: String,
            data: u32,
        }
        let mut store = ActionStore::<TestStruct>::new();
        let tag = Tag::now(crate::Timestamp::now());
        store.push(
            tag,
            Some(TestStruct {
                name: "test".to_string(),
                data: 42,
            }),
        );

        let mut builder = store.new_builder().unwrap();
        store.build_value_at(&mut builder, tag).unwrap();
        store
            .build_value_at(&mut builder, tag.delay(Duration::ZERO))
            .unwrap();

        arrow::util::pretty::print_batches(&[builder.to_record_batch().unwrap()]).unwrap();
    }
}
