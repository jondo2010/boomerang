//! A set of keys
//!
//! `TinySecondarySet` is more efficient than a [`super::TinySecondaryMap<K, ()>`] in memory and compute since it uses a
//! [`FixedBitSet`] to store the keys under the hood.
use std::{marker::PhantomData, ops::Index};

use fixedbitset::{FixedBitSet, Ones};

use super::Key;

/// A set of keys in a [`TinySecondaryMap`].
#[derive(Clone, Debug)]
pub struct TinySecondarySet<K: Key> {
    data: FixedBitSet,
    _k: PhantomData<K>,
}

impl<K: Key> TinySecondarySet<K> {
    /// Construct a new, empty [`TinySecondarySet`].
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: FixedBitSet::with_capacity(capacity),
            _k: PhantomData,
        }
    }

    /// Returns `true` if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_clear()
    }

    /// Insert a key into the set.
    #[inline]
    pub fn insert(&mut self, key: K) {
        self.data.set(key.index(), true);
    }

    /// Extend the set from an iterable.
    #[inline]
    pub fn extend(&mut self, keys: impl IntoIterator<Item = K>) {
        self.data.extend(keys.into_iter().map(|key| key.index()));
    }

    /// Clear the set.
    #[inline]
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Returns an iterator over the `K` entries in the map.
    pub fn iter(&self) -> Iter<'_, K> {
        Iter {
            inner: self.data.ones(),
            _k: PhantomData,
        }
    }
}

impl<K: Key + std::fmt::Display> std::fmt::Display for TinySecondarySet<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.iter().map(|k| k.to_string()))
            .finish()
    }
}

#[derive(Clone)]
pub struct Iter<'a, K: Key> {
    inner: Ones<'a>,
    _k: PhantomData<K>,
}

impl<'a, K: Key> Iterator for Iter<'a, K> {
    type Item = K;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|idx| K::from(idx))
    }
}

impl<'a, K: Key> ExactSizeIterator for Iter<'a, K> {
    fn len(&self) -> usize {
        self.inner.size_hint().1.unwrap()
    }
}

impl<K: Key> FromIterator<K> for TinySecondarySet<K> {
    fn from_iter<T: IntoIterator<Item = K>>(iter: T) -> Self {
        Self {
            data: iter.into_iter().map(|k| k.index()).collect(),
            _k: PhantomData,
        }
    }
}

impl<K: Key> Index<K> for TinySecondarySet<K> {
    type Output = bool;

    fn index(&self, key: K) -> &Self::Output {
        // Note: bits outside the capcity are always disabled, thus this will never panic
        &self.data[key.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultKey;

    #[test]
    fn test_new() {
        let set: TinySecondarySet<DefaultKey> = TinySecondarySet::with_capacity(10);
        assert!(set.is_empty());
    }

    #[test]
    fn test_insert_and_index() {
        let mut set = TinySecondarySet::with_capacity(10);
        let key1 = DefaultKey::from(0);
        let key2 = DefaultKey::from(1);

        set.insert(key1);
        assert!(set[key1]);
        assert!(!set[key2]);

        set.insert(key2);
        assert!(set[key1]);
        assert!(set[key2]);
    }

    #[test]
    fn test_iter() {
        let mut set = TinySecondarySet::with_capacity(10);
        set.insert(DefaultKey::from(0));
        set.insert(DefaultKey::from(2));
        set.insert(DefaultKey::from(4));

        let keys: Vec<DefaultKey> = set.iter().collect();
        assert_eq!(
            keys,
            vec![
                DefaultKey::from(0),
                DefaultKey::from(2),
                DefaultKey::from(4)
            ]
        );
    }

    #[test]
    fn test_from_iter() {
        let set: TinySecondarySet<DefaultKey> = vec![DefaultKey::from(0), DefaultKey::from(1)]
            .into_iter()
            .collect();
        assert_eq!(
            set.iter().collect::<Vec<DefaultKey>>(),
            vec![DefaultKey::from(0), DefaultKey::from(1)]
        );
    }

    #[test]
    fn test_empty_iter() {
        let set: TinySecondarySet<DefaultKey> = TinySecondarySet::with_capacity(10);
        assert!(set.iter().next().is_none());
    }
}
