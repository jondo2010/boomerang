mod storage;

use core::{marker::PhantomData, mem::ManuallyDrop};

use crate::TinyMapError;

#[cfg(feature = "alloc")]
pub use storage::HeapStorage;
#[doc(hidden)]
pub use storage::Storage;
pub use storage::{BorrowedStorage, InlineStorage};

/// A not-yet-sealed sequence builder.
///
/// Values occupy the initialized prefix of its private backing storage. The
/// storage interface remains private so callers cannot observe or alter the
/// initialized-slot metadata. Inline and borrowed storage have fixed capacity;
/// heap storage may grow and allocate while values are appended.
pub struct TinyVecBuilder<T, B: Storage<T>> {
    backing: B,
    initialized: usize,
    marker: PhantomData<T>,
}

/// A fixed-shape sequence whose initialized values are ready for later phases.
///
/// A sealed sequence can move freely. It deliberately offers only borrowed
/// value views, not structural mutation.
pub struct SealedTinyVec<T, B: Storage<T>> {
    backing: B,
    initialized: usize,
    marker: PhantomData<T>,
}

/// A borrowed read-only view of the initialized values in a sealed TinyVec.
#[derive(Clone, Copy)]
pub struct TinyVecRef<'a, T> {
    values: &'a [T],
}

/// A borrowed value-mutation view of a sealed TinyVec.
///
/// It cannot append, remove, or otherwise change the sequence length.
pub struct TinyVecMut<'a, T> {
    values: &'a mut [T],
}

impl<T, B: Storage<T>> TinyVecBuilder<T, B> {
    /// Returns the number of values currently initialized in the builder.
    pub const fn len(&self) -> usize {
        self.initialized
    }

    /// Returns whether the builder contains no initialized values.
    pub const fn is_empty(&self) -> bool {
        self.initialized == 0
    }

    /// Transfers initialized values into a move-safe, fixed-shape sequence.
    pub fn seal(self) -> SealedTinyVec<T, B> {
        let builder = ManuallyDrop::new(self);
        // SAFETY: ManuallyDrop prevents TinyVecBuilder::drop from observing or
        // dropping this backing after ownership is transferred. ptr::read moves
        // the backing exactly once into SealedTinyVec; the initialized prefix
        // is unchanged, so its value-ownership metadata transfers intact.
        unsafe {
            SealedTinyVec {
                backing: core::ptr::read(&builder.backing),
                initialized: builder.initialized,
                marker: PhantomData,
            }
        }
    }

    /// Appends one value.
    ///
    /// Inline and borrowed storage append without allocating. Heap storage may
    /// grow its backing vector and allocate.
    pub fn try_push(&mut self, value: T) -> Result<(), TinyMapError> {
        let limit = self.backing.capacity();
        if self.initialized == limit {
            return Err(TinyMapError::Capacity {
                limit,
                requested: self.initialized.saturating_add(1),
            });
        }

        self.backing.write_slot(self.initialized, value);
        self.initialized += 1;
        Ok(())
    }

    /// Extends from an iterator that promises an exact length.
    ///
    /// The advertised length is checked before consuming the iterator. Any
    /// iterator length lie, returned error, or unwind restores the builder to
    /// its original initialized prefix.
    pub fn try_extend_exact<I>(&mut self, mut values: I) -> Result<(), TinyMapError>
    where
        I: ExactSizeIterator<Item = T>,
    {
        let expected = values.len();
        let limit = self.backing.capacity();
        if expected > limit.saturating_sub(self.initialized) {
            return Err(TinyMapError::Capacity {
                limit,
                requested: self.initialized.saturating_add(expected),
            });
        }

        let rollback = Rollback::new(self);
        for actual in 0..expected {
            let Some(value) = values.next() else {
                return Err(TinyMapError::ExactLength { expected, actual });
            };
            rollback.builder.try_push(value)?;
        }

        let mut actual = expected;
        for extra in values.by_ref() {
            actual = actual.saturating_add(1);
            drop(extra);
        }
        if actual != expected {
            return Err(TinyMapError::ExactLength { expected, actual });
        }

        drop(values);
        rollback.commit();
        Ok(())
    }

