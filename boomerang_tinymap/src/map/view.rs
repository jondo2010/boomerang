use core::{marker::PhantomData, ops::Index};

use crate::Key;

/// An allocation-free borrowed view of a densely keyed value table.
#[derive(Clone, Copy)]
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
    use crate::{map::TinyMapView, Key};

    crate::key_type!(TestKey);

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
