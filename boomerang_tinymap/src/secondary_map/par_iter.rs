use rayon::iter::{plumbing::bridge, IntoParallelIterator};

use crate::Key;

use super::TinySecondaryMap;

/// Parallel iterator that moves out of a SecondaryMap.
#[derive(Debug, Clone)]
pub struct IntoIter<T: Send> {
    vec: Vec<T>,
}

impl<K: Key + Send, V: Send> IntoParallelIterator for TinySecondaryMap<K, V> {
    type Item = (K, V);
    type Iter = IntoIter<V>;

    fn into_par_iter(self) -> Self::Iter {
        IntoIter { vec: self }
    }
}

impl<T: Send> ParallelIterator for IntoIter<T> {
    type Item = T;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        bridge(self, consumer)
    }

    fn opt_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl<T: Send> IndexedParallelIterator for IntoIter<T> {
    fn drive<C>(self, consumer: C) -> C::Result
    where
        C: Consumer<Self::Item>,
    {
        bridge(self, consumer)
    }

    fn len(&self) -> usize {
        self.vec.len()
    }

    fn with_producer<CB>(mut self, callback: CB) -> CB::Output
    where
        CB: ProducerCallback<Self::Item>,
    {
        // Drain every item, and then the vector only needs to free its buffer.
        self.vec.par_drain(..).with_producer(callback)
    }
}