    fn drop_suffix_from(&mut self, original: usize) {
        drop_initialized::<T, _>(&mut self.backing, &mut self.initialized, original);
    }
}

impl<T, const N: usize> TinyVecBuilder<T, InlineStorage<T, N>> {
    /// Creates a builder backed by exactly `N` inline slots.
    pub fn inline() -> Self {
        new_builder(InlineStorage::new())
    }
}

impl<'a, T> TinyVecBuilder<T, BorrowedStorage<'a, T>> {
    /// Creates a builder backed by caller-provided slots.
    pub fn borrowed(backing: BorrowedStorage<'a, T>) -> Self {
        new_builder(backing)
    }
}

#[cfg(feature = "alloc")]
impl<T> TinyVecBuilder<T, HeapStorage<T>> {
    /// Creates an empty heap-backed builder.
    pub fn heap() -> Self {
        new_builder(HeapStorage::new())
    }
}

impl<T, B: Storage<T>> Drop for TinyVecBuilder<T, B> {
    fn drop(&mut self) {
        drop_initialized::<T, _>(&mut self.backing, &mut self.initialized, 0);
    }
}

impl<T, B: Storage<T>> SealedTinyVec<T, B> {
    /// Returns the number of fixed-shape values in this sealed sequence.
    pub const fn len(&self) -> usize {
        self.initialized
    }

    /// Returns whether the sealed sequence has no values.
    pub const fn is_empty(&self) -> bool {
        self.initialized == 0
    }

    /// Returns a read-only borrowed view of the initialized values.
    pub fn as_ref(&self) -> TinyVecRef<'_, T> {
        TinyVecRef {
            values: self.backing.values(self.initialized),
        }
    }

    /// Returns a borrowed view that may update values but not sequence shape.
    pub fn as_mut(&mut self) -> TinyVecMut<'_, T> {
        TinyVecMut {
            values: self.backing.values_mut(self.initialized),
        }
    }
}

impl<T, B: Storage<T>> Drop for SealedTinyVec<T, B> {
    fn drop(&mut self) {
        drop_initialized::<T, _>(&mut self.backing, &mut self.initialized, 0);
    }
}

impl<'a, T> TinyVecRef<'a, T> {
    /// Returns the number of values in this view.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the view has no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterates over the borrowed values.
    pub fn iter(&self) -> core::slice::Iter<'a, T> {
        self.values.iter()
    }
}

impl<'a, T> TinyVecMut<'a, T> {
    /// Returns the number of values in this view.
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether the view has no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterates over the borrowed values.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.values.iter()
    }

    /// Iterates mutably over the borrowed values without changing their count.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.values.iter_mut()
    }
}

fn new_builder<T, B: Storage<T>>(backing: B) -> TinyVecBuilder<T, B> {
    TinyVecBuilder {
        backing,
        initialized: 0,
        marker: PhantomData,
    }
}

fn drop_initialized<T, B: Storage<T>>(backing: &mut B, initialized: &mut usize, original: usize) {
    let mut cleanup = DropRemaining {
        backing,
        initialized,
        original,
        marker: PhantomData,
        active: true,
    };
    cleanup.run();
    cleanup.active = false;
}

/// Cleans a remaining initialized suffix, including while a value destructor unwinds.
///
/// Each slot is protected by a nested guard. If one value destructor panics,
/// that guard cleans the remaining valid slots during the active unwind. A
/// second destructor panic is intentionally left to Rust's normal double-panic
/// abort behavior.
struct DropRemaining<'a, T, B: Storage<T>> {
    backing: &'a mut B,
    initialized: &'a mut usize,
    original: usize,
    marker: PhantomData<T>,
    active: bool,
}

