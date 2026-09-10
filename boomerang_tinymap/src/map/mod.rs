//! A map that uses a custom key type to index its values.
//!
//! [`TinyMap`] is a map that uses a custom key type to index its values. It is more efficient than a
//! [`std::collections::HashMap`] or [`std::collections::BTreeMap`] as the keys are known at compile time and are small
//! integers.
//!
//! Key values are not created by the user, but are instead created by the `TinyMap` itself when
//! inserting values.
//!
//! # Examples
//!
//! ```
//! use boomerang_tinymap::{DefaultKey, TinyMap};
//!
//! let mut map = TinyMap::<DefaultKey, i32>::new();
//! let key1 = map.insert(10);
//! let key2 = map.insert(20);
//!
//! assert_eq!(map[key1], 10);
//! assert_eq!(map[key2], 20);
//! ```
use std::{
    fmt::{Debug, Display},
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::{IndexSpan, Key};

mod chunks;
mod iter_many;
mod view;

pub use chunks::{Chunks, ChunksMut, SplitChunks};
pub use iter_many::IterManyMut;
pub use view::TinyMapView;

/// A dense collection cannot represent the requested number of values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityError {
    max_len: usize,
    attempted_len: usize,
}

impl CapacityError {
    const fn new(max_len: usize, attempted_len: usize) -> Self {
        Self {
            max_len,
            attempted_len,
        }
    }

    /// Returns the greatest representable collection length.
    pub const fn max_len(self) -> usize {
        self.max_len
    }

    /// Returns the length requested by the failed operation.
    pub const fn attempted_len(self) -> usize {
        self.attempted_len
    }
}

impl Display for CapacityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "dense collection length {} exceeds key capacity {}",
            self.attempted_len, self.max_len
        )
    }
}

impl std::error::Error for CapacityError {}

/// A map that uses a custom key type to index its values.
///
/// See the [module-level documentation](index.html) for more information.
#[derive(Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TinyMap<K: Key, V> {
    pub(crate) data: Vec<V>,
    #[cfg_attr(feature = "serde", serde(skip))]
    _k: PhantomData<K>,
}

impl<K: Key + Debug, V: Debug> Debug for TinyMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Key + Display, V: Display> Display for TinyMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ty = std::any::type_name::<Self>();
        let vals = self
            .values()
            .map(|v| format!("{v}"))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "[{vals}].into_iter().collect::<{ty}>()")
    }
}

impl<K: Key, V> Default for TinyMap<K, V> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            _k: PhantomData,
        }
    }
}

#[derive(Debug)]
pub struct Iter<'a, K: Key, V> {
    inner: Enumerate<std::slice::Iter<'a, V>>,
    _k: PhantomData<K>,
}

#[derive(Debug)]
pub struct IntoIter<K: Key, V> {
    inner: Enumerate<std::vec::IntoIter<V>>,
    _k: PhantomData<K>,
}

impl<'a, K: Key, V> Iterator for Iter<'a, K, V> {
    type Item = (K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(index, value)| (K::from(index), value))
    }
}

