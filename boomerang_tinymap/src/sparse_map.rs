//! Sparse secondary-key tables over storage-agnostic slots.
//!
//! A [`TinySecondaryMapBuilder`] addresses a fixed parent-key domain while allowing each slot to
//! be absent. Inline and caller-provided backings obtain that domain from their slot capacity;
//! heap construction accepts an explicit domain length. The builder alone may insert or replace
//! entries. Calling [`TinySecondaryMapBuilder::seal`] fixes presence before the owner moves to its
//! final location.
//!
//! Runtime code borrows [`TinySecondaryMapRef`] or [`TinySecondaryMapMut`]. Mutable views yield
//! only `&mut V`, never `&mut Option<V>`, so they cannot change sparse presence. Checked
//! [`IndexSpan`] views retain global parent keys and preserve absent slots. The backing type is an
//! owner/build concern and does not appear in borrowed runtime interfaces.

use core::marker::PhantomData;

use crate::{
    tiny_vec::{storage::Storage, SealedTinyVec, TinyVecMut, TinyVecRef},
    BorrowedStorage, IndexSpan, InlineStorage, Key, TinyMapError, TinyVecBuilder,
};

#[cfg(feature = "alloc")]
use crate::tiny_vec::HeapStorage;

/// Inline slot backing for a sparse secondary-map builder.
pub type InlineSecondaryStorage<V, const N: usize> = InlineStorage<Option<V>, N>;

/// Caller-provided slot backing for a sparse secondary-map builder.
pub type BorrowedSecondaryStorage<'a, V> = BorrowedStorage<'a, Option<V>>;

/// A sparse secondary map under construction.
#[allow(private_bounds)]
pub struct TinySecondaryMapBuilder<K: Key, V, B: Storage<Option<V>>> {
    slots: TinyVecBuilder<Option<V>, B>,
    present: usize,
    marker: PhantomData<K>,
}

/// A fixed-shape sparse secondary map ready to provide runtime views.
///
/// Sealing removes presence mutation from the public interface:
///
/// ```compile_fail,E0599
/// use boomerang_tinymap::{
///     key_type, InlineSecondaryStorage, InlineStorage, TinyMapBuilder,
///     TinySecondaryMapBuilder,
/// };
/// key_type!(EntryKey);
/// let mut entries = TinyMapBuilder::<EntryKey, (), InlineStorage<(), 2>>::inline();
/// let first = entries.try_insert(()).unwrap();
/// let second = entries.try_insert(()).unwrap();
/// let mut builder = TinySecondaryMapBuilder::<
///     EntryKey,
///     u8,
///     InlineSecondaryStorage<u8, 2>,
/// >::inline();
/// builder.try_insert(first, 1).unwrap();
/// let mut sealed = builder.seal();
/// sealed.try_insert(second, 2).unwrap();
/// ```
#[allow(private_bounds)]
pub struct SealedTinySecondaryMap<K: Key, V, B: Storage<Option<V>>> {
    slots: SealedTinyVec<Option<V>, B>,
    present: usize,
    marker: PhantomData<K>,
}

/// A borrowed read-only view of a sealed sparse secondary map.
pub struct TinySecondaryMapRef<'a, K: Key, V> {
    slots: TinyVecRef<'a, Option<V>>,
    present: usize,
    marker: PhantomData<K>,
}

/// A borrowed value-mutation view of a sealed sparse secondary map.
pub struct TinySecondaryMapMut<'a, K: Key, V> {
    slots: TinyVecMut<'a, Option<V>>,
    present: usize,
    marker: PhantomData<K>,
}

/// A read-only sparse subview that retains global parent-key coordinates.
pub struct TinySecondaryMapSpanRef<'a, K: Key, V> {
    start: usize,
    slots: &'a [Option<V>],
    marker: PhantomData<K>,
}

/// A mutable sparse subview that cannot change which slots are present.
pub struct TinySecondaryMapSpanMut<'a, K: Key, V> {
    start: usize,
    slots: &'a mut [Option<V>],
    marker: PhantomData<K>,
}

impl<K: Key, V> Clone for TinySecondaryMapSpanRef<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key, V> Copy for TinySecondaryMapSpanRef<'_, K, V> {}

impl<K: Key, V> Clone for TinySecondaryMapRef<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key, V> Copy for TinySecondaryMapRef<'_, K, V> {}

#[allow(private_bounds)]
impl<K: Key, V, B: Storage<Option<V>>> TinySecondaryMapBuilder<K, V, B> {
    fn try_from_empty_slots(
        mut slots: TinyVecBuilder<Option<V>, B>,
        domain_len: usize,
    ) -> Result<Self, TinyMapError> {
        let limit = slots.capacity().min(K::MAX_LEN);
        if domain_len > limit {
            return Err(TinyMapError::Capacity {
                limit,
                requested: domain_len,
            });
        }
        slots
            .try_extend_exact(core::iter::repeat_with(|| None).take(domain_len))
            .expect("the sparse domain was checked against its backing capacity");
        Ok(Self {
            slots,
            present: 0,
            marker: PhantomData,
        })
    }