impl<T, B: Storage<T>> DropRemaining<'_, T, B> {
    fn run(&mut self) {
        while *self.initialized > self.original {
            *self.initialized -= 1;
            let index = *self.initialized;
            let mut remaining = DropRemaining {
                backing: &mut *self.backing,
                initialized: &mut *self.initialized,
                original: self.original,
                marker: PhantomData,
                active: true,
            };
            remaining.backing.drop_slot(index);
            remaining.active = false;
        }
    }
}

impl<T, B: Storage<T>> Drop for DropRemaining<'_, T, B> {
    fn drop(&mut self) {
        if self.active {
            self.run();
        }
    }
}

struct Rollback<'a, T, B: Storage<T>> {
    builder: &'a mut TinyVecBuilder<T, B>,
    original: usize,
    committed: bool,
}

impl<'a, T, B: Storage<T>> Rollback<'a, T, B> {
    fn new(builder: &'a mut TinyVecBuilder<T, B>) -> Self {
        Self {
            original: builder.initialized,
            builder,
            committed: false,
        }
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl<T, B: Storage<T>> Drop for Rollback<'_, T, B> {
    fn drop(&mut self) {
        if !self.committed {
            self.builder.drop_suffix_from(self.original);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::MaybeUninit;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
        vec::Vec,
    };

    use super::{BorrowedStorage, InlineStorage, TinyVecBuilder};
    use crate::TinyMapError;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static DROPS: [AtomicUsize; 4] = [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ];
    static PANIC_ON_DROP: AtomicUsize = AtomicUsize::new(usize::MAX);

    #[derive(Debug)]
    struct DropCounter(usize);

    impl DropCounter {
        fn new(index: usize) -> Self {
            Self(index)
        }
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROPS[self.0].fetch_add(1, Ordering::SeqCst);
            if PANIC_ON_DROP.swap(usize::MAX, Ordering::SeqCst) == self.0 {
                panic!("value destructor panic");
            }
        }
    }

    fn reset_drops() {
        for count in &DROPS {
            count.store(0, Ordering::SeqCst);
        }
        PANIC_ON_DROP.store(usize::MAX, Ordering::SeqCst);
    }

    fn drops() -> [usize; 4] {
        DROPS.each_ref().map(|count| count.load(Ordering::SeqCst))
    }

    struct LyingExact<I> {
        values: I,
        expected: usize,
    }

    impl<const N: usize> LyingExact<core::array::IntoIter<DropCounter, N>> {
        fn short(values: [DropCounter; N]) -> Self {
            Self {
                values: values.into_iter(),
                expected: N + 1,
            }
        }

        fn long(values: [DropCounter; N]) -> Self {
            Self {
                values: values.into_iter(),
                expected: N - 1,
            }
        }
    }

    impl<I: Iterator> Iterator for LyingExact<I> {
        type Item = I::Item;

        fn next(&mut self) -> Option<Self::Item> {
            self.values.next()
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.expected, Some(self.expected))
        }
    }

    impl<I: Iterator> ExactSizeIterator for LyingExact<I> {
        fn len(&self) -> usize {
            self.expected
        }
    }

    struct CountLiar {
        values: core::array::IntoIter<DropCounter, 3>,
    }

    impl CountLiar {
        fn new() -> Self {
            Self {
                values: [
                    DropCounter::new(1),
                    DropCounter::new(2),
                    DropCounter::new(3),
                ]
                .into_iter(),
            }
        }
    }

    impl Iterator for CountLiar {
        type Item = DropCounter;

        fn next(&mut self) -> Option<Self::Item> {
            self.values.next()
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (2, Some(2))
        }

        fn count(self) -> usize {
            0
        }
    }

    impl ExactSizeIterator for CountLiar {
        fn len(&self) -> usize {
            2
        }
    }

    struct PanicAfterOne {
        next: usize,
    }

    impl PanicAfterOne {
        fn new() -> Self {
            Self { next: 0 }
        }
    }

