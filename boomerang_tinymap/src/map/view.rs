use core::{marker::PhantomData, ops::Index};

use crate::{Key, TableRange};

/// An allocation-free borrowed view of a densely keyed value table.
#[derive(Clone, Copy, Debug)]
pub struct TinyMapView<'a, K: Key, V> {
    data: &'a [V],
    _key: PhantomData<K>,
}

impl<'a, K: Key, V> TinyMapView<'a, K, V> {
    /// Creates a view over `data`.
    ///
    /// Panics when `data` exceeds the key type's supported table length.
    pub const fn new(data: &'a [V]) -> Self {
        assert!(data.len() <= K::MAX_LEN, "dense view exceeds key domain");
        Self {
            data,
            _key: PhantomData,
        }
    }

    /// Returns the number of values in this view.
    pub const fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns whether this view has no values.
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the value associated with `key`, if it is in range.
    pub fn get(&self, key: K) -> Option<&V> {
        self.data.get(key.index())
    }

    /// Returns the values in `range`, or `None` when it exceeds this view.
    pub fn get_range(&self, range: TableRange<K>) -> Option<&'a [V]> {
        self.data.get(range.indices()?)
    }

    /// Iterates over the values in dense key order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.data.iter()
    }

    /// Iterates over the keys in dense key order.
    pub fn keys(&self) -> impl Iterator<Item = K> {
        (0..self.len()).map(K::from)
    }

    /// Iterates over `(key, value)` pairs in dense key order.
    pub fn iter(&self) -> impl Iterator<Item = (K, &V)> {
        self.data
            .iter()
            .enumerate()
            .map(|(index, value)| (K::from(index), value))
    }
}

impl<K: Key, V> Index<K> for TinyMapView<'_, K, V> {
    type Output = V;

    fn index(&self, key: K) -> &Self::Output {
        &self.data[key.index()]
    }
}

#[cfg(test)]
mod tests {
    use crate::{map::TinyMapView, Key, TableRange};

    crate::key_type!(TestKey);

    const _: () = assert!(TableRange::<TestKey>::new(u32::MAX, 1).contains(u32::MAX));

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct ManualKey(usize);

    impl From<usize> for ManualKey {
        fn from(value: usize) -> Self {
            Self(value)
        }
    }

    impl Key for ManualKey {
        fn index(&self) -> usize {
            self.0
        }
    }

    static VALUES: [u16; 3] = [10, 20, 30];
    const VIEW: TinyMapView<'static, TestKey, u16> = TinyMapView::new(&VALUES);

    #[test]
    fn borrowed_view_looks_up_values_in_dense_key_order() {
        assert_eq!(VIEW.len(), 3);
        assert!(!VIEW.is_empty());
        assert_eq!(VIEW.get(TestKey::new(1)), Some(&20));
        assert_eq!(VIEW[TestKey::new(2)], 30);
        assert_eq!(VIEW.get(TestKey::new(3)), None);
        assert_eq!(
            VIEW.keys().collect::<Vec<_>>(),
            vec![TestKey::new(0), TestKey::new(1), TestKey::new(2)]
        );
        assert_eq!(
            VIEW.iter().collect::<Vec<_>>(),
            vec![
                (TestKey::new(0), &10),
                (TestKey::new(1), &20),
                (TestKey::new(2), &30),
            ]
        );
        assert_eq!(VIEW.values().collect::<Vec<_>>(), vec![&10, &20, &30]);
    }

    #[test]
    fn borrowed_view_returns_checked_table_ranges() {
        assert_eq!(VIEW.get_range(TableRange::new(1, 2)), Some(&VALUES[1..3]));
        assert_eq!(VIEW.get_range(TableRange::new(2, 2)), None);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            TableRange::<TestKey>::new(u32::MAX, 1)
                .checked_end()
                .map(|end| end as u64),
            Some(u64::from(u32::MAX) + 1)
        );
        let terminal = TableRange::<TestKey>::new(u32::MAX, 1);
        assert!(terminal.contains(u32::MAX));
        assert!(!terminal.contains(u32::MAX - 1));
    }

    #[test]
    fn dense_keys_are_u32_backed() {
        let key = TestKey::new(42);

        assert_eq!(core::mem::size_of::<TestKey>(), 4);
        assert_eq!(key.as_u32(), 42);
        assert_eq!(TestKey::from(42_usize).as_u32(), 42);
    }

    #[test]
    fn manual_key_implementations_use_the_unbounded_default() {
        let view = TinyMapView::<ManualKey, _>::new(&VALUES);

        assert_eq!(view[ManualKey::from(1)], 20);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    #[should_panic(expected = "dense key index exceeds u32 range")]
    fn generated_keys_reject_indices_outside_u32() {
        let _ = TestKey::from(u32::MAX as usize + 1);
    }
}
