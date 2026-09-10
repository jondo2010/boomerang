use core::marker::PhantomData;

/// A contiguous span of keys allocated by a dense map owner.
///
/// Production code should obtain spans from [`crate::TinyMap::try_extend_exact`].
/// [`IndexSpan::new`] exists for immutable generated images and test fixtures that
/// reconstruct already-allocated metadata.
#[derive(Debug, PartialEq, Eq)]
pub struct IndexSpan<K: crate::Key> {
    start: usize,
    len: usize,
    marker: PhantomData<fn() -> K>,
}

impl<K: crate::Key> Clone for IndexSpan<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: crate::Key> Copy for IndexSpan<K> {}

impl<K: crate::Key> IndexSpan<K> {
    /// Reconstructs a dense-key span from validated or generated metadata.
    pub const fn new(start: usize, len: usize) -> Self {
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }

    /// Returns the first dense-table index.
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the number of keys in the span.
    pub const fn len(self) -> usize {
        self.len
    }

    /// Returns whether the span contains no keys.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Returns the exclusive end, or `None` when the span overflows `usize`.
    pub const fn checked_end(self) -> Option<usize> {
        self.start.checked_add(self.len)
    }

    pub(crate) fn indices(self) -> Option<core::ops::Range<usize>> {
        Some(self.start..self.checked_end()?)
    }

    /// Returns whether `key` belongs to this owner-allocated span.
    pub fn contains(self, key: K) -> bool {
        self.checked_end()
            .is_some_and(|end| key.index() >= self.start && key.index() < end)
    }
}

/// A checked start-plus-length range into a packed contiguous backing slice.
#[derive(Debug, PartialEq, Eq)]
pub struct SliceRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for SliceRange<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for SliceRange<T> {}

impl<T> SliceRange<T> {
    /// Reconstructs a packed-slice range from validated or generated metadata.
    pub const fn new(start: u32, len: u32) -> Self {
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }

    /// Returns the first backing-slice index.
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the number of entries.
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Returns whether the range contains no entries.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Returns the platform-sized exclusive end, or `None` when unaddressable.
    pub const fn checked_end(self) -> Option<usize> {
        (self.start as usize).checked_add(self.len as usize)
    }

    pub(crate) fn indices(self) -> Option<core::ops::Range<usize>> {
        Some(self.start as usize..self.checked_end()?)
    }

    /// Returns the entries in this range, or `None` when it exceeds `values`.
    pub fn get(self, values: &[T]) -> Option<&[T]> {
        values.get(self.indices()?)
    }
}
