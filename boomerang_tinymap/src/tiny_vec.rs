mod storage;

use core::marker::PhantomData;

use crate::TinyMapError;

use storage::Storage;
pub use storage::{BorrowedStorage, InlineStorage};

/// A not-yet-sealed, fixed-capacity sequence builder.
///
/// Values occupy the initialized prefix of its private backing storage. The
/// storage interface remains private so callers cannot observe or alter the
/// initialized-slot metadata.
pub struct TinyVecBuilder<T, B> {
    backing: B,
    initialized: usize,
    capacity: fn(&B) -> usize,
    write_slot: fn(&mut B, usize, T),
    drop_slot: fn(&mut B, usize),
    marker: PhantomData<T>,
}

impl<T, B> TinyVecBuilder<T, B> {
    /// Returns the number of values currently initialized in the builder.
    pub const fn len(&self) -> usize {
        self.initialized
    }

    /// Appends one value without allocating.
    pub fn try_push(&mut self, value: T) -> Result<(), TinyMapError> {
        let limit = (self.capacity)(&self.backing);
        if self.initialized == limit {
            return Err(TinyMapError::Capacity {
                limit,
                requested: self.initialized.saturating_add(1),
            });
        }

        (self.write_slot)(&mut self.backing, self.initialized, value);
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
        let limit = (self.capacity)(&self.backing);
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

        let actual = expected.saturating_add(values.count());
        if actual != expected {
            return Err(TinyMapError::ExactLength { expected, actual });
        }

        rollback.commit();
        Ok(())
    }

    fn drop_suffix_from(&mut self, original: usize) {
        while self.initialized > original {
            self.initialized -= 1;
            (self.drop_slot)(&mut self.backing, self.initialized);
        }
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

impl<T, B> Drop for TinyVecBuilder<T, B> {
    fn drop(&mut self) {
        drop_initialized(&mut self.backing, &mut self.initialized, self.drop_slot);
    }
}

fn new_builder<T, B: Storage<T>>(backing: B) -> TinyVecBuilder<T, B> {
    TinyVecBuilder {
        backing,
        initialized: 0,
        capacity: storage_capacity::<T, B>,
        write_slot: write_slot::<T, B>,
        drop_slot: drop_slot::<T, B>,
        marker: PhantomData,
    }
}

fn storage_capacity<T, B: Storage<T>>(backing: &B) -> usize {
    backing.capacity()
}

fn write_slot<T, B: Storage<T>>(backing: &mut B, index: usize, value: T) {
    backing.slots()[index].write(value);
}

fn drop_slot<T, B: Storage<T>>(backing: &mut B, index: usize) {
    // SAFETY: the caller establishes that `index` is a valid slot in the
    // initialized prefix, then removes it from that prefix before this call.
    // Therefore this valid value is selected for an exactly-once drop.
    unsafe {
        backing.slots()[index].assume_init_drop();
    }
}

fn drop_initialized<B>(backing: &mut B, initialized: &mut usize, drop_slot: fn(&mut B, usize)) {
    while *initialized > 0 {
        *initialized -= 1;
        drop_slot(backing, *initialized);
    }
}

struct Rollback<'a, T, B> {
    builder: &'a mut TinyVecBuilder<T, B>,
    original: usize,
    committed: bool,
}

impl<'a, T, B> Rollback<'a, T, B> {
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

impl<T, B> Drop for Rollback<'_, T, B> {
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
        }
    }

    fn reset_drops() {
        for count in &DROPS {
            count.store(0, Ordering::SeqCst);
        }
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
}
