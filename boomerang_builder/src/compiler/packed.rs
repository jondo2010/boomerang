//! Checked ownership for compiler-generated packed relationship slices.
//!
//! This module packs only anonymous, ordered relationship entries that image rows address through
//! [`SliceRange`]. Examples include reaction triggers, used ports, scope descendants, and RTI
//! dependencies. Such entries have no independent entity identity and therefore do not belong in
//! a keyed table.
//!
//! This is not identity-string packing. Stable identities remain typed string values on the host
//! and become ordinary borrowed strings or Rust string literals at the target boundary. They must
//! not be concatenated into a byte blob or addressed through byte-offset ranges.

use crate::runtime::image::SliceRange;

/// A packed backing slice that generates checked ranges as values are appended.
pub(crate) struct PackedSliceBuilder<T> {
    /// Target-image table named in capacity diagnostics.
    table: &'static str,
    /// Contiguous backing values accumulated in canonical owner order.
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

    /// Collects and appends a segment whose iterator does not expose its exact length.
    pub(crate) fn try_extend<I>(&mut self, values: I) -> Result<SliceRange<T>, PackedSliceOverflow>
    where
        I: IntoIterator<Item = T>,
    {
        self.try_extend_exact(values.into_iter().collect::<Vec<_>>())
    }

    /// Finishes the packed owner as immutable backing storage.
    pub(crate) fn into_boxed_slice(self) -> Box<[T]> {
        self.values.into_boxed_slice()
    }
}

/// The named packed backing slice cannot represent another segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PackedSliceOverflow {
    /// Target-image table whose `u32` coordinate domain was exceeded.
    table: &'static str,
}

impl PackedSliceOverflow {
    /// Returns the target-image table that exceeded its coordinate domain.
    pub(crate) const fn table(self) -> &'static str {
        self.table
    }
}
