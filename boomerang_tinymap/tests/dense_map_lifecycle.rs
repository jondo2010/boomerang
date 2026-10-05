use boomerang_tinymap::{
    key_type, BorrowedStorage, HeapSealedTinyMap, HeapTinyMapBuilder, IndexSpan, InlineStorage,
    Key, TinyMapBuilder, TinyMapError, TinyMapMut, TinyMapRef,
};
use core::mem::MaybeUninit;
use std::{cell::Cell, rc::Rc};

key_type!(DenseKey);

const VALUES: [u16; 2] = [10, 20];
const VIEW: TinyMapRef<'static, DenseKey, u16> = TinyMapRef::from_slice(&VALUES);

#[test]
fn heap_builder_updates_owner_issued_keys_and_transfers_sealed_values() {
    assert_eq!(VIEW.values().copied().collect::<Vec<_>>(), [10, 20]);

    let mut builder = HeapTinyMapBuilder::<DenseKey, u16>::heap();
    let first = builder.try_insert(10).unwrap();
    let second = builder.try_insert(20).unwrap();
    assert_eq!(builder.get(first), Some(&10));
    *builder.get_mut(second).unwrap() = 21;

    let sealed: HeapSealedTinyMap<DenseKey, u16> = builder.seal();
    assert_eq!(
        sealed
            .as_ref()
            .iter()
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
        [10, 21]
    );
    assert_eq!(sealed.into_values().collect::<Vec<_>>(), [10, 21]);
}

#[test]
fn sealed_heap_maps_forward_value_traits() {
    let mut builder = HeapTinyMapBuilder::<DenseKey, u16>::heap();
    builder.try_insert(10).unwrap();
    builder.try_insert(20).unwrap();
    let original = builder.seal();
    let mut copy = original.clone();

    assert_eq!(original, copy);
    assert_eq!(format!("{original:?}"), format!("{copy:?}"));
    *copy.as_mut().get_mut(DenseKey::new(1)).unwrap() = 21;
    assert_ne!(original, copy);
    assert_eq!(
        original.as_ref().values().copied().collect::<Vec<_>>(),
        [10, 20]
    );
}