impl<K: Key, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(index, value)| (K::from(index), value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K: Key, V> Index<K> for TinyMap<K, V> {
    type Output = V;

    fn index(&self, key: K) -> &Self::Output {
        &self.data[key.index()]
    }
}

impl<K: Key, V> IndexMut<K> for TinyMap<K, V> {
    fn index_mut(&mut self, key: K) -> &mut Self::Output {
        &mut self.data[key.index()]
    }
}

impl<K: Key, V> TinyMap<K, V> {
    /// Creates an empty `TinyMap`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an emtpy `TinyMap` with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            _k: PhantomData,
        }
    }

    /// Inserts a new value into the map and returns the key.
    pub fn insert(&mut self, value: V) -> K {
        self.try_insert(value)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Inserts a new value and returns its owner-generated key.
    ///
    /// Returns an error without modifying the map when the key domain is full.
    pub fn try_insert(&mut self, value: V) -> Result<K, CapacityError> {
        self.try_insert_with_key(|_| value)
    }

    /// Inserts a value created from its owner-generated key.
    pub fn insert_with_key<F>(&mut self, f: F) -> K
    where
        F: FnOnce(K) -> V,
    {
        self.try_insert_with_key(f)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Inserts a value created from its owner-generated key.
    ///
    /// The factory is not called when the key domain is full.
    pub fn try_insert_with_key<F>(&mut self, f: F) -> Result<K, CapacityError>
    where
        F: FnOnce(K) -> V,
    {
        let attempted_len = self.data.len().saturating_add(1);
        if attempted_len > K::MAX_LEN {
            return Err(CapacityError::new(K::MAX_LEN, attempted_len));
        }
        let key = K::from(self.data.len());
        self.data.push(f(key));
        Ok(key)
    }

    /// Appends an exact-size value segment and returns its generated key span.
    ///
    /// Capacity is checked before the map is modified.
    pub fn try_extend_exact<I>(&mut self, values: I) -> Result<IndexSpan<K>, CapacityError>
    where
        I: IntoIterator<Item = V>,
        I::IntoIter: ExactSizeIterator,
    {
        let values = values.into_iter();
        let start = self.data.len();
        let additional = values.len();
        let attempted_len = start.saturating_add(additional);
        if start.checked_add(additional).is_none() || attempted_len > K::MAX_LEN {
            return Err(CapacityError::new(K::MAX_LEN, attempted_len));
        }
        self.data.extend(values);
        Ok(IndexSpan::new(start, additional))
    }

    /// Collects values into a dense map while enforcing the key capacity.
    pub fn try_from_iter<I>(values: I) -> Result<Self, CapacityError>
    where
        I: IntoIterator<Item = V>,
    {
        let values = values.into_iter();
        let mut map = Self::with_capacity(values.size_hint().0.min(K::MAX_LEN));
        for value in values {
            map.try_insert(value)?;
        }
        Ok(map)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = K> {
        (0..self.data.len()).map(K::from)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.data.iter()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.data.iter_mut()
    }

    /// Returns an iterator over the (`K`, `V`) entries in the map.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            inner: self.data.iter().enumerate(),
            _k: PhantomData,
        }
    }

    /// Returns a reference to the value corresponding to the key.
    pub fn get(&self, key: K) -> Option<&V> {
        self.data.get(key.index())
    }

    /// Returns the values in an owner-allocated key span.
    pub fn get_span(&self, span: IndexSpan<K>) -> Option<&[V]> {
        self.data.get(span.indices()?)
    }

    /// Borrows this map as an allocation-free typed view.
    pub fn as_view(&self) -> TinyMapView<'_, K, V> {
        TinyMapView::new(&self.data)
    }
}

impl<K: Key, V> FromIterator<V> for TinyMap<K, V> {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        Self::try_from_iter(iter).unwrap_or_else(|error| panic!("{error}"))
    }
}

