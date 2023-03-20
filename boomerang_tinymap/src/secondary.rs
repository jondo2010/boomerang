use std::{
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use super::Key;

#[derive(Debug)]
pub struct TinySecondaryMap<K: Key, V> {
    data: Vec<Option<V>>,
    _k: PhantomData<K>,
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
    inner: Enumerate<core::slice::Iter<'a, Option<V>>>,
    _k: PhantomData<K>,
}

impl<'a, K: Key, V> Iterator for Iter<'a, K, V> {
    type Item = (K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((idx, v)) = self.inner.next() {
            if let Some(v) = v {
                return Some((K::from(idx), v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K: Key, V> TinySecondaryMap<K, V> {
    /// Construct a new, empty [`TinySecondaryMap`].
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            _k: PhantomData,
        }
    }

    /// Creates an emtpy `TinySecondaryMap` with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            _k: PhantomData,
        }
    }

    /// Inserts a value into the secondary map at the given `key`. Returns [`None`] if the key
    /// was not present, otherwise returns the previous value.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.data
            .extend((self.data.len()..=key.index()).map(|_| None));
        if let Some(v) = &mut self.data[key.index()] {
            Some(std::mem::replace(v, value))
        } else {
            self.data[key.index()] = Some(value);
            None
        }
    }

    pub fn extend(&mut self, iter: impl IntoIterator<Item = (K, V)>) {
        for (key, value) in iter {
            self.insert(key, value);
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

    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            inner: self.data.iter().enumerate(),
            _k: PhantomData,
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