#[test]
fn consuming_heap_sealed_map_drops_each_value_once() {
    struct DropCounter(Rc<Cell<usize>>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    let first_drops = Rc::new(Cell::new(0));
    let second_drops = Rc::new(Cell::new(0));
    let mut builder = HeapTinyMapBuilder::<DenseKey, DropCounter>::heap();
    let _first = builder
        .try_insert(DropCounter(first_drops.clone()))
        .unwrap();
    let _second = builder
        .try_insert(DropCounter(second_drops.clone()))
        .unwrap();

    let mut values = builder.seal().into_values();
    let first = values.next().unwrap();
    assert_eq!((first_drops.get(), second_drops.get()), (0, 0));
    drop(values);
    assert_eq!((first_drops.get(), second_drops.get()), (0, 1));
    drop(first);
    assert_eq!((first_drops.get(), second_drops.get()), (1, 1));
}

#[test]
fn inline_dense_map_generates_keys_seals_moves_and_preserves_global_spans() {
    let mut builder = TinyMapBuilder::<DenseKey, u16, InlineStorage<u16, 4>>::inline();

    let first = builder.try_insert(10).unwrap();
    let tail = builder.try_extend_exact([20, 30].into_iter()).unwrap();
    assert_eq!(first, DenseKey::new(0));
    assert_eq!(tail.start(), 1);
    assert_eq!(tail.len(), 2);

    let mut sealed = builder.seal();
    assert_ref_shape(sealed.as_ref(), tail);

    let mut values = sealed.as_mut();
    assert_mut_shape(&mut values, first, tail);

    let moved = sealed;
    assert_eq!(moved.as_ref().get(first), Some(&11));
}

fn assert_ref_shape(
    view: TinyMapRef<'_, DenseKey, u16>,
    tail: boomerang_tinymap::IndexSpan<DenseKey>,
) {
    assert_eq!(view.get(DenseKey::new(0)), Some(&10));
    let tail = view.get_span(tail).unwrap();
    assert_eq!(tail.start_key(), Some(DenseKey::new(1)));
    assert_eq!(tail.values(), [20, 30]);
    assert_eq!(
        tail.iter().collect::<Vec<_>>(),
        [(DenseKey::new(1), &20), (DenseKey::new(2), &30)]
    );
    assert_eq!(
        view.iter().collect::<Vec<_>>(),
        vec![
            (DenseKey::new(0), &10),
            (DenseKey::new(1), &20),
            (DenseKey::new(2), &30),
        ]
    );
}

fn assert_mut_shape(
    view: &mut TinyMapMut<'_, DenseKey, u16>,
    first: DenseKey,
    tail: boomerang_tinymap::IndexSpan<DenseKey>,
) {
    *view.get_mut(first).unwrap() += 1;
    let mut tail = view.get_span_mut(tail).unwrap();
    assert_eq!(tail.start_key(), Some(DenseKey::new(1)));
    tail.values_mut().copy_from_slice(&[21, 31]);
    assert_eq!(view.values().copied().collect::<Vec<_>>(), vec![11, 21, 31]);
}

#[test]
fn caller_and_heap_backings_preserve_the_same_dense_key_contract() {
    let mut slots = [const { MaybeUninit::<u16>::uninit() }; 3];
    let mut caller = TinyMapBuilder::<DenseKey, u16, _>::borrowed(BorrowedStorage::new(&mut slots));
    let caller_span = caller.try_extend_exact([1, 2, 3].into_iter()).unwrap();
    assert_eq!(caller_span.start(), 0);
    assert_eq!(caller_span.len(), 3);
    assert_eq!(
        caller.seal().as_ref().values().copied().collect::<Vec<_>>(),
        [1, 2, 3]
    );

    let mut heap = TinyMapBuilder::<DenseKey, u16, _>::heap();
    let heap_span = heap.try_extend_exact([1, 2, 3].into_iter()).unwrap();
    assert_eq!(heap_span, caller_span);
    assert_eq!(
        heap.seal().as_ref().values().copied().collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TwoKey(usize);

impl From<usize> for TwoKey {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl Key for TwoKey {
    const MAX_LEN: usize = 2;

    fn index(&self) -> usize {
        self.0
    }
}

#[test]
fn borrowed_dense_slice_accepts_exact_key_domain_capacity_in_const_context() {
    const VALUES: [u8; TwoKey::MAX_LEN] = [10, 20];
    const VIEW: TinyMapRef<'static, TwoKey, u8> = TinyMapRef::from_slice(&VALUES);

    assert_eq!(VIEW.as_slice(), &VALUES);
}

#[test]
#[should_panic(expected = "dense view exceeds key domain")]
fn borrowed_dense_slice_rejects_values_beyond_key_domain() {
    let values = [10, 20, 30];
    let _ = TinyMapRef::<TwoKey, _>::from_slice(&values);
}

#[test]
fn key_domain_capacity_precedes_value_construction_and_preserves_shape() {
    let mut builder = TinyMapBuilder::<TwoKey, u8, InlineStorage<u8, 4>>::inline();
    builder.try_insert(1).unwrap();
    builder.try_insert(2).unwrap();

    let mut called = false;
    let error = builder
        .try_insert_with_key(|_| {
            called = true;
            3
        })
        .unwrap_err();

    assert!(!called);
    assert_eq!(
        error,
        TinyMapError::Capacity {
            limit: 2,
            requested: 3
        }
    );
    assert_eq!(
        builder
            .seal()
            .as_ref()
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

struct ShortExact {
    yielded: bool,
}

impl Iterator for ShortExact {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.yielded {
            None
        } else {
            self.yielded = true;
            Some(2)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (2, Some(2))
    }
}

impl ExactSizeIterator for ShortExact {
    fn len(&self) -> usize {
        2
    }
}

#[test]
fn failed_exact_extension_rolls_back_the_dense_shape() {
    let mut builder = TinyMapBuilder::<DenseKey, u8, InlineStorage<u8, 4>>::inline();
    let first = builder.try_insert(1).unwrap();

    assert_eq!(
        builder.try_extend_exact(ShortExact { yielded: false }),
        Err(TinyMapError::ExactLength {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(builder.len(), 1);
    assert_eq!(builder.seal().as_ref().get(first), Some(&1));
}

#[test]
fn disjoint_mutable_spans_are_checked_atomically_and_keep_global_keys() {
    let mut builder = TinyMapBuilder::<DenseKey, u8, InlineStorage<u8, 6>>::inline();
    builder
        .try_extend_exact([0, 1, 2, 3, 4, 5].into_iter())
        .unwrap();
    let mut sealed = builder.seal();
    let mut view = sealed.as_mut();

    {
        let [mut high, mut low] = view
            .get_disjoint_spans_mut([IndexSpan::new(4, 2), IndexSpan::new(1, 2)])
            .unwrap();
        assert_eq!(high.start_key(), Some(DenseKey::new(4)));
        assert_eq!(low.start_key(), Some(DenseKey::new(1)));
        high.values_mut().copy_from_slice(&[40, 50]);
        low.values_mut().copy_from_slice(&[10, 20]);
    }

    assert_eq!(
        view.values().copied().collect::<Vec<_>>(),
        [0, 10, 20, 3, 40, 50]
    );
    assert!(matches!(
        view.get_disjoint_spans_mut([IndexSpan::new(1, 3), IndexSpan::new(2, 2)]),
        Err(TinyMapError::OverlappingSpans {
            first: 0,
            second: 1
        })
    ));
    assert!(matches!(
        view.get_disjoint_spans_mut([IndexSpan::new(5, 2)]),
        Err(TinyMapError::InvalidSpan {
            start: 5,
            len: 2,
            domain_len: 6
        })
    ));

    let [interior_empty, enclosing] = view
        .get_disjoint_spans_mut([IndexSpan::new(2, 0), IndexSpan::new(1, 2)])
        .unwrap();
    assert_eq!(interior_empty.start_index(), 2);
    assert_eq!(interior_empty.start_key(), Some(DenseKey::new(2)));
    assert_eq!(enclosing.values(), [10, 20]);

    let [one_past_empty, whole] = view
        .get_disjoint_spans_mut([IndexSpan::new(6, 0), IndexSpan::new(0, 6)])
        .unwrap();
    assert_eq!(one_past_empty.start_index(), 6);
    assert_eq!(one_past_empty.start_key(), Some(DenseKey::new(6)));
    assert_eq!(whole.len(), 6);
}

#[test]
fn full_key_domain_empty_span_keeps_numeric_coordinate_without_fabricating_a_key() {
    let mut builder = TinyMapBuilder::<TwoKey, u8, InlineStorage<u8, 2>>::inline();
    builder.try_extend_exact([1, 2].into_iter()).unwrap();
    let sealed = builder.seal();
    let empty = sealed
        .as_ref()
        .get_span(IndexSpan::new(TwoKey::MAX_LEN, 0))
        .unwrap();

    assert_eq!(empty.start_index(), TwoKey::MAX_LEN);
    assert_eq!(empty.start_key(), None);
}

struct NonCopy;

#[test]
fn shared_dense_views_are_copyable_without_copyable_values() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<TinyMapRef<'static, DenseKey, NonCopy>>();
}
