use core::marker::PhantomData;

/// A typed start-plus-length range into a contiguous table.
///
/// `T` is the table element type, or the dense key type when used with
/// [`crate::TinyMapView`].
#[derive(Debug, PartialEq, Eq)]
pub struct TableRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for TableRange<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for TableRange<T> {}

impl<T> TableRange<T> {
    /// Creates an unchecked table range.
    pub const fn new(start: u32, len: u32) -> Self {
        Self {
            start,
            len,
            marker: PhantomData,
        }
    }

    /// Returns the first table index.
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the number of entries.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Returns the platform-sized exclusive end, or `None` when unaddressable.
    pub const fn checked_end(self) -> Option<usize> {
        (self.start as usize).checked_add(self.len as usize)
    }

    /// Returns whether the table index belongs to this range.
    pub const fn contains(self, index: u32) -> bool {
        let index = index as u64;
        let start = self.start as u64;
        index >= start && index < start + self.len as u64
    }

    /// Returns this range as checked platform-sized indices.
    pub(crate) fn indices(self) -> Option<core::ops::Range<usize>> {
        Some(self.start as usize..self.checked_end()?)
    }

    /// Returns the entries in this range, or `None` when it exceeds `values`.
    pub fn get(self, values: &[T]) -> Option<&[T]> {
        values.get(self.indices()?)
    }
}