    impl Iterator for PanicAfterOne {
        type Item = DropCounter;

        fn next(&mut self) -> Option<Self::Item> {
            match self.next {
                0 => {
                    self.next += 1;
                    Some(DropCounter::new(1))
                }
                _ => panic!("iterator panic"),
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (2, Some(2))
        }
    }

    impl ExactSizeIterator for PanicAfterOne {
        fn len(&self) -> usize {
            2
        }
    }

    struct PanicOnDropExact {
        values: core::array::IntoIter<DropCounter, 2>,
    }

    impl PanicOnDropExact {
        fn new() -> Self {
            Self {
                values: [DropCounter::new(1), DropCounter::new(2)].into_iter(),
            }
        }
    }

    impl Iterator for PanicOnDropExact {
        type Item = DropCounter;

        fn next(&mut self) -> Option<Self::Item> {
            self.values.next()
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            self.values.size_hint()
        }
    }

    impl ExactSizeIterator for PanicOnDropExact {
        fn len(&self) -> usize {
            self.values.len()
        }
    }

    impl Drop for PanicOnDropExact {
        fn drop(&mut self) {
            panic!("iterator destructor panic");
        }
    }

    #[test]
    fn inline_and_borrowed_backing_rollback_a_short_exact_iterator() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        let error = inline
            .try_extend_exact(LyingExact::short([DropCounter::new(1)]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 1
            }
        );
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 0, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 3] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        let error = borrowed
            .try_extend_exact(LyingExact::short([DropCounter::new(1)]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 1
            }
        );
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[test]
    fn inline_and_borrowed_backing_reject_capacity_without_touching_existing_values() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 1>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        let error = inline.try_push(DropCounter::new(1)).unwrap_err();
        assert_eq!(
            error,
            TinyMapError::Capacity {
                limit: 1,
                requested: 2
            }
        );
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 0, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 1] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        let error = borrowed.try_push(DropCounter::new(1)).unwrap_err();
        assert_eq!(
            error,
            TinyMapError::Capacity {
                limit: 1,
                requested: 2
            }
        );
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 0, 0]);

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 1>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        let error = inline
            .try_extend_exact(LyingExact::short([DropCounter::new(1)]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::Capacity {
                limit: 1,
                requested: 3
            }
        );
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 0, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 1] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        let error = borrowed
            .try_extend_exact(LyingExact::short([DropCounter::new(1)]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::Capacity {
                limit: 1,
                requested: 3
            }
        );
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[test]
    fn inline_and_borrowed_backing_rollback_a_long_exact_iterator() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 4>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        let error = inline
            .try_extend_exact(LyingExact::long([
                DropCounter::new(1),
                DropCounter::new(2),
                DropCounter::new(3),
            ]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 3
            }
        );
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 1]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 1, 1]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 4] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        let error = borrowed
            .try_extend_exact(LyingExact::long([
                DropCounter::new(1),
                DropCounter::new(2),
                DropCounter::new(3),
            ]))
            .unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 3
            }
        );
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 1]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 1, 1]);
    }

    #[test]
    fn inline_and_borrowed_backing_reject_count_override_that_hides_extra_values() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 4>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        let error = inline.try_extend_exact(CountLiar::new()).unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 3
            }
        );
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 1]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 1, 1]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 4] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        let error = borrowed.try_extend_exact(CountLiar::new()).unwrap_err();
        assert_eq!(
            error,
            TinyMapError::ExactLength {
                expected: 2,
                actual: 3
            }
        );
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 1]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 1, 1]);
    }

    #[test]
    fn inline_and_borrowed_backing_rollback_remaining_values_after_a_drop_panic() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 4>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            inline.try_extend_exact(LyingExact::short([
                DropCounter::new(1),
                DropCounter::new(2),
            ]))
        }))
        .is_err());
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 0]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 1, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 4] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| {
            borrowed.try_extend_exact(LyingExact::short([
                DropCounter::new(1),
                DropCounter::new(2),
            ]))
        }))
        .is_err());
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 0]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 1, 0]);
    }

    #[test]
    fn inline_and_borrowed_backing_drop_remaining_values_after_a_drop_panic() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        inline.try_push(DropCounter::new(1)).unwrap();
        inline.try_push(DropCounter::new(2)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| drop(inline))).is_err());
        assert_eq!(drops(), [1, 1, 1, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 3] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        borrowed.try_push(DropCounter::new(1)).unwrap();
        borrowed.try_push(DropCounter::new(2)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| drop(borrowed))).is_err());
        assert_eq!(drops(), [1, 1, 1, 0]);
    }

    #[test]
    fn inline_and_borrowed_backing_rollback_when_the_iterator_panics() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        assert!(catch_unwind(AssertUnwindSafe(
            || inline.try_extend_exact(PanicAfterOne::new())
        ))
        .is_err());
        assert_eq!(inline.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(inline);
        assert_eq!(drops(), [1, 1, 0, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 3] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        assert!(catch_unwind(AssertUnwindSafe(
            || borrowed.try_extend_exact(PanicAfterOne::new())
        ))
        .is_err());
        assert_eq!(borrowed.len(), 1);
        assert_eq!(drops(), [0, 1, 0, 0]);
        drop(borrowed);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[test]
    fn iterator_drop_panic_rolls_back_exact_extension() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut builder = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        builder.try_push(DropCounter::new(0)).unwrap();

        assert!(catch_unwind(AssertUnwindSafe(|| {
            builder.try_extend_exact(PanicOnDropExact::new())
        }))
        .is_err());
        assert_eq!(builder.len(), 1);
        assert_eq!(drops(), [0, 1, 1, 0]);

        drop(builder);
        assert_eq!(drops(), [1, 1, 1, 0]);
    }

    #[test]
    fn sealed_inline_storage_can_move_then_yield_a_borrowed_view() {
        let mut builder = TinyVecBuilder::<u16, InlineStorage<u16, 3>>::inline();
        builder.try_extend_exact([10, 20].into_iter()).unwrap();

        let sealed = builder.seal();
        let moved = [sealed].into_iter().next().unwrap();

        assert_eq!(moved.len(), 2);
        assert_eq!(moved.as_ref().iter().copied().collect::<Vec<_>>(), [10, 20]);
    }

    #[test]
    fn sealed_inline_storage_moves_drop_counters_without_dropping_them() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut builder = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        builder.try_push(DropCounter::new(0)).unwrap();
        builder.try_push(DropCounter::new(1)).unwrap();

        let moved = [builder.seal()].into_iter().next().unwrap();

        assert_eq!(drops(), [0, 0, 0, 0]);
        assert_eq!(moved.as_ref().len(), 2);
        drop(moved);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[test]
    fn sealed_borrowed_storage_moves_and_mutates_drop_counters_without_changing_length() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 3] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut builder = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        builder.try_push(DropCounter::new(0)).unwrap();
        builder.try_push(DropCounter::new(1)).unwrap();

        let mut moved = [builder.seal()].into_iter().next().unwrap();

        assert_eq!(drops(), [0, 0, 0, 0]);
        {
            let mut values = moved.as_mut();
            let mut iter = values.iter_mut();
            let first = iter.next().unwrap();
            let second = iter.next().unwrap();
            core::mem::swap(first, second);
            assert_eq!(values.len(), 2);
            assert_eq!(
                values.iter().map(|value| value.0).collect::<Vec<_>>(),
                [1, 0]
            );
        }
        assert_eq!(moved.as_ref().len(), 2);
        drop(moved);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[test]
    fn sealed_inline_and_borrowed_storage_drop_remaining_values_after_a_drop_panic() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut inline = TinyVecBuilder::<DropCounter, InlineStorage<DropCounter, 3>>::inline();
        inline.try_push(DropCounter::new(0)).unwrap();
        inline.try_push(DropCounter::new(1)).unwrap();
        inline.try_push(DropCounter::new(2)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| drop(inline.seal()))).is_err());
        assert_eq!(drops(), [1, 1, 1, 0]);

        reset_drops();
        let mut slots: [MaybeUninit<DropCounter>; 3] =
            core::array::from_fn(|_| MaybeUninit::uninit());
        let mut borrowed = TinyVecBuilder::borrowed(BorrowedStorage::new(&mut slots));
        borrowed.try_push(DropCounter::new(0)).unwrap();
        borrowed.try_push(DropCounter::new(1)).unwrap();
        borrowed.try_push(DropCounter::new(2)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| drop(borrowed.seal()))).is_err());
        assert_eq!(drops(), [1, 1, 1, 0]);
    }

    #[test]
    fn sealed_mutable_view_changes_values_without_changing_length() {
        let mut builder = TinyVecBuilder::<u16, InlineStorage<u16, 3>>::inline();
        builder.try_extend_exact([10, 20].into_iter()).unwrap();
        let mut sealed = builder.seal();

        {
            let mut values = sealed.as_mut();
            values.iter_mut().for_each(|value| *value += 1);
            assert_eq!(values.len(), 2);
            assert_eq!(values.iter().copied().collect::<Vec<_>>(), [11, 21]);
        }

        let values = sealed.as_ref();
        assert_eq!(values.len(), 2);
        assert_eq!(values.iter().copied().collect::<Vec<_>>(), [11, 21]);
    }

    #[test]
    fn inline_storage_seals_and_mutates_without_erased_dispatch_state() {
        let mut builder = TinyVecBuilder::<u16, InlineStorage<u16, 3>>::inline();
        builder.try_extend_exact([10, 20].into_iter()).unwrap();
        let mut sealed = builder.seal();

        sealed.as_mut().iter_mut().for_each(|value| *value += 1);

        assert_eq!(
            sealed.as_ref().iter().copied().collect::<Vec<_>>(),
            [11, 21]
        );
        assert_eq!(
            core::mem::size_of::<TinyVecBuilder<u16, InlineStorage<u16, 3>>>(),
            core::mem::size_of::<(InlineStorage<u16, 3>, usize)>(),
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn sealed_heap_storage_can_move_then_yield_a_borrowed_view() {
        let mut builder = TinyVecBuilder::<u16, super::HeapStorage<u16>>::heap();
        builder.try_extend_exact([10, 20].into_iter()).unwrap();

        let sealed = builder.seal();
        let moved = [sealed].into_iter().next().unwrap();

        assert_eq!(moved.as_ref().iter().copied().collect::<Vec<_>>(), [10, 20]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn sealed_heap_storage_moves_drop_counters_without_dropping_them() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut builder = TinyVecBuilder::<DropCounter, super::HeapStorage<DropCounter>>::heap();
        builder.try_push(DropCounter::new(0)).unwrap();
        builder.try_push(DropCounter::new(1)).unwrap();

        let moved = [builder.seal()].into_iter().next().unwrap();

        assert_eq!(drops(), [0, 0, 0, 0]);
        assert_eq!(moved.as_ref().len(), 2);
        drop(moved);
        assert_eq!(drops(), [1, 1, 0, 0]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn sealed_heap_storage_drops_remaining_values_after_a_drop_panic() {
        let _lock = TEST_LOCK.lock().unwrap();

        reset_drops();
        let mut heap = TinyVecBuilder::<DropCounter, super::HeapStorage<DropCounter>>::heap();
        heap.try_push(DropCounter::new(0)).unwrap();
        heap.try_push(DropCounter::new(1)).unwrap();
        heap.try_push(DropCounter::new(2)).unwrap();
        PANIC_ON_DROP.store(2, Ordering::SeqCst);
        assert!(catch_unwind(AssertUnwindSafe(|| drop(heap.seal()))).is_err());
        assert_eq!(drops(), [1, 1, 1, 0]);
    }
}
