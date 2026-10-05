use core::mem::MaybeUninit;
use std::{cell::Cell, rc::Rc};

use boomerang_tinymap::{
    key_type, BorrowedSecondaryStorage, IndexSpan, InlineSecondaryStorage, InlineStorage,
    TinyMapError, TinySecondaryMapBuilder, TinySecondaryMapRef,
};

key_type!(SlotKey);

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct TwoKey(usize);

impl From<usize> for TwoKey {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl boomerang_tinymap::Key for TwoKey {
    const MAX_LEN: usize = 2;

    fn index(&self) -> usize {
        self.0
    }
}

struct DropCounter(Rc<Cell<usize>>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn inline_sparse_map_preserves_absence_and_replacement_across_sealing() {
    let mut builder =
        TinySecondaryMapBuilder::<SlotKey, u16, InlineSecondaryStorage<u16, 4>>::inline();

    assert_eq!(builder.domain_len(), 4);
    assert_eq!(builder.len(), 0);
    assert_eq!(builder.try_insert(SlotKey::new(3), 30), Ok(None));
    assert_eq!(builder.try_insert(SlotKey::new(1), 10), Ok(None));
    assert_eq!(builder.try_insert(SlotKey::new(3), 31), Ok(Some(30)));
    assert_eq!(builder.len(), 2);

    let sealed = builder.seal();
    let view = sealed.as_ref();
    assert_eq!(view.domain_len(), 4);
    assert_eq!(view.len(), 2);
    assert_eq!(view.get(SlotKey::new(0)), None);
    assert_eq!(view.get(SlotKey::new(1)), Some(&10));
    assert_eq!(view.get(SlotKey::new(3)), Some(&31));
}

fn assert_sparse_contents(view: TinySecondaryMapRef<'_, SlotKey, u16>) {
    assert_eq!(view.domain_len(), 3);
    assert_eq!(view.len(), 2);
    assert_eq!(view.get(SlotKey::new(0)), Some(&5));
    assert_eq!(view.get(SlotKey::new(1)), None);
    assert_eq!(view.get(SlotKey::new(2)), Some(&7));
}

#[test]
fn caller_and_heap_backings_share_the_sparse_slot_contract() {
    let mut slots: [MaybeUninit<Option<u16>>; 3] = core::array::from_fn(|_| MaybeUninit::uninit());
    let mut caller = TinySecondaryMapBuilder::<SlotKey, u16, _>::borrowed(
        BorrowedSecondaryStorage::new(&mut slots),
    );
    caller.try_insert(SlotKey::new(2), 7).unwrap();
    caller.try_insert(SlotKey::new(0), 5).unwrap();
    assert_eq!(
        caller.try_insert(SlotKey::new(3), 9),
        Err(TinyMapError::Capacity {
            limit: 3,
            requested: 4,
        })
    );
    assert_sparse_contents(caller.seal().as_ref());

    let mut heap = TinySecondaryMapBuilder::<SlotKey, u16, _>::try_heap(3).unwrap();
    heap.try_insert(SlotKey::new(2), 7).unwrap();
    heap.try_insert(SlotKey::new(0), 5).unwrap();
    assert_sparse_contents(heap.seal().as_ref());
}

#[test]
fn shared_sparse_views_iterate_present_values_and_spans_keep_global_keys() {
    let mut builder =
        TinySecondaryMapBuilder::<SlotKey, u16, InlineStorage<Option<u16>, 5>>::inline();
    builder.try_insert(SlotKey::new(4), 40).unwrap();
    builder.try_insert(SlotKey::new(1), 10).unwrap();
    builder.try_insert(SlotKey::new(3), 30).unwrap();

    let sealed = builder.seal();
    let view = sealed.as_ref();
    let copied = view;
    assert_eq!(
        copied.iter().collect::<Vec<_>>(),
        vec![
            (SlotKey::new(1), &10),
            (SlotKey::new(3), &30),
            (SlotKey::new(4), &40),
        ]
    );
    assert_eq!(
        view.keys().collect::<Vec<_>>(),
        vec![SlotKey::new(1), SlotKey::new(3), SlotKey::new(4)]
    );
    assert_eq!(view.values().collect::<Vec<_>>(), vec![&10, &30, &40]);
    assert_eq!(view.first_key(), Some(SlotKey::new(1)));

    let span = view.get_span(IndexSpan::new(1, 3)).unwrap();
    assert_eq!(span.start_key(), Some(SlotKey::new(1)));
    assert_eq!(span.len(), 3);
    assert_eq!(span.get(SlotKey::new(1)), Some(&10));
    assert_eq!(span.get(SlotKey::new(2)), None);
    assert_eq!(span.get(SlotKey::new(3)), Some(&30));
    assert_eq!(
        span.iter().collect::<Vec<_>>(),
        vec![(SlotKey::new(1), &10), (SlotKey::new(3), &30)]
    );
    assert!(view.get_span(IndexSpan::new(4, 2)).is_none());
}

#[test]
fn mutable_sparse_spans_change_values_without_changing_presence() {
    let mut builder =
        TinySecondaryMapBuilder::<SlotKey, u16, InlineStorage<Option<u16>, 6>>::inline();
    builder.try_insert(SlotKey::new(0), 10).unwrap();
    builder.try_insert(SlotKey::new(2), 20).unwrap();
    builder.try_insert(SlotKey::new(5), 50).unwrap();
    let mut sealed = builder.seal();

    {
        let mut view = sealed.as_mut();
        *view.get_mut(SlotKey::new(2)).unwrap() = 22;
        assert_eq!(view.get_mut(SlotKey::new(1)), None);

        let [mut left, mut right] = view
            .get_disjoint_spans_mut([IndexSpan::new(0, 3), IndexSpan::new(4, 2)])
            .unwrap();
        *left.get_mut(SlotKey::new(0)).unwrap() = 11;
        *right.get_mut(SlotKey::new(5)).unwrap() = 55;
    }

    {
        let mut view = sealed.as_mut();
        assert_eq!(
            view.get_disjoint_spans_mut([IndexSpan::new(1, 3), IndexSpan::new(2, 2)])
                .err()
                .unwrap(),
            TinyMapError::OverlappingSpans {
                first: 0,
                second: 1,
            }
        );
        assert_eq!(
            view.get_disjoint_spans_mut([IndexSpan::new(0, 1), IndexSpan::new(5, 2)])
                .err()
                .unwrap(),
            TinyMapError::InvalidSpan {
                start: 5,
                len: 2,
                domain_len: 6,
            }
        );
    }

    let view = sealed.as_ref();
    assert_eq!(view.len(), 3);
    assert_eq!(
        view.iter().collect::<Vec<_>>(),
        vec![
            (SlotKey::new(0), &11),
            (SlotKey::new(2), &22),
            (SlotKey::new(5), &55),
        ]
    );
}

#[test]
fn mutable_sparse_view_and_single_span_expose_only_present_values() {
    let mut builder =
        TinySecondaryMapBuilder::<SlotKey, u16, InlineStorage<Option<u16>, 4>>::inline();
    builder.try_insert(SlotKey::new(0), 10).unwrap();
    builder.try_insert(SlotKey::new(2), 20).unwrap();
    let mut sealed = builder.seal();

    let mut view = sealed.as_mut();
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());
    assert_eq!(view.get(SlotKey::new(0)), Some(&10));
    assert_eq!(view.get(SlotKey::new(1)), None);

    for (key, value) in view.iter_mut() {
        *value += key.as_u32() as u16;
    }
    assert_eq!(view.get(SlotKey::new(0)), Some(&10));
    assert_eq!(view.get(SlotKey::new(2)), Some(&22));

    let mut span = view.get_span_mut(IndexSpan::new(1, 3)).unwrap();
    assert_eq!(span.start_key(), Some(SlotKey::new(1)));
    assert_eq!(span.len(), 3);
    assert_eq!(span.get(SlotKey::new(1)), None);
    assert_eq!(span.get(SlotKey::new(2)), Some(&22));
    for (_, value) in span.iter_mut() {
        *value += 1;
    }
    assert_eq!(span.get(SlotKey::new(2)), Some(&23));
}

#[test]
fn sparse_domain_is_bounded_by_the_key_type_before_heap_initialization() {
    assert!(matches!(
        TinySecondaryMapBuilder::<TwoKey, u16, _>::try_heap(3),
        Err(TinyMapError::Capacity {
            limit: 2,
            requested: 3,
        })
    ));

    let inline = TinySecondaryMapBuilder::<TwoKey, u16, InlineSecondaryStorage<u16, 4>>::inline();
    assert_eq!(inline.domain_len(), 2);
}

#[test]
fn sparse_replacement_and_teardown_drop_each_owned_value_once() {
    let first_drops = Rc::new(Cell::new(0));
    let replacement_drops = Rc::new(Cell::new(0));
    let rejected_drops = Rc::new(Cell::new(0));
    let mut builder = TinySecondaryMapBuilder::<
        TwoKey,
        DropCounter,
        InlineSecondaryStorage<DropCounter, 2>,
    >::inline();

    builder
        .try_insert(TwoKey(1), DropCounter(first_drops.clone()))
        .unwrap();
    let replaced = builder
        .try_insert(TwoKey(1), DropCounter(replacement_drops.clone()))
        .unwrap();
    assert_eq!(first_drops.get(), 0);
    drop(replaced);
    assert_eq!(first_drops.get(), 1);

    assert!(builder
        .try_insert(TwoKey(2), DropCounter(rejected_drops.clone()))
        .is_err());
    assert_eq!(rejected_drops.get(), 1);

    drop(builder.seal());
    assert_eq!(replacement_drops.get(), 1);
    assert_eq!(first_drops.get(), 1);
    assert_eq!(rejected_drops.get(), 1);
}

#[test]
fn disjoint_sparse_spans_cover_empty_and_arithmetic_boundaries() {
    let mut builder =
        TinySecondaryMapBuilder::<SlotKey, u16, InlineSecondaryStorage<u16, 6>>::inline();
    builder.try_insert(SlotKey::new(0), 10).unwrap();
    builder.try_insert(SlotKey::new(4), 40).unwrap();
    let mut sealed = builder.seal();
    let mut view = sealed.as_mut();

    let none: [IndexSpan<SlotKey>; 0] = [];
    assert_eq!(view.get_disjoint_spans_mut(none).unwrap().len(), 0);

    {
        let [interior_a, interior_b, tail, mut high, mut low] = view
            .get_disjoint_spans_mut([
                IndexSpan::new(2, 0),
                IndexSpan::new(2, 0),
                IndexSpan::new(6, 0),
                IndexSpan::new(4, 1),
                IndexSpan::new(0, 1),
            ])
            .unwrap();
        assert!(interior_a.is_empty());
        assert!(interior_b.is_empty());
        assert!(tail.is_empty());
        assert_eq!(interior_a.start_key(), None);
        assert_eq!(tail.start_key(), None);
        *high.get_mut(SlotKey::new(4)).unwrap() = 44;
        *low.get_mut(SlotKey::new(0)).unwrap() = 11;
    }

    assert_eq!(
        view.get_disjoint_spans_mut([IndexSpan::new(7, 0)])
            .err()
            .unwrap(),
        TinyMapError::InvalidSpan {
            start: 7,
            len: 0,
            domain_len: 6,
        }
    );
    assert_eq!(
        view.get_disjoint_spans_mut([IndexSpan::new(usize::MAX, 1)])
            .err()
            .unwrap(),
        TinyMapError::InvalidSpan {
            start: usize::MAX,
            len: 1,
            domain_len: 6,
        }
    );
    assert_eq!(view.get(SlotKey::new(0)), Some(&11));
    assert_eq!(view.get(SlotKey::new(4)), Some(&44));
}
