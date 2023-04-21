/// Wrapper around [`slotmap::SlotMap`] and [`slotmap::SecondaryMap`] that implements [`Debug`] as a map.
pub struct DebugMap<'a, T>(pub &'a T);

impl<'a, K, V> std::fmt::Debug for DebugMap<'a, slotmap::SecondaryMap<K, V>>
where
    K: slotmap::Key + std::fmt::Debug,
    V: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

impl<'a, K, V> std::fmt::Debug for DebugMap<'a, slotmap::SlotMap<K, V>>
where
    K: slotmap::Key + std::fmt::Debug,
    V: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

trait TupleSlice {
    type Item;
    fn tuple_at_mut(&mut self, idxs: (usize, usize)) -> (&mut Self::Item, &mut Self::Item);
}

impl<T: Sized> TupleSlice for [T] {
    type Item = T;
    fn tuple_at_mut(&mut self, idx: (usize, usize)) -> (&mut Self::Item, &mut Self::Item) {
        let len = self.len();
        assert!(idx.0 != idx.1 && idx.0 <= len && idx.1 <= len);
        // SAFETY: [ptr; idx0] and [ptr; idx1] are non-overlapping and within `self`
        let ptr = self.as_mut_ptr();
        let slice = std::ptr::slice_from_raw_parts_mut(ptr, len);
        unsafe { (&mut (*slice)[idx.0], &mut (*slice)[idx.1]) }
    }
}
