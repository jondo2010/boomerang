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
    fmt::Debug,
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::Key;

mod chunks;
mod iter_many;

pub use chunks::{Chunks, ChunksMut, SplitChunks};
pub use iter_many::IterManyMut;

/// A map that uses a custom key type to index its values.
///
/// See the [module-level documentation](index.html) for more information.
pub struct TinyMap<K: Key, V> {
    pub(crate) data: Vec<V>,
    _k: PhantomData<K>,
}

impl<K: Key + Debug, V: Debug> Debug for TinyMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
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

impl<'a, K: Key, V> Iterator for Iter<'a, K, V> {
    type Item = (K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(index, value)| (K::from(index), value))
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
        let key = K::from(self.data.len());
        self.data.push(value);
        key
    }

    pub fn insert_with_key<F>(&mut self, f: F) -> K
    where
        F: FnOnce(K) -> V,
    {
        let key = K::from(self.data.len());
        self.data.push(f(key));
        key
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
}

impl<K: Key, V> FromIterator<V> for TinyMap<K, V> {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        Self {
            data: iter.into_iter().collect(),
            _k: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::key_type;

    use super::*;

    key_type!(pub TestKey);

    #[test]
    fn test_insert_and_get() {
        let mut map = TinyMap::<TestKey, _>::new();
        let key1 = map.insert(10);
        let key2 = map.insert(20);

        assert_eq!(map[key1], 10);
        assert_eq!(map[key2], 20);
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
}
