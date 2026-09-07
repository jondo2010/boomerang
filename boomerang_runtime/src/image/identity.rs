//! Borrowed storage for open identities in compiled images.
//!
//! Compiled images encode user- or implementation-defined names in one UTF-8
//! string and store a dense table of byte ranges into it. [`IdentityTable`]
//! binds those two pieces together so callers see keyed identities rather than
//! an unexplained `IdentityRange` payload. Closed vocabularies such as runtime
//! policies belong in enums instead.

use super::IdentityRange;
use tinymap::{Key, TinyMapView};

/// A densely keyed table of UTF-8 identities stored as ranges into one backing string.
///
/// Unlike `TinyMapView<K, IdentityRange>`, this type makes the range payload and its
/// backing storage one inseparable abstraction. It is used only for open stable
/// identities; closed operational vocabularies are represented by enums.
#[derive(Clone, Copy, Debug)]
pub struct IdentityTable<'a, K: Key> {
    pub(super) identity_data: &'a str,
    ranges: TinyMapView<'a, K, IdentityRange>,
}

impl<'a, K: Key> IdentityTable<'a, K> {
    /// Creates an unchecked identity table over `identity_data` and its byte ranges.
    #[must_use]
    pub const fn new(identity_data: &'a str, ranges: TinyMapView<'a, K, IdentityRange>) -> Self {
        Self {
            identity_data,
            ranges,
        }
    }

    /// Returns the number of densely keyed identities.
    #[must_use]
    pub const fn len(self) -> usize {
        self.ranges.len()
    }

    /// Returns whether the table contains no identities.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.ranges.is_empty()
    }

    /// Resolves one key to its UTF-8 identity, or returns `None` for an invalid key or range.
    #[must_use]
    pub fn get(self, key: K) -> Option<&'a str> {
        self.ranges.get(key)?.get(self.identity_data)
    }

    pub(super) const fn ranges(self) -> TinyMapView<'a, K, IdentityRange> {
        self.ranges
    }
}

impl<K: Key> PartialEq for IdentityTable<'_, K> {
    fn eq(&self, other: &Self) -> bool {
        self.identity_data == other.identity_data && self.ranges.values().eq(other.ranges.values())
    }
}

impl<K: Key> Eq for IdentityTable<'_, K> {}
