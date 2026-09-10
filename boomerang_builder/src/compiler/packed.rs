//! Checked ownership for compiler-generated packed relationship slices.

use crate::runtime::image::SliceRange;

/// A packed backing slice that generates checked ranges as values are appended.
pub(crate) struct PackedSliceBuilder<T> {
    table: &'static str,
    values: Vec<T>,
}

impl<T> PackedSliceBuilder<T> {
    /// Creates an empty owner for the named target-image table.
    pub(crate) const fn new(table: &'static str) -> Self {
        Self {
            table,
            values: Vec::new(),
        }
    }

    /// Appends an exact-size segment and returns its generated backing-slice range.
    pub(crate) fn try_extend_exact<I>(
        &mut self,
        values: I,
    ) -> Result<SliceRange<T>, PackedSliceOverflow>
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let values = values.into_iter();
        let start = u32::try_from(self.values.len())
            .map_err(|_| PackedSliceOverflow { table: self.table })?;
        let len =
            u32::try_from(values.len()).map_err(|_| PackedSliceOverflow { table: self.table })?;
        if self.values.len().checked_add(values.len()).is_none() {
            return Err(PackedSliceOverflow { table: self.table });
        }
        self.values.extend(values);
        Ok(SliceRange::new(start, len))
    }

    /// Finishes the packed owner as immutable backing storage.
    pub(crate) fn into_boxed_slice(self) -> Box<[T]> {
        self.values.into_boxed_slice()
    }
}

/// The named packed backing slice cannot represent another segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PackedSliceOverflow {
    table: &'static str,
}

impl PackedSliceOverflow {
    /// Returns the target-image table that exceeded its coordinate domain.
    pub(crate) const fn table(self) -> &'static str {
        self.table
    }
}
