use std::marker::PhantomData;

use crate::{map, Key, TinyMap};

/// `Chunks` is an iterator over slices of a given owned data buffer. Each call to `next` returns a
/// slice of the data buffer, starting at the index specified by the next element of the given
/// indices iterator.
pub struct Chunks<'a, K: Key, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
{
    ptr: *const V,
    keys: IO,
    _marker: PhantomData<&'a V>,
}

unsafe impl<'a, K: Key, V, IO, II> Send for Chunks<'a, K, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
{
}

impl<'a, K, V, IO, II> Iterator for Chunks<'a, K, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
    K: Key,
{
    type Item = map::IterMany<'a, K, V, II>;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys
            .next()
            .map(|keys| map::IterMany::new(self.ptr, keys))
    }
}

/// `ChunkMut` is a mutable version of [`Chunks`].
pub struct ChunksMut<'a, K: Key, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
{
    ptr: *mut V,
    keys: IO,
    _marker: PhantomData<&'a mut V>,
}

unsafe impl<'a, K: Key, V, IO, II> Send for ChunksMut<'a, K, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
{
}

impl<'a, K, V, IO, II> Iterator for ChunksMut<'a, K, V, IO, II>
where
    IO: Iterator<Item = II> + Send,
    II: Iterator<Item = K> + Send,
    K: Key,
{
    type Item = map::IterManyMut<'a, K, V, II>;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys
            .next()
            .map(|keys| map::IterManyMut::new(self.ptr, keys))
    }
}

pub type SplitChunks<'a, K, V, IO1, IO2, II> =
    (Chunks<'a, K, V, IO1, II>, ChunksMut<'a, K, V, IO2, II>);

impl<K: Key, V: Send> TinyMap<K, V> {
    /// Returns a tuple of two iterators over chunks of the map's data buffer. The first iterator
    /// yields immutable slices of the data buffer, while the second iterator yields mutable slices
    /// of the data buffer. The keys for each chunk are provided by the given iterators `keys` and
    /// `keys_mut`, respectively.
    ///
    /// # Safety
    /// - `keys` and `keys_mut` must not overlap with each other.
    /// - the keys in `keys_mut` must not repeat or overlap with itself.
    pub unsafe fn iter_chunks_split_unchecked<IO1, IO2, II>(
        &mut self,
        keys: IO1,
        keys_mut: IO2,
    ) -> SplitChunks<'_, K, V, IO1, IO2, II>
    where
        IO1: Iterator<Item = II> + Clone + Send,
        IO2: Iterator<Item = II> + Clone + Send,
        II: Iterator<Item = K> + Clone + Send,
    {
        (
            Chunks {
                ptr: self.data.as_ptr(),
                keys,
                _marker: PhantomData,
            },
            ChunksMut {
                ptr: self.data.as_mut_ptr(),
                keys: keys_mut,
                _marker: PhantomData,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultKey;

    /// Make a map containing `N` elements and a vector of keys.
    fn make_map<const N: usize>() -> (TinyMap<DefaultKey, usize>, Vec<DefaultKey>) {
        let map = (0..N).collect();
        let keys = (0..N).map(DefaultKey::from).collect();
        (map, keys)
    }

    #[test]
    fn test_par_iter_chunks_split_unchecked() {
        use itertools::Itertools;

        let (mut map, keys) = make_map::<20>();

        let chunked_keys = [
            // Even keys 0,2,4,6
            keys.iter().step_by(2).take(4).copied().collect_vec(),
            // Odd keys 1,3,5,7
            keys.iter()
                .skip(1)
                .step_by(2)
                .take(4)
                .copied()
                .collect_vec(),
        ];
        let keys_iter = chunked_keys.iter().map(|c| c.iter().copied());

        let (_, mut op) = map.iter_chunks_split_unchecked(std::iter::empty(), keys_iter);

        let chunk1 = op.next().unwrap();
        let chunk2 = op.next().unwrap();

        std::thread::scope(|s| {
            let v1 = s.spawn(move || chunk1.map(|x| *x).collect_vec());
            let v2 = chunk2.map(|x| *x).collect_vec();
            let v1 = v1.join().unwrap();

            assert_eq!(v1, vec![0, 2, 4, 6]);
            assert_eq!(v2, vec![1, 3, 5, 7]);
        });
    }

    #[test]
    fn test_iter_chunks_split_unchecked() {
        let (mut map, keys) = make_map::<6>();
        let c0 = vec![keys[0], keys[5]];
        let c1 = vec![keys[2], keys[1], keys[0]];
        let keys_select = [c0, c1];
        let keys_iter = keys_select.iter().map(|c| c.iter().copied());

        let (_ip, mut op) =
            unsafe { map.iter_chunks_split_unchecked(std::iter::empty(), keys_iter) };

        let o0 = op.next().unwrap().map(|x| *x).collect::<Vec<_>>();
        let o1 = op.next().unwrap().map(|x| *x).collect::<Vec<_>>();

        assert_eq!(o0, vec![0, 5]);
        assert_eq!(o1, vec![2, 1, 0]);
        assert!(op.next().is_none());
    }
}
