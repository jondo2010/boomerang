use core::marker::PhantomData;

use crate::Key;

/// A start-plus-length range in the dense index domain of `K`.
#[derive(Debug, PartialEq, Eq)]
pub struct KeyRange<K: Key> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> K>,
}

impl<K: Key> Clone for KeyRange<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key> Copy for KeyRange<K> {}

impl<K: Key> KeyRange<K> {
    /// Creates an unchecked dense key range.
    pub const fn new(start: u32, len: u32) -> Self {
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }

    /// Returns the first dense key index.
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the number of keys.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u32 {
        self.len
    }
}
