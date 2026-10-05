//! Dense owner-generated keys over storage-agnostic values.
//!
//! Use [`TinyMapBuilder`] while allocating keys and initializing values. Calling
//! [`TinyMapBuilder::seal`] transfers the backing into a fixed-shape [`SealedTinyMap`], which may
//! then move to its final owning location. Runtime code borrows [`TinyMapRef`] for shared access or
//! [`TinyMapMut`] for value mutation that cannot change the key domain.
//!
//! Inline, borrowed, and heap-backed builders share these semantics. The backing type remains an
//! owner/build concern and is erased from borrowed views. Sealing one table does not perform
//! whole-image validation, pinning, or linking; an owning image must perform those later phases
//! before publishing stable non-owning handles.
//!
//! [`IndexSpan`] values come only from the owning dense key domain. Span views
//! retain global key coordinates rather than rebasing the selected values to zero.
//! For hosted construction, [`HeapTinyMapBuilder`] and [`HeapSealedTinyMap`] name the concrete
//! heap backing. A sealed heap map can consume its values exactly once with
//! [`SealedTinyMap::into_values`]. [`TinyMapRef::from_slice`] provides a const borrowed view of
//! static or code-generated arrays whose order is already the authoritative dense domain; it
//! does not allocate keys at runtime.

use core::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::{
    tiny_vec::{storage::Storage, SealedTinyVec, TinyVecMut, TinyVecRef},
    BorrowedStorage, IndexSpan, InlineStorage, Key, TinyMapError, TinyVecBuilder,
};

#[cfg(feature = "alloc")]
use crate::tiny_vec::HeapStorage;

/// A dense owner-keyed table under construction.
#[allow(private_bounds)]
pub struct TinyMapBuilder<K: Key, V, B: Storage<V>> {
    values: TinyVecBuilder<V, B>,
    marker: PhantomData<K>,
}

/// A heap-backed dense builder for hosted construction.
#[cfg(feature = "alloc")]
pub type HeapTinyMapBuilder<K, V> = TinyMapBuilder<K, V, HeapStorage<V>>;

/// A fixed-shape dense table ready to provide borrowed runtime views.
///
/// Sealing removes structural mutation from the public interface:
///
/// ```compile_fail,E0599
/// use boomerang_tinymap::{key_type, InlineStorage, TinyMapBuilder};
/// key_type!(EntryKey);
/// let mut builder = TinyMapBuilder::<EntryKey, u8, InlineStorage<u8, 2>>::inline();
/// builder.try_insert(1).unwrap();
/// let mut sealed = builder.seal();
/// sealed.try_insert(2).unwrap();
/// ```
#[allow(private_bounds)]
pub struct SealedTinyMap<K: Key, V, B: Storage<V>> {
    values: SealedTinyVec<V, B>,
    marker: PhantomData<K>,
}

/// A heap-backed fixed-shape dense table.
#[cfg(feature = "alloc")]
pub type HeapSealedTinyMap<K, V> = SealedTinyMap<K, V, HeapStorage<V>>;

#[cfg(feature = "alloc")]
impl<K: Key, V: Clone> Clone for HeapSealedTinyMap<K, V> {
    fn clone(&self) -> Self {
        let mut builder = HeapTinyMapBuilder::<K, V>::heap();
        builder
            .try_extend_exact(self.as_ref().values().cloned())
            .unwrap_or_else(|_| {
                unreachable!("cloning an existing sealed map preserves its length")
            });
        builder.seal()
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V: core::fmt::Debug> core::fmt::Debug for HeapSealedTinyMap<K, V> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("SealedTinyMap")
            .field(&self.as_ref().as_slice())
            .finish()
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V: PartialEq> PartialEq for HeapSealedTinyMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref().as_slice() == other.as_ref().as_slice()
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V: Eq> Eq for HeapSealedTinyMap<K, V> {}

/// A borrowed read-only view of a sealed dense table.
pub struct TinyMapRef<'a, K: Key, V> {
    values: TinyVecRef<'a, V>,
    marker: PhantomData<K>,
}

impl<K: Key, V> Clone for TinyMapRef<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key, V> Copy for TinyMapRef<'_, K, V> {}

impl<K: Key, V: core::fmt::Debug> core::fmt::Debug for TinyMapRef<'_, K, V> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TinyMapRef")
            .field("values", &self.values.as_slice())
            .finish()
    }
}