    /// Returns the addressable parent-key slot range.
    pub const fn domain_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of present entries.
    pub const fn len(&self) -> usize {
        self.present
    }

    /// Returns whether every addressable slot is absent.
    pub const fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Inserts or replaces a value while the sparse shape is still mutable.
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, TinyMapError> {
        let index = key.index();
        if index >= self.domain_len() {
            return Err(TinyMapError::Capacity {
                limit: self.domain_len(),
                requested: index.saturating_add(1),
            });
        }
        let previous = self.slots.as_mut_slice()[index].replace(value);
        if previous.is_none() {
            self.present += 1;
        }
        Ok(previous)
    }

    /// Transfers the fixed sparse shape into a move-safe sealed table.
    pub fn seal(self) -> SealedTinySecondaryMap<K, V, B> {
        SealedTinySecondaryMap {
            slots: self.slots.seal(),
            present: self.present,
            marker: PhantomData,
        }
    }
}

impl<K: Key, V, const N: usize> TinySecondaryMapBuilder<K, V, InlineStorage<Option<V>, N>> {
    /// Creates a sparse builder with `min(N, K::MAX_LEN)` addressable inline slots.
    pub fn inline() -> Self {
        Self::try_from_empty_slots(TinyVecBuilder::inline(), N.min(K::MAX_LEN))
            .expect("the inline sparse domain is bounded by construction")
    }
}

impl<'a, K: Key, V> TinySecondaryMapBuilder<K, V, BorrowedStorage<'a, Option<V>>> {
    /// Creates a sparse builder spanning up to `K::MAX_LEN` caller-provided slots.
    pub fn borrowed(backing: BorrowedStorage<'a, Option<V>>) -> Self {
        let domain_len = backing.capacity().min(K::MAX_LEN);
        Self::try_from_empty_slots(TinyVecBuilder::borrowed(backing), domain_len)
            .expect("the borrowed sparse domain is bounded by construction")
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V> TinySecondaryMapBuilder<K, V, HeapStorage<Option<V>>> {
    /// Creates a heap-backed sparse builder with `domain_len` addressable slots.
    ///
    /// The error reports only a domain length that exceeds `K::MAX_LEN`. Heap allocation uses the
    /// global allocator and retains its normal panic-or-abort behavior on allocation failure.
    pub fn try_heap(domain_len: usize) -> Result<Self, TinyMapError> {
        Self::try_from_empty_slots(TinyVecBuilder::heap(), domain_len)
    }
}

#[allow(private_bounds)]
impl<K: Key, V, B: Storage<Option<V>>> SealedTinySecondaryMap<K, V, B> {
    /// Returns a read-only storage-erased view.
    pub fn as_ref(&self) -> TinySecondaryMapRef<'_, K, V> {
        TinySecondaryMapRef {
            slots: self.slots.as_ref(),
            present: self.present,
            marker: PhantomData,
        }
    }

    /// Returns a storage-erased view that may mutate values but not presence.
    pub fn as_mut(&mut self) -> TinySecondaryMapMut<'_, K, V> {
        TinySecondaryMapMut {
            slots: self.slots.as_mut(),
            present: self.present,
            marker: PhantomData,
        }
    }
}

impl<'a, K: Key, V> TinySecondaryMapRef<'a, K, V> {
    /// Returns the addressable parent-key slot range.
    pub const fn domain_len(&self) -> usize {
        self.slots.as_slice().len()
    }

    /// Returns the number of present entries.
    pub const fn len(&self) -> usize {
        self.present
    }

    /// Returns whether every addressable slot is absent.
    pub const fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Returns the value present at `key`, if any.
    pub fn get(&self, key: K) -> Option<&'a V> {
        self.slots.as_slice().get(key.index())?.as_ref()
    }

    /// Returns the first present key.
    pub fn first_key(&self) -> Option<K> {
        self.keys().next()
    }

    /// Iterates over present keys in ascending order.
    pub fn keys(&self) -> impl Iterator<Item = K> + 'a {
        self.slots
            .as_slice()
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|_| K::from(index)))
    }

    /// Iterates over present values in ascending key order.
    pub fn values(&self) -> impl Iterator<Item = &'a V> + 'a {
        self.slots.as_slice().iter().filter_map(Option::as_ref)
    }

    /// Iterates over present key-value pairs in ascending key order.
    pub fn iter(&self) -> impl Iterator<Item = (K, &'a V)> + 'a {
        self.slots
            .as_slice()
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|value| (K::from(index), value)))
    }

    /// Resolves a checked parent-key span while preserving absent slots.
    pub fn get_span(&self, span: IndexSpan<K>) -> Option<TinySecondaryMapSpanRef<'a, K, V>> {
        Some(TinySecondaryMapSpanRef {
            start: span.start(),
            slots: self.slots.as_slice().get(span.indices()?)?,
            marker: PhantomData,
        })
    }
}

