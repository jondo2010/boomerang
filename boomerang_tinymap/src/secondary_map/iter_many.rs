use super::*;

use std::marker::PhantomData;

pub struct IterManyMut<'a, K: Key, V, I>
where
    I: Iterator<Item = K>,
{
    ptr: *mut Option<V>,
    keys: I,
    _marker: PhantomData<&'a mut V>,
}

impl<'a, K: Key, V, I> IterManyMut<'a, K, V, I>
where
    I: Iterator<Item = K>,
{
    pub(crate) fn new(ptr: *mut Option<V>, keys: I) -> Self {
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
            let ptr = self.ptr.add(key.index());
            (*ptr).as_mut().unwrap()
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

impl<K: Key, V> TinySecondaryMap<K, V> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultKey;

    #[test]
    fn test_iter_many_unchecked_mut() {
        let mut map = TinySecondaryMap::<DefaultKey, _>::with_capacity(10);
        map.insert(DefaultKey(3), 4);
        map.insert(DefaultKey(0), 1);
        map.insert(DefaultKey(2), 3);
        map.insert(DefaultKey(1), 2);

        let keys = [DefaultKey(0), DefaultKey(2), DefaultKey(3)];
        let mut values = unsafe { map.iter_many_unchecked_mut(keys.iter().copied()) };
        assert_eq!(values.len(), 3);
        assert_eq!(values.next(), Some(&mut 1));
        assert_eq!(values.next(), Some(&mut 3));
        assert_eq!(values.next(), Some(&mut 4));
    }
}