/// A borrowed value-mutation view of a sealed dense table.
pub struct TinyMapMut<'a, K: Key, V> {
    values: TinyVecMut<'a, V>,
    marker: PhantomData<K>,
}

/// A read-only subview that retains its keys' global dense-map coordinates.
pub struct TinyMapSpanRef<'a, K: Key, V> {
    start: usize,
    values: &'a [V],
    marker: PhantomData<K>,
}

impl<K: Key, V> Clone for TinyMapSpanRef<'_, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Key, V> Copy for TinyMapSpanRef<'_, K, V> {}

/// A value-mutation subview that retains global keys without changing shape.
pub struct TinyMapSpanMut<'a, K: Key, V> {
    start: usize,
    values: &'a mut [V],
    marker: PhantomData<K>,
}

#[allow(private_bounds)]
impl<K: Key, V, B: Storage<V>> TinyMapBuilder<K, V, B> {
    /// Returns the number of initialized values.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the builder contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the effective capacity shared by the key domain and backing.
    pub fn capacity(&self) -> usize {
        self.values.capacity().min(K::MAX_LEN)
    }

    /// Returns a value by a key issued by this builder.
    pub fn get(&self, key: K) -> Option<&V> {
        self.values.as_slice().get(key.index())
    }

    /// Returns a mutable value by a key issued by this builder.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        self.values.as_mut_slice().get_mut(key.index())
    }

    /// Inserts one value and returns its owner-generated key.
    pub fn try_insert(&mut self, value: V) -> Result<K, TinyMapError> {
        self.try_insert_with_key(|_| value)
    }

    /// Inserts one key-derived value without invoking the factory on overflow.
    pub fn try_insert_with_key<F>(&mut self, factory: F) -> Result<K, TinyMapError>
    where
        F: FnOnce(K) -> V,
    {
        let requested = self.len().saturating_add(1);
        let limit = self.capacity();
        if requested > limit {
            return Err(TinyMapError::Capacity { limit, requested });
        }
        let key = K::from(self.len());
        self.values.try_push(factory(key))?;
        Ok(key)
    }

    /// Appends an exact-size segment and returns its global key span.
    pub fn try_extend_exact<I>(&mut self, values: I) -> Result<IndexSpan<K>, TinyMapError>
    where
        I: ExactSizeIterator<Item = V>,
    {
        let start = self.len();
        let additional = values.len();
        let requested = start.saturating_add(additional);
        let limit = self.capacity();
        if start.checked_add(additional).is_none() || requested > limit {
            return Err(TinyMapError::Capacity { limit, requested });
        }
        self.values.try_extend_exact(values)?;
        Ok(IndexSpan::new(start, additional))
    }

    /// Transfers the fixed dense shape into a move-safe sealed table.
    pub fn seal(self) -> SealedTinyMap<K, V, B> {
        SealedTinyMap {
            values: self.values.seal(),
            marker: PhantomData,
        }
    }
}

impl<K: Key, V, const N: usize> TinyMapBuilder<K, V, InlineStorage<V, N>> {
    /// Creates a dense builder backed by exactly `N` inline slots.
    pub fn inline() -> Self {
        Self {
            values: TinyVecBuilder::inline(),
            marker: PhantomData,
        }
    }
}

