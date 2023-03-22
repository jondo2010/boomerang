use std::{borrow::Borrow, marker::PhantomData};

use crate::{Key, TinyMap};

/// `Chunks` is an iterator over slices of a given owned data buffer. Each call to `next` returns a
/// slice of the data buffer, starting at the index specified by the next element of the given
/// indices iterator.
pub struct Chunks<'a, T: 'a, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K>,
{
    data: Box<[T]>,
    offset: usize,
    indices: I,
    phantom: PhantomData<&'a T>,
}

impl<'a, T: 'a, I, K> Chunks<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    fn new(data: Box<[T]>, indices: I) -> Self {
        Self {
            data,
            offset: 0,
            indices,
            phantom: PhantomData,
        }
    }
}

impl<'a, T: 'a, I, K> Iterator for Chunks<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K>,
{
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        let width = self.indices.next();
        let remaining = self.data.len() - self.offset;
        match (width, remaining) {
            (Some(width), remaining) if remaining >= *width.borrow() => {
                let offset = self.offset as isize;
                self.offset += *width.borrow();
                Some(unsafe {
                    std::slice::from_raw_parts(self.data.as_ptr().offset(offset), *width.borrow())
                })
            }
            _ => None,
        }
    }
}

/// `ChunkMut` is a mutable version of [`Chunks`].
pub struct ChunksMut<'a, T, I, K>(Chunks<'a, T, I, K>)
where
    K: Borrow<usize>,
    I: Iterator<Item = K>;

impl<'a, T: 'a, I, K> ChunksMut<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K>,
{
    fn new(data: Box<[T]>, indices: I) -> Self {
        Self(Chunks {
            data,
            offset: 0,
            indices,
            phantom: PhantomData,
        })
    }
}

impl<'a, T: 'a + Send, I, K> Iterator for ChunksMut<'a, T, I, K>
where
    K: Borrow<usize>,
    I: Iterator<Item = K> + Send,
{
    type Item = &'a mut [T];

    fn next(&mut self) -> Option<Self::Item> {
        let slf = &mut self.0;
        let width = slf.indices.next();
        let remaining = slf.data.len() - slf.offset;
        match (width, remaining) {
            (Some(width), remaining) if remaining >= *width.borrow() => {
                let offset = slf.offset as isize;
                slf.offset += *width.borrow();
                Some(unsafe {
                    std::slice::from_raw_parts_mut(
                        slf.data.as_mut_ptr().offset(offset),
                        *width.borrow(),
                    )
                })
            }
            _ => None,
        }
    }
}

impl<K: Key, V: Send> TinyMap<K, V> {
    pub fn iter_chunks_split_unchecked<'a: 'keys, 'keys, IOuter1, IOuter2, IInner>(
        &'a mut self,
        keys: IOuter1,
        keys_mut: IOuter2,
    ) -> (
        Chunks<'a, &V, impl Iterator<Item = usize>, usize>,
        ChunksMut<'a, &mut V, impl Iterator<Item = usize>, usize>,
    )
    where
        // K: AsRef<T> + AsMut<T>,
        // T: ?Sized + Send + Sync,
        IOuter1: Iterator<Item = IInner> + Clone + Send,
        IOuter2: Iterator<Item = IInner> + Clone + Send,
        IInner: Iterator<Item = &'keys K> + ExactSizeIterator + Clone + Send,
    {
        let all_keys = keys.clone().flatten();
        let all_keys_mut = keys_mut.clone().flatten();

        let ptr = self.data.as_mut_ptr();

        let chunks = {
            let key_indices = keys.map(|inner| inner.len());
            Chunks::new(
                all_keys
                    .map(move |key| unsafe { ptr.add(key.index()).as_ref().unwrap() })
                    .collect(),
                key_indices,
            )
        };

        let chunks_mut = {
            let key_indices = keys_mut.map(|inner| inner.len());
            ChunksMut::new(
                all_keys_mut
                    .map(move |key| unsafe { ptr.add(key.index()).as_mut().unwrap() })
                    .collect(),
                key_indices,
            )
        };
        (chunks, chunks_mut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultKey;

    fn make_map<const N: usize>() -> (TinyMap<DefaultKey, usize>, Vec<DefaultKey>) {
        let map = (0..N).map(|i| i).collect();
        let keys = (0..N).map(DefaultKey::from).collect();
        (map, keys)
    }

    #[test]
    fn test_iter_chunks_split_unchecked() {
        let (mut map, keys) = make_map::<5>();

        let keys_select = vec![&keys[0..=1], &keys[2..=3]];
        let chunked_keys = keys_select.iter().map(|x| x.iter());

        let (_ip, mut op) = map.iter_chunks_split_unchecked(std::iter::empty(), chunked_keys);

        let o0 = op
            .next()
            .unwrap()
            .into_iter()
            .map(|x| **x)
            .collect::<Vec<_>>();

        let o1 = op
            .next()
            .unwrap()
            .into_iter()
            .map(|x| **x)
            .collect::<Vec<_>>();

        assert_eq!(o0, vec![0, 1]);
        assert_eq!(o1, vec![2, 3]);
        assert_eq!(op.next(), None);
    }
}
