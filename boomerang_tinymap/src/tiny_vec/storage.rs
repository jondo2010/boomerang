use core::mem::MaybeUninit;

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

/// Private access to the backing slots used by the builder.
pub(super) trait Storage<T> {
    fn capacity(&self) -> usize;
    fn slots(&mut self) -> &mut [MaybeUninit<T>];
}

impl<T, const N: usize> Storage<T> for InlineStorage<T, N> {
    fn capacity(&self) -> usize {
        N
    }

    fn slots(&mut self) -> &mut [MaybeUninit<T>] {
        &mut self.slots
    }
}

impl<T> Storage<T> for BorrowedStorage<'_, T> {
    fn capacity(&self) -> usize {
        self.slots.len()
    }

    fn slots(&mut self) -> &mut [MaybeUninit<T>] {
        self.slots
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