impl<'a, K: Key, V> TinyMapBuilder<K, V, BorrowedStorage<'a, V>> {
    /// Creates a dense builder over caller-provided slots.
    pub fn borrowed(backing: BorrowedStorage<'a, V>) -> Self {
        Self {
            values: TinyVecBuilder::borrowed(backing),
            marker: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V> TinyMapBuilder<K, V, HeapStorage<V>> {
    /// Creates an empty heap-backed dense builder.
    pub fn heap() -> Self {
        Self {
            values: TinyVecBuilder::heap(),
            marker: PhantomData,
        }
    }
}

#[allow(private_bounds)]
impl<K: Key, V, B: Storage<V>> SealedTinyMap<K, V, B> {
    /// Returns the fixed number of values.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the table contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Borrows the table for read-only keyed access.
    pub fn as_ref(&self) -> TinyMapRef<'_, K, V> {
        TinyMapRef {
            values: self.values.as_ref(),
            marker: PhantomData,
        }
    }

    /// Borrows the table for value mutation without changing its shape.
    pub fn as_mut(&mut self) -> TinyMapMut<'_, K, V> {
        TinyMapMut {
            values: self.values.as_mut(),
            marker: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<K: Key, V> SealedTinyMap<K, V, HeapStorage<V>> {
    /// Consumes a heap-backed table and yields its values in dense order.
    pub fn into_values(self) -> alloc::vec::IntoIter<V> {
        self.values.into_vec().into_iter()
    }
}

impl<'a, K: Key, V> TinyMapRef<'a, K, V> {
    /// Borrows an authoritative static or code-generated dense value slice.
    ///
    /// The caller supplies values in the owning image's established key order;
    /// this adapter does not allocate runtime keys.
    ///
    /// # Panics
    ///
    /// Panics when `values` exceeds the key type's supported table length.
    pub const fn from_slice(values: &'a [V]) -> Self {
        assert!(values.len() <= K::MAX_LEN, "dense view exceeds key domain");
        Self {
            values: TinyVecRef::from_slice(values),
            marker: PhantomData,
        }
    }

    /// Returns the exact borrowed values in the owner's dense key order.
    pub const fn as_slice(&self) -> &'a [V] {
        self.values.as_slice()
    }

    /// Returns the number of values.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the view contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the value owned by `key`.
    pub fn get(&self, key: K) -> Option<&'a V> {
        self.values.as_slice().get(key.index())
    }

    /// Returns the values in an owner-generated global key span.
    pub fn get_span(&self, span: IndexSpan<K>) -> Option<TinyMapSpanRef<'a, K, V>> {
        Some(TinyMapSpanRef {
            start: span.start(),
            values: self.values.as_slice().get(span.indices()?)?,
            marker: PhantomData,
        })
    }

    /// Iterates over values in dense key order.
    pub fn values(&self) -> core::slice::Iter<'a, V> {
        self.values.iter()
    }

    /// Iterates over generated keys in dense order.
    pub fn keys(&self) -> impl Iterator<Item = K> {
        (0..self.len()).map(K::from)
    }

    /// Iterates over keyed values in dense order.
    pub fn iter(&self) -> impl Iterator<Item = (K, &'a V)> {
        self.values
            .iter()
            .enumerate()
            .map(|(index, value)| (K::from(index), value))
    }
}

impl<K: Key, V> Index<K> for TinyMapRef<'_, K, V> {
    type Output = V;

    fn index(&self, key: K) -> &Self::Output {
        &self.values.as_slice()[key.index()]
    }
}

impl<'a, K: Key, V> TinyMapMut<'a, K, V> {
    /// Returns the number of values.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the view contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the value owned by `key`.
    pub fn get(&self, key: K) -> Option<&V> {
        self.values.as_slice().get(key.index())
    }

