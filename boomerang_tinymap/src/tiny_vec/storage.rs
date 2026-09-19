use core::mem::MaybeUninit;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Inline fixed-capacity storage for a [`super::TinyVecBuilder`].
pub struct InlineStorage<T, const N: usize> {
    slots: [MaybeUninit<T>; N],
}

impl<T, const N: usize> InlineStorage<T, N> {
    /// Creates uninitialized storage with capacity `N`.
    pub fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| MaybeUninit::uninit()),
        }
    }
}

impl<T, const N: usize> Default for InlineStorage<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Caller-provided fixed-capacity storage for a [`super::TinyVecBuilder`].
pub struct BorrowedStorage<'a, T> {
    slots: &'a mut [MaybeUninit<T>],
}

impl<'a, T> BorrowedStorage<'a, T> {
    /// Wraps caller-owned uninitialized slots without allocating.
    pub fn new(slots: &'a mut [MaybeUninit<T>]) -> Self {
        Self { slots }
    }
}

/// Heap-backed storage for a [`super::TinyVecBuilder`].
///
/// This adapter grows its owned vector as values are added. It is available
/// only when the crate's `alloc` feature is enabled.
#[cfg(feature = "alloc")]
pub struct HeapStorage<T> {
    values: Vec<T>,
}

#[cfg(feature = "alloc")]
impl<T> HeapStorage<T> {
    /// Creates empty heap-backed storage.
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }
}

#[cfg(feature = "alloc")]
impl<T> Default for HeapStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Private access to the backing slots used by the builder.
pub(super) trait Storage<T> {
    fn capacity(&self) -> usize;
    fn write_slot(&mut self, index: usize, value: T);
    fn drop_slot(&mut self, index: usize);
    fn values(&self, initialized: usize) -> &[T];
    fn values_mut(&mut self, initialized: usize) -> &mut [T];
}

impl<T, const N: usize> Storage<T> for InlineStorage<T, N> {
    fn capacity(&self) -> usize {
        N
    }

    fn write_slot(&mut self, index: usize, value: T) {
        self.slots[index].write(value);
    }

    fn drop_slot(&mut self, index: usize) {
        // SAFETY: the builder marks exactly its initialized prefix as valid,
        // decrements that prefix before requesting a drop, and passes an index
        // in the prior prefix. This selects one valid value for one drop.
        unsafe {
            self.slots[index].assume_init_drop();
        }
    }

    fn values(&self, initialized: usize) -> &[T] {
        // SAFETY: TinyVecBuilder writes exactly the initialized prefix and
        // updates its initialized count only after each write. Callers supply
        // that count, so this prefix contains valid, properly aligned T values.
        unsafe { core::slice::from_raw_parts(self.slots.as_ptr().cast::<T>(), initialized) }
    }

    fn values_mut(&mut self, initialized: usize) -> &mut [T] {
        // SAFETY: as above, the initialized prefix contains valid T values.
        // The mutable borrow of this storage prevents aliasing that prefix.
        unsafe { core::slice::from_raw_parts_mut(self.slots.as_mut_ptr().cast::<T>(), initialized) }
    }
}

impl<T> Storage<T> for BorrowedStorage<'_, T> {
    fn capacity(&self) -> usize {
        self.slots.len()
    }

    fn write_slot(&mut self, index: usize, value: T) {
        self.slots[index].write(value);
    }

    fn drop_slot(&mut self, index: usize) {
        // SAFETY: the builder marks exactly its initialized prefix as valid,
        // decrements that prefix before requesting a drop, and passes an index
        // in the prior prefix. This selects one valid value for one drop.
        unsafe {
            self.slots[index].assume_init_drop();
        }
    }

    fn values(&self, initialized: usize) -> &[T] {
        // SAFETY: TinyVecBuilder writes exactly the initialized prefix and
        // updates its initialized count only after each write. Callers supply
        // that count, so this prefix contains valid, properly aligned T values.
        unsafe { core::slice::from_raw_parts(self.slots.as_ptr().cast::<T>(), initialized) }
    }

    fn values_mut(&mut self, initialized: usize) -> &mut [T] {
        // SAFETY: as above, the initialized prefix contains valid T values.
        // The mutable borrow of this storage prevents aliasing that prefix.
        unsafe { core::slice::from_raw_parts_mut(self.slots.as_mut_ptr().cast::<T>(), initialized) }
    }
}

#[cfg(feature = "alloc")]
impl<T> Storage<T> for HeapStorage<T> {
    fn capacity(&self) -> usize {
        usize::MAX
    }

    fn write_slot(&mut self, index: usize, value: T) {
        debug_assert_eq!(index, self.values.len());
        self.values.push(value);
    }

    fn drop_slot(&mut self, index: usize) {
        debug_assert_eq!(index + 1, self.values.len());
        drop(self.values.pop());
    }

    fn values(&self, initialized: usize) -> &[T] {
        &self.values[..initialized]
    }

    fn values_mut(&mut self, initialized: usize) -> &mut [T] {
        &mut self.values[..initialized]
    }
}

#[cfg(test)]
mod tests {
    use core::mem::MaybeUninit;

    use super::{BorrowedStorage, InlineStorage, Storage};

    #[test]
    fn inline_and_borrowed_storage_report_their_fixed_capacity() {
        let inline = InlineStorage::<u8, 3>::new();
        assert_eq!(Storage::capacity(&inline), 3);

        let mut slots: [MaybeUninit<u8>; 2] = core::array::from_fn(|_| MaybeUninit::uninit());
        let borrowed = BorrowedStorage::new(&mut slots);
        assert_eq!(Storage::capacity(&borrowed), 2);
    }
}
