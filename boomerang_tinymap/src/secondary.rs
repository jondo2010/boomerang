use std::{
    fmt::Debug,
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use super::Key;

#[derive(Clone)]
pub struct TinySecondaryMap<K: Key, V> {
    data: Vec<Option<V>>,
    num_values: usize,
    _k: PhantomData<K>,
}

impl<K: Key + Debug, V: Debug> Debug for TinySecondaryMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Key, V> Default for TinySecondaryMap<K, V> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            num_values: 0,
            _k: PhantomData,
        }
    }
}

impl<K: Key, V> Index<K> for TinySecondaryMap<K, V> {
    type Output = V;

    fn index(&self, key: K) -> &Self::Output {
        self.data[key.index()].as_ref().unwrap()
    }
}

impl<K: Key, V> IndexMut<K> for TinySecondaryMap<K, V> {
    fn index_mut(&mut self, key: K) -> &mut Self::Output {
        self.data[key.index()].as_mut().unwrap()
    }
}

#[derive(Debug)]
pub struct Iter<'a, K: Key, V: 'a> {
    values_left: usize,
    inner: Enumerate<core::slice::Iter<'a, Option<V>>>,
    _k: PhantomData<K>,
}

#[derive(Debug)]
pub struct IntoIter<K: Key, V> {
    values_left: usize,
    inner: Enumerate<std::vec::IntoIter<Option<V>>>,
    _k: PhantomData<(K, V)>,
}

impl<K: Key, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        for (idx, v) in self.inner.by_ref() {
            if let Some(v) = v {
                self.values_left -= 1;
                return Some((K::from(idx), v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.values_left, Some(self.values_left))
    }
}

impl<K: Key, V> ExactSizeIterator for IntoIter<K, V> {
    fn len(&self) -> usize {
        self.values_left
    }
}

impl<'a, K: Key, V> Iterator for Iter<'a, K, V> {
    type Item = (K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        for (idx, v) in self.inner.by_ref() {
            if let Some(v) = v {
                return Some((K::from(idx), v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.values_left, None)
    }
}

impl<K: Key, V> TinySecondaryMap<K, V> {
    /// Construct a new, empty [`TinySecondaryMap`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an emtpy `TinySecondaryMap` with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            num_values: 0,
            _k: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.num_values
    }

    pub fn is_empty(&self) -> bool {
        self.num_values == 0
    }

    /// Inserts or replaces a value into the secondary map at the given `key`. Returns [`None`] if
    /// the key was not present, otherwise returns the previous value.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.data
            .extend((self.data.len()..=key.index()).map(|_| None));
        if let Some(v) = &mut self.data[key.index()] {
            Some(std::mem::replace(v, value))
        } else {
            self.num_values += 1;
            self.data[key.index()] = Some(value);
            None
        }
    }

    pub fn contains_key(&self, key: K) -> bool {
        self.data.get(key.index()).map_or(false, Option::is_some)
    }

    /// Returns a reference to the value corresponding to the key.
    pub fn get(&self, key: K) -> Option<&V> {
        self.data.get(key.index())?.as_ref()
    }

    /// Returns a mutable reference to the value corresponding to the key.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        self.data.get_mut(key.index())?.as_mut()
    }

    /// Returns an iterator over the (`K`, `V`) entries in the map.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            inner: self.data.iter().enumerate(),
            values_left: self.num_values,
            _k: PhantomData,
        }
    }

    /// Returns an iterator over the keys in the map.
    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.data
            .iter()
            .enumerate()
            .filter_map(|(idx, v)| v.as_ref().map(|_| K::from(idx)))
    }

    /// Turns the map into a vector of the keys in the map.
    pub fn into_keys(self) -> Vec<K> {
        self.data
            .into_iter()
            .enumerate()
            .filter_map(|(idx, v)| v.map(|_| K::from(idx)))
            .collect()
    }

    /// Returns an iterator over the values in the map.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.data.iter().filter_map(Option::as_ref)
    }
}

impl<K: Key, V> Extend<(K, V)> for TinySecondaryMap<K, V> {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<K: Key, V> FromIterator<(K, V)> for TinySecondaryMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        map.extend(iter);
        map
    }
}

impl<K: Key, V> IntoIterator for TinySecondaryMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            values_left: self.num_values,
            inner: self.data.into_iter().enumerate(),
            _k: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::DefaultKey;

    use super::*;

    #[test]
    fn test_tiny_secondary_map() {
        let mut map = TinySecondaryMap::<DefaultKey, usize>::new();
        map.insert(DefaultKey(3), 4);
        map.insert(DefaultKey(0), 1);
        map.insert(DefaultKey(2), 3);
        map.insert(DefaultKey(1), 2);

        for i in 0..4 {
            assert_eq!(map.get(DefaultKey(i)), Some(&(i + 1)));
        }
    }
}