impl<K: Key, V> IntoIterator for TinyMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.data.into_iter().enumerate(),
            _k: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{key_type, DefaultKey, Key};

    use super::*;

    key_type!(pub TestKey);

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct TwoKey(u8);

    impl From<usize> for TwoKey {
        fn from(value: usize) -> Self {
            Self(value.try_into().expect("test key index fits u8"))
        }
    }

    impl Key for TwoKey {
        const MAX_LEN: usize = 2;

        fn index(&self) -> usize {
            usize::from(self.0)
        }
    }

    #[test]
    fn fallible_insertion_stops_before_exceeding_the_key_domain() {
        let mut map = TinyMap::<TwoKey, _>::new();

        assert_eq!(map.try_insert(10).unwrap(), TwoKey(0));
        assert_eq!(map.try_insert(20).unwrap(), TwoKey(1));
        let error = map.try_insert(30).unwrap_err();

        assert_eq!(error.max_len(), 2);
        assert_eq!(error.attempted_len(), 3);
        assert_eq!(map.values().copied().collect::<Vec<_>>(), vec![10, 20]);
    }

    #[test]
    fn failed_key_aware_insertion_does_not_call_the_value_factory() {
        let mut map = TinyMap::<TwoKey, _>::try_from_iter([10, 20]).unwrap();
        let called = Cell::new(false);

        let error = map
            .try_insert_with_key(|_| {
                called.set(true);
                30
            })
            .unwrap_err();

        assert_eq!(error.attempted_len(), 3);
        assert!(!called.get());
    }

    #[test]
    fn exact_extension_is_atomic_and_returns_the_allocated_span() {
        let mut map = TinyMap::<TwoKey, _>::new();

        let span = map.try_extend_exact([10, 20]).unwrap();
        assert_eq!(span, crate::IndexSpan::new(0, 2));

        let mut map = TinyMap::<TwoKey, _>::new();
        map.try_insert(10).unwrap();
        let error = map.try_extend_exact([20, 30]).unwrap_err();
        assert_eq!(error.attempted_len(), 3);
        assert_eq!(map.values().copied().collect::<Vec<_>>(), vec![10]);
    }

    #[test]
    fn fallible_collection_rejects_an_unrepresentable_dense_table() {
        let error = TinyMap::<TwoKey, _>::try_from_iter([10, 20, 30]).unwrap_err();

        assert_eq!(error.max_len(), 2);
        assert_eq!(error.attempted_len(), 3);
    }

    #[test]
    fn test_insert_and_get() {
        let mut map = TinyMap::<TestKey, _>::new();
        let key1 = map.insert(10);
        let key2 = map.insert(20);

        assert_eq!(key1, TestKey::new(0));
        assert_eq!(key2.as_u32(), 1);
        assert_eq!(map[key1], 10);
        assert_eq!(map[key2], 20);

        assert_eq!(map.get(key1), Some(&10));
        assert_eq!(map.get(key2), Some(&20));
        assert_eq!(map.get(TestKey::from(2)), None);
    }

    #[test]
    fn owned_map_exposes_a_borrowed_typed_view() {
        let map = [10, 20, 30].into_iter().collect::<TinyMap<DefaultKey, _>>();

        let view = map.as_view();

        assert_eq!(view[DefaultKey(1)], 20);
        assert_eq!(view.get(DefaultKey(3)), None);
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut map = TinyMap::<TestKey, i32>::default();
        assert_eq!(map.len(), 0);
        assert!(map.is_empty());

        map.insert(10);
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
    }

    #[test]
    fn test_keys() {
        let mut map = TinyMap::<TestKey, i32>::default();
        let key0 = map.insert(10);
        let key1 = map.insert(20);

        let keys: Vec<_> = map.keys().collect();
        assert_eq!(keys, vec![key0, key1]);
    }

    #[test]
    fn test_values() {
        let mut map = TinyMap::<TestKey, i32>::default();
        map.insert(10);
        map.insert(20);

        let values: Vec<_> = map.values().collect();
        assert_eq!(values, vec![&10, &20]);
    }

    #[test]
    fn test_values_mut() {
        let mut map = TinyMap::<TestKey, i32>::default();
        let key0 = map.insert(10);
        let key1 = map.insert(20);

        for value in map.values_mut() {
            *value *= 2;
        }

        assert_eq!(map[key0], 20);
        assert_eq!(map[key1], 40);
    }

    #[test]
    fn test_iter() {
        let mut map = TinyMap::<TestKey, i32>::default();
        let key0 = map.insert(10);
        let key1 = map.insert(20);

        let entries: Vec<_> = map.iter().collect();
        assert_eq!(entries, vec![(key0, &10), (key1, &20)]);
    }

    #[test]
    fn test_from_iter() {
        let values = vec![10, 20, 30];
        let map: TinyMap<TestKey, _> = values.into_iter().collect();

        assert_eq!(map[TestKey::from(0)], 10);
        assert_eq!(map[TestKey::from(1)], 20);
        assert_eq!(map[TestKey::from(2)], 30);
    }

    #[test]
    fn test_into_iter() {
        let mut map = TinyMap::<TestKey, i32>::default();
        map.insert(10);
        map.insert(20);

        let entries: Vec<_> = map.into_iter().collect();
        assert_eq!(
            entries,
            vec![(TestKey::from(0), 10), (TestKey::from(1), 20)]
        );
    }
}