    /// Returns the mutable value owned by `key`.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        self.values.as_mut_slice().get_mut(key.index())
    }

    /// Returns the values in an owner-generated global key span.
    pub fn get_span(&self, span: IndexSpan<K>) -> Option<TinyMapSpanRef<'_, K, V>> {
        Some(TinyMapSpanRef {
            start: span.start(),
            values: self.values.as_slice().get(span.indices()?)?,
            marker: PhantomData,
        })
    }

    /// Returns mutable values in an owner-generated global key span.
    pub fn get_span_mut(&mut self, span: IndexSpan<K>) -> Option<TinyMapSpanMut<'_, K, V>> {
        Some(TinyMapSpanMut {
            start: span.start(),
            values: self.values.as_mut_slice().get_mut(span.indices()?)?,
            marker: PhantomData,
        })
    }

    /// Returns mutable span views after checking every bound and overlap.
    pub fn get_disjoint_spans_mut<const N: usize>(
        &mut self,
        spans: [IndexSpan<K>; N],
    ) -> Result<[TinyMapSpanMut<'_, K, V>; N], TinyMapError> {
        let domain_len = self.len();
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

        let values = self.values.as_mut_slice();
        let base = values.as_mut_ptr();
        Ok(spans.map(|span| {
            // SAFETY: all spans were checked against `values`, and all pairs
            // were checked for overlap before any mutable view was published.
            // Each slice therefore covers only its own in-bounds elements for
            // the lifetime of this exclusive map borrow.
            let values =
                unsafe { core::slice::from_raw_parts_mut(base.add(span.start()), span.len()) };
            TinyMapSpanMut {
                start: span.start(),
                values,
                marker: PhantomData,
            }
        }))
    }

    /// Iterates over values in dense key order.
    pub fn values(&self) -> core::slice::Iter<'_, V> {
        self.values.iter()
    }

    /// Iterates mutably over values without changing table shape.
    pub fn values_mut(&mut self) -> core::slice::IterMut<'_, V> {
        self.values.iter_mut()
    }

    /// Iterates over generated keys in dense order.
    pub fn keys(&self) -> impl Iterator<Item = K> {
        (0..self.len()).map(K::from)
    }

    /// Iterates over keyed values in dense order.
    pub fn iter(&self) -> impl Iterator<Item = (K, &V)> {
        self.values
            .iter()
            .enumerate()
            .map(|(index, value)| (K::from(index), value))
    }

    /// Iterates mutably over keyed values without changing table shape.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        self.values
            .iter_mut()
            .enumerate()
            .map(|(index, value)| (K::from(index), value))
    }
}

impl<K: Key, V> Index<K> for TinyMapMut<'_, K, V> {
    type Output = V;

    fn index(&self, key: K) -> &Self::Output {
        &self.values.as_slice()[key.index()]
    }
}

impl<K: Key, V> IndexMut<K> for TinyMapMut<'_, K, V> {
    fn index_mut(&mut self, key: K) -> &mut Self::Output {
        &mut self.values.as_mut_slice()[key.index()]
    }
}

impl<'a, K: Key, V> TinyMapSpanRef<'a, K, V> {
    /// Returns the numeric global coordinate at which this span starts.
    pub const fn start_index(&self) -> usize {
        self.start
    }

    /// Returns the first global key, or `None` for an empty one-past-domain span.
    pub fn start_key(&self) -> Option<K> {
        (self.start < K::MAX_LEN).then(|| K::from(self.start))
    }

    /// Returns the number of values in this span.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether this span contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the values while the span object retains their global start.
    pub const fn values(&self) -> &'a [V] {
        self.values
    }

    /// Returns a value only when `key` belongs to this global span.
    pub fn get(&self, key: K) -> Option<&'a V> {
        self.values.get(key.index().checked_sub(self.start)?)
    }

    /// Iterates over values paired with their global keys.
    pub fn iter(&self) -> impl Iterator<Item = (K, &'a V)> {
        let start = self.start;
        self.values
            .iter()
            .enumerate()
            .map(move |(offset, value)| (K::from(start + offset), value))
    }
}