impl<'a, K: Key, V> TinySecondaryMapSpanRef<'a, K, V> {
    /// Returns the first global key, or `None` for an empty span.
    pub fn start_key(&self) -> Option<K> {
        (!self.slots.is_empty()).then(|| K::from(self.start))
    }

    /// Returns the number of addressed slots, including absent slots.
    pub const fn len(&self) -> usize {
        self.slots.len()
    }

    /// Returns whether this span addresses no slots.
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns the value present at a global key within this span.
    pub fn get(&self, key: K) -> Option<&'a V> {
        let offset = key.index().checked_sub(self.start)?;
        self.slots.get(offset)?.as_ref()
    }

    /// Iterates over present entries while preserving global keys.
    pub fn iter(&self) -> impl Iterator<Item = (K, &'a V)> + 'a {
        let start = self.start;
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(offset, slot)| {
                slot.as_ref().map(|value| (K::from(start + offset), value))
            })
    }
}

impl<K: Key, V> TinySecondaryMapMut<'_, K, V> {
    /// Returns the addressable parent-key slot range.
    pub const fn domain_len(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of present entries.
    pub const fn len(&self) -> usize {
        self.present
    }

    /// Returns whether every addressable slot is absent.
    pub const fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Returns the value present at `key`, if any.
    pub fn get(&self, key: K) -> Option<&V> {
        self.slots.as_slice().get(key.index())?.as_ref()
    }

    /// Returns the value present at `key` for mutation, if any.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        self.slots.as_mut_slice().get_mut(key.index())?.as_mut()
    }

    /// Iterates mutably over present entries without changing sparse presence.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        self.slots
            .as_mut_slice()
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_mut().map(|value| (K::from(index), value)))
    }

    /// Resolves a checked mutable parent-key span without changing presence.
    pub fn get_span_mut(
        &mut self,
        span: IndexSpan<K>,
    ) -> Option<TinySecondaryMapSpanMut<'_, K, V>> {
        Some(TinySecondaryMapSpanMut {
            start: span.start(),
            slots: self.slots.as_mut_slice().get_mut(span.indices()?)?,
            marker: PhantomData,
        })
    }

    /// Resolves disjoint mutable spans after atomically validating all inputs.
    pub fn get_disjoint_spans_mut<const N: usize>(
        &mut self,
        spans: [IndexSpan<K>; N],
    ) -> Result<[TinySecondaryMapSpanMut<'_, K, V>; N], TinyMapError> {
        let domain_len = self.domain_len();
        for span in spans {
            if span.checked_end().is_none_or(|end| end > domain_len) {
                return Err(TinyMapError::InvalidSpan {
                    start: span.start(),
                    len: span.len(),
                    domain_len,
                });
            }
        }
        for first in 0..N {
            let first_end = spans[first]
                .checked_end()
                .expect("bounds were checked above");
            for second in first + 1..N {
                let second_end = spans[second]
                    .checked_end()
                    .expect("bounds were checked above");
                let overlaps = !spans[first].is_empty()
                    && !spans[second].is_empty()
                    && spans[first].start() < second_end
                    && spans[second].start() < first_end;
                if overlaps {
                    return Err(TinyMapError::OverlappingSpans { first, second });
                }
            }
        }

        let slots = self.slots.as_mut_slice();
        let base = slots.as_mut_ptr();
        Ok(spans.map(|span| {
            // SAFETY: every span is in bounds, and every pair is disjoint.
            // Each returned slice therefore has exclusive access to its slots
            // for the lifetime of this exclusive sparse-map borrow.
            let slots =
                unsafe { core::slice::from_raw_parts_mut(base.add(span.start()), span.len()) };
            TinySecondaryMapSpanMut {
                start: span.start(),
                slots,
                marker: PhantomData,
            }
        }))
    }
}

impl<K: Key, V> TinySecondaryMapSpanMut<'_, K, V> {
    /// Returns the first global key, or `None` for an empty span.
    pub fn start_key(&self) -> Option<K> {
        (!self.slots.is_empty()).then(|| K::from(self.start))
    }

    /// Returns the number of addressed slots, including absent slots.
    pub const fn len(&self) -> usize {
        self.slots.len()
    }

    /// Returns whether this span addresses no slots.
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns the value present at a global key within this span.
    pub fn get(&self, key: K) -> Option<&V> {
        let offset = key.index().checked_sub(self.start)?;
        self.slots.get(offset)?.as_ref()
    }

    /// Returns the value present at a global key for mutation.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let offset = key.index().checked_sub(self.start)?;
        self.slots.get_mut(offset)?.as_mut()
    }

    /// Iterates over present entries while preserving global keys.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        let start = self.start;
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(move |(offset, slot)| {
                slot.as_mut().map(|value| (K::from(start + offset), value))
            })
    }
}
