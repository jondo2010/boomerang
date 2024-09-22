use std::marker::PhantomData;

use super::{Key, TinyMap};

pub struct IterMany<'a, K: Key, V, I>
where
    I: Iterator<Item = K>,
{
    ptr: *const V,
    keys: I,
    _marker: PhantomData<&'a V>,
}

impl<'a, K: Key, V, I> IterMany<'a, K, V, I>
where
    I: Iterator<Item = K>,
{
    pub(crate) fn new(ptr: *const V, keys: I) -> Self {
        Self {
            ptr,
            keys,
            _marker: PhantomData,
        }
    }
}

impl<'a, K: Key, V, I> Iterator for IterMany<'a, K, V, I>
where
    I: Iterator<Item = K>,
{
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.next().map(|key| unsafe {
            let ptr = self.ptr.wrapping_add(key.index());
            &*ptr
        })
    }
}

impl<'a, K: Key, V, I> ExactSizeIterator for IterMany<'a, K, V, I>
where
    I: Iterator<Item = K> + ExactSizeIterator,
{
    fn len(&self) -> usize {
        self.keys.len()
    }
}

unsafe impl<K: Key, V: Send, I: Iterator<Item = K>> Send for IterMany<'_, K, V, I> {}

pub struct IterManyMut<'a, K: Key, V, I>
where
    I: Iterator<Item = K>,
{
    ptr: *mut V,
    keys: I,
    _marker: PhantomData<&'a mut V>,
}

impl<'a, K: Key, V, I> IterManyMut<'a, K, V, I>
where
    I: Iterator<Item = K>,
{
    pub(crate) fn new(ptr: *mut V, keys: I) -> Self {
        Self {
            ptr,
            keys,
            _marker: PhantomData,
        }
    }
}

impl<'a, K: Key, V, I> Iterator for IterManyMut<'a, K, V, I>
where
    I: Iterator<Item = K>,
{
    type Item = &'a mut V;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.next().map(|key| unsafe {
            let ptr = self.ptr.wrapping_add(key.index());
            &mut *ptr
        })
    }
}

impl<'a, K: Key, V, I> ExactSizeIterator for IterManyMut<'a, K, V, I>
where
    I: Iterator<Item = K> + ExactSizeIterator,
{
    fn len(&self) -> usize {
        self.keys.len()
    }
}

unsafe impl<K: Key, V: Send, I: Iterator<Item = K>> Send for IterManyMut<'_, K, V, I> {}

impl<K: Key, V> TinyMap<K, V> {
    /// Returns an iterator of mutable references to the values corresponding to the keys.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the keys are valid and unique, otherwise the returned
    /// references will UB.
    pub unsafe fn iter_many_unchecked_mut<'a, I>(
        &'a mut self,
        keys: I,
    ) -> IterManyMut<'a, K, V, I::IntoIter>
    where
        I: IntoIterator<Item = K>,
        <I as IntoIterator>::IntoIter: 'a,
    {
        IterManyMut::new(self.data.as_mut_ptr(), keys.into_iter())
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
        impl Iterator<Item = &'a V> + 'a,
        impl Iterator<Item = &'a mut V> + 'a,
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

        let iter_mut = IterManyMut {
            ptr,
            keys: keys_mut.into_iter(),
            _marker: PhantomData,
        };

        (iter, iter_mut)
    }
}

#[cfg(test)]
mod tests {
    use crate::DefaultKey;

    use super::*;

    #[test]
    fn test_get_many_unchecked_mut() {
        let mut map = TinyMap::<DefaultKey, usize>::new();
        let _k1 = map.insert(0);
        let k2 = map.insert(1);
        let k3 = map.insert(2);
        let k4 = map.insert(3);
        let k5 = map.insert(4);
        let k6 = map.insert(5);

        let values = unsafe {
            map.iter_many_unchecked_mut([k4, k2, k6, k3, k5]).map(|x| {
                *x += 1;
                *x
            })
        };

        assert_eq!(values.collect::<Vec<_>>(), vec![4, 2, 6, 3, 5]);
    }

    #[test]
    fn test_get_many_unchecked_mut_send() {
        let mut map = TinyMap::<DefaultKey, usize>::new();
        let k1 = map.insert(0);
        let _k2 = map.insert(1);
        let k3 = map.insert(2);
        let _k4 = map.insert(3);
        let k5 = map.insert(4);
        let _k6 = map.insert(5);
        let keys = [k1, k3, k5];

        let map = std::thread::scope(|scope| {
            let thread = scope.spawn(move || {
                for v in unsafe { map.iter_many_unchecked_mut(keys) } {
                    *v += 1;
                }
                map
            });

            thread.join().unwrap()
        });

        assert_eq!(
            map.values().copied().collect::<Vec<_>>(),
            vec![1, 1, 3, 3, 5, 5]
        );
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

        let (_, values_mut) = map.iter_many_unchecked_split([], [k2, k4]);
        for v in values_mut {
            *v += 1;
        }
        assert_eq!(map.values().collect::<Vec<_>>(), vec![&1, &3, &3, &5, &5]);
    }
}
