//! A set of keys
//!
//! [`KeySet`] is more efficient than a [`super::TinySecondaryMap<K, ()>`] in memory and compute since it uses a
//! [`FixedBitSet`] to store the keys under the hood.
use std::{marker::PhantomData, ops::Index};

use fixedbitset::{FixedBitSet, Ones};

use super::Key;

/// A unique set of keys.
#[derive(Clone)]
pub struct KeySet<K: Key> {
    data: FixedBitSet,
    _k: PhantomData<K>,
}

impl<K: Key + std::fmt::Debug> std::fmt::Debug for KeySet<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<K: Key> KeySet<K> {
    /// Construct a new, empty [`KeySet`].
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
            count: self.data.count_ones(..),
            _k: PhantomData,
        }
    }
}

impl<K: Key + std::fmt::Display> std::fmt::Display for KeySet<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.iter().map(|k| k.to_string()))
            .finish()
    }
}

pub struct Iter<'a, K: Key> {
    inner: Ones<'a>,
    count: usize,
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
        self.count
    }
}

impl<K: Key> FromIterator<K> for KeySet<K> {
    fn from_iter<T: IntoIterator<Item = K>>(iter: T) -> Self {
        Self {
            data: iter.into_iter().map(|k| k.index()).collect(),
            _k: PhantomData,
        }
    }
}

impl<K: Key> Index<K> for KeySet<K> {
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
        let set: KeySet<DefaultKey> = KeySet::with_capacity(10);
        assert!(set.is_empty());
    }

    #[test]
    fn test_insert_and_index() {
        let mut set = KeySet::with_capacity(10);
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
        let mut set = KeySet::with_capacity(10);
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
        let set: KeySet<DefaultKey> = vec![DefaultKey::from(0), DefaultKey::from(1)]
            .into_iter()
            .collect();
        assert_eq!(
            set.iter().collect::<Vec<DefaultKey>>(),
            vec![DefaultKey::from(0), DefaultKey::from(1)]
        );
    }

    #[test]
    fn test_empty_iter() {
        let set: KeySet<DefaultKey> = KeySet::with_capacity(10);
        assert!(set.iter().next().is_none());
    }

    #[test]
    fn test_extend() {
        let mut set = KeySet::with_capacity(10);
        set.extend(vec![DefaultKey::from(0), DefaultKey::from(1)]);
        assert_eq!(
            set.iter().collect::<Vec<DefaultKey>>(),
            vec![DefaultKey::from(0), DefaultKey::from(1)]
        );
    }

    #[test]
    fn test_exact_size_iter() {
        let mut set = KeySet::with_capacity(10);
        set.extend(vec![DefaultKey::from(0), DefaultKey::from(1)]);
        assert_eq!(set.iter().len(), 2);
    }
}