impl<K: Key, V> TinyMapSpanMut<'_, K, V> {
    /// Returns the numeric global coordinate at which this span starts.
    pub const fn start_index(&self) -> usize {
        self.start
    }

    /// Returns the first global key, or `None` for an empty one-past-domain span.
    pub fn start_key(&self) -> Option<K> {
        (self.start < K::MAX_LEN).then(|| K::from(self.start))
    }

    /// Returns the number of values in this span.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether this span contains no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the span's values for read-only access.
    pub fn values(&self) -> &[V] {
        self.values
    }

    /// Returns the span's values for mutation without changing shape.
    pub fn values_mut(&mut self) -> &mut [V] {
        self.values
    }

    /// Returns a value only when `key` belongs to this global span.
    pub fn get(&self, key: K) -> Option<&V> {
        self.values.get(key.index().checked_sub(self.start)?)
    }

    /// Returns a mutable value only when `key` belongs to this global span.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        self.values.get_mut(key.index().checked_sub(self.start)?)
    }

    /// Iterates over values paired with their global keys.
    pub fn iter(&self) -> impl Iterator<Item = (K, &V)> {
        let start = self.start;
        self.values
            .iter()
            .enumerate()
            .map(move |(offset, value)| (K::from(start + offset), value))
    }

    /// Iterates mutably over values paired with their global keys.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        let start = self.start;
        self.values
            .iter_mut()
            .enumerate()
            .map(move |(offset, value)| (K::from(start + offset), value))
    }
}

#[cfg(test)]
mod tests {
    use core::mem::MaybeUninit;

    use super::{BorrowedStorage, InlineStorage, Storage, TinyMapBuilder};
    use crate::{Key, TinyMapError};

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct TwoKey(usize);

    impl From<usize> for TwoKey {
        fn from(value: usize) -> Self {
            Self(value)
        }
    }

    impl Key for TwoKey {
        const MAX_LEN: usize = 2;

        fn index(&self) -> usize {
            self.0
        }
    }

    struct EmptyExact;

    impl Iterator for EmptyExact {
        type Item = u16;

        fn next(&mut self) -> Option<Self::Item> {
            None
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (1, Some(1))
        }
    }

    impl ExactSizeIterator for EmptyExact {
        fn len(&self) -> usize {
            1
        }
    }

    fn exercise_dense_lifecycle<B: Storage<u16>>(mut map: TinyMapBuilder<TwoKey, u16, B>) {
        let first = map.try_insert(10).unwrap();
        assert_eq!(
            map.try_extend_exact(EmptyExact),
            Err(TinyMapError::ExactLength {
                expected: 1,
                actual: 0
            })
        );
        assert_eq!(map.len(), 1);

        let tail = map.try_extend_exact([20].into_iter()).unwrap();
        assert_eq!(
            map.try_insert(30),
            Err(TinyMapError::Capacity {
                limit: 2,
                requested: 3
            })
        );

        let mut sealed = map.seal();
        assert_eq!(sealed.as_ref().get(first), Some(&10));
        assert_eq!(
            sealed.as_ref().get_span(tail).unwrap().start_key(),
            Some(TwoKey(1))
        );
        *sealed.as_mut().get_mut(TwoKey(1)).unwrap() = 21;
        assert_eq!(sealed.as_ref().get(TwoKey(1)), Some(&21));
    }

    #[test]
    fn inline_backing_passes_the_dense_lifecycle_suite() {
        exercise_dense_lifecycle(TinyMapBuilder::<TwoKey, u16, InlineStorage<u16, 3>>::inline());
    }

    #[test]
    fn caller_backing_passes_the_dense_lifecycle_suite() {
        let mut slots = [const { MaybeUninit::<u16>::uninit() }; 3];
        exercise_dense_lifecycle(TinyMapBuilder::<TwoKey, u16, _>::borrowed(
            BorrowedStorage::new(&mut slots),
        ));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn heap_backing_passes_the_dense_lifecycle_suite() {
        exercise_dense_lifecycle(TinyMapBuilder::<TwoKey, u16, _>::heap());
    }
}
