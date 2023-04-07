use std::{
    fmt::Debug,
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::Key;

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

    // pub fn get_many_unchecked_mut(&mut self, keys: &[K]) -> Box<[&mut V]> {
    //    let ptr = self.data.as_mut_ptr();
    //    keys.iter()
    //        .map(|key| unsafe { ptr.offset(key.index() as isize).as_mut().unwrap() })
    //        .collect()
    //}

    /// Returns an iterator mutable references to the values corresponding to the keys.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the keys are valid and unique, otherwise the returned
    /// references will UB.
    pub fn iter_many_unchecked_mut<'a, I>(
        &'a mut self,
        keys: I,
    ) -> impl Iterator<Item = &mut V> + 'a
    where
        I: IntoIterator<Item = K>,
        <I as IntoIterator>::IntoIter: 'a,
    {
        let ptr = self.data.as_mut_ptr();
        keys.into_iter()
            .map(move |key| unsafe { ptr.add(key.index()).as_mut().unwrap() })
    }

    /// Returns an tuple of 2 iterators of the items in `keys` and `keys_mut`.
    /// The first iterator returns immutable references to the values, the second one mutable
    /// references.
    ///
    /// The caller must ensure that the keys are valid and unique, otherwise the returned
    /// references will UB.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// 1. `keys` and `keys_mut` are disjoint.
    /// 2. `keys_mut` does not contain any duplicates.
    ///
    /// Otherwise the returned references will UB.
    pub fn iter_many_unchecked_split<'a, I, IM>(
        &'a mut self,
        keys: I,
        keys_mut: IM,
    ) -> (
        impl Iterator<Item = &V> + 'a,
        impl Iterator<Item = &mut V> + 'a,
    )
    where
        I: IntoIterator<Item = K>,
        <I as IntoIterator>::IntoIter: 'a,
        IM: IntoIterator<Item = K>,
        <IM as IntoIterator>::IntoIter: 'a,
    {
        let ptr = self.data.as_mut_ptr();
        let iter = keys
            .into_iter()
            .map(move |key| unsafe { ptr.add(key.index()).as_ref().unwrap() });
        let iter_mut = keys_mut
            .into_iter()
            .map(move |key| unsafe { ptr.add(key.index()).as_mut().unwrap() });
        (iter, iter_mut)
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
    use crate::DefaultKey;

    use super::*;

    #[test]
    fn test_get_many_unchecked_mut() {
        let mut map = TinyMap::<DefaultKey, usize> {
            data: Vec::new(),
            _k: PhantomData,
        };
        let _k1 = map.insert(0);
        let k2 = map.insert(1);
        let k3 = map.insert(2);
        let k4 = map.insert(3);
        let k5 = map.insert(4);
        let k6 = map.insert(5);

        let values = map.iter_many_unchecked_mut([k4, k2, k6, k3, k5]).map(|x| {
            *x += 1;
            *x
        });

        assert_eq!(values.collect::<Vec<_>>(), vec![4, 2, 6, 3, 5]);
    }

    #[test]
    fn test_iter_many_unchecked_split() {
        let mut map = TinyMap::<DefaultKey, usize>::with_capacity(5);
        let k1 = map.insert(1);
        let k2 = map.insert(2);
        let k3 = map.insert(3);
        let k4 = map.insert(4);
        let k5 = map.insert(5);

        let (values, values_mut) = map.iter_many_unchecked_split([k3, k1, k5], [k2, k4]);

        assert_eq!(values.collect::<Vec<_>>(), vec![&3, &1, &5]);
        assert_eq!(values_mut.collect::<Vec<_>>(), vec![&mut 2, &mut 4]);
    }
}
