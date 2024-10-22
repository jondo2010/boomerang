//! Iterators over references to Reactor elements owned by the `Store`.
//!
//! The partitioning logic is used to destructure a [`Refs`] and [`RefsMut`] into a tuple of concrete types.
//! The `Partition` and `PartitionMut` traits are implemented for tuples of concrete types.
//! The `part` and `part_mut` functions are used to perform the dynamic destructuring.
//!
//! This module is *not* necessary When using derived Reaction implementations (e.g. `#[derive(Reaction)]`), as the
//! generated code will handle the partitioning logic under the hood.
//!
//! # Example
//!
//! ```rust,ignore
//! reaction_closure!(_ctx, _state, ref_ports, _mut_ports, _actions => {
//!     // destructure the ref_ports into a a tuple of concrete types
//!     let (clock, [in1, in2]): (runtime::InputRef<bool>, [runtime::InputRef<u32>; 2]) =
//!         ref_ports.partition().unwrap();
//! });
//! ```

use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{BaseAction, BasePort};

/// Iterator over references to elements in a `Vec<NonNull<T>>`.
pub struct Refs<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    /// the non-null pointer to the past-the-end element.
    end: *mut NonNull<T>,
    _marker: PhantomData<&'a T>,
}

impl<T: std::fmt::Debug> std::fmt::Debug for Refs<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.len();
        let slice = unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), len) };
        f.debug_tuple("Refs").field(&slice).finish()
    }
}

impl<'a, T: 'a + ?Sized> Refs<'a, T> {
    pub(crate) fn new(vec: &mut Vec<NonNull<T>>) -> Self {
        // SAFETY: `vec` is guaranteed to be non-empty.
        let ptr: NonNull<NonNull<T>> = unsafe { NonNull::new_unchecked(vec.as_mut_ptr()) };
        let len = vec.len();
        let end = unsafe { ptr.as_ptr().add(len) };
        Self {
            ptr: ptr.cast(),
            end,
            _marker: PhantomData,
        }
    }

    /// Wrapper function for dynamic destructuring.
    ///
    /// This function is used to destructure `Refs` into a tuple of concrete types.
    /// The function will panic if not fully consumed.
    pub fn partition<S>(self) -> Option<S>
    where
        S: Partition<'a, T>,
    {
        if let Some((result, rest)) = S::part(self) {
            assert_eq!(rest.len(), 0, "Partition error");
            Some(result)
        } else {
            None
        }
    }
}

impl<'a, T: 'a + ?Sized> Iterator for Refs<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ptr.as_ptr() == self.end {
            None
        } else {
            let ptr = self.ptr;
            // SAFETY: `ptr` is guaranteed to be non-null and within bounds.
            self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(1)) };
            Some(unsafe { (*ptr.as_ptr()).as_ref() })
        }
    }
}

impl<'a, T: 'a + ?Sized> ExactSizeIterator for Refs<'a, T> {
    fn len(&self) -> usize {
        unsafe { usize::try_from(self.end.offset_from(self.ptr.as_ptr())).unwrap_unchecked() }
    }
}

/// Iterator over mutable references to elements in a `Vec<NonNull<T>>`.
pub struct RefsMut<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    /// the non-null pointer to the past-the-end element.
    end: *mut NonNull<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<T: std::fmt::Debug> std::fmt::Debug for RefsMut<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.len();
        let slice = unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), len) };
        f.debug_tuple("RefsMut").field(&slice).finish()
    }
}

impl<'a, T: 'a + ?Sized> RefsMut<'a, T> {
    pub(crate) fn new(vec: &mut Vec<NonNull<T>>) -> Self {
        // SAFETY: `vec` is guaranteed to be non-empty.
        let ptr: NonNull<NonNull<T>> = unsafe { NonNull::new_unchecked(vec.as_mut_ptr()) };
        let len = vec.len();
        let end = unsafe { ptr.as_ptr().add(len) };
        Self {
            ptr: ptr.cast(),
            end,
            _marker: PhantomData,
        }
    }

    /// Wrapper function for dynamic destructuring with mutable references.
    ///
    /// This function is used to destructure `RefsMut` into a tuple of concrete types.
    /// The function will panic if not fully consumed.
    pub fn partition_mut<S>(self) -> Option<S>
    where
        S: PartitionMut<'a, T>,
    {
        if let Some((result, rest)) = S::part_mut(self) {
            assert_eq!(rest.len(), 0, "Destructuring error");
            Some(result)
        } else {
            None
        }
    }
}

impl<'a, T: 'a + ?Sized> Iterator for RefsMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ptr.as_ptr() == self.end {
            None
        } else {
            let ptr = self.ptr;
            // SAFETY: `ptr` is guaranteed to be non-null and within bounds.
            self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(1)) };
            Some(unsafe { (*ptr.as_ptr()).as_mut() })
        }
    }
}

impl<'a, T: 'a + ?Sized> ExactSizeIterator for RefsMut<'a, T> {
    fn len(&self) -> usize {
        unsafe { usize::try_from(self.end.offset_from(self.ptr.as_ptr())).unwrap_unchecked() }
    }
}

pub trait Partition<'a, T: ?Sized>: Sized {
    fn part(refs: Refs<'a, T>) -> Option<(Self, Refs<'a, T>)>;
}

pub trait PartitionMut<'a, T: ?Sized>: Sized {
    fn part_mut(refs: RefsMut<'a, T>) -> Option<(Self, RefsMut<'a, T>)>;
}

// Partition for BasePort scalars
impl<'a, P> Partition<'a, dyn BasePort> for P
where
    P: From<&'a dyn BasePort>,
{
    fn part(mut refs: Refs<'a, dyn BasePort>) -> Option<(Self, Refs<'a, dyn BasePort>)> {
        refs.next().map(|p| (Self::from(p), refs))
    }
}

// Partition for BasePort arrays
impl<'a, P, const N: usize> Partition<'a, dyn BasePort> for [P; N]
where
    P: From<&'a dyn BasePort>,
{
    fn part(mut refs: Refs<'a, dyn BasePort>) -> Option<(Self, Refs<'a, dyn BasePort>)> {
        if refs.len() < N {
            return None;
        }

        let mut array = MaybeUninit::<[P; N]>::uninit();

        for i in 0..N {
            if let Some(r) = refs.next() {
                // Safety: should be safe since we have checked the length of the iterator and are not reading from
                // uninitialized memory.
                unsafe {
                    (*array.as_mut_ptr()).as_mut_ptr().add(i).write(P::from(r));
                }
            } else {
                // Not enough elements in the iterator.
                return None;
            }
        }

        // Safety: we have initialized all elements of the array.
        Some((unsafe { array.assume_init() }, refs))
    }
}

// PartitionMut for BasePort scalars
impl<'a, P> PartitionMut<'a, dyn BasePort> for P
where
    P: From<&'a mut dyn BasePort>,
{
    fn part_mut(mut refs: RefsMut<'a, dyn BasePort>) -> Option<(Self, RefsMut<'a, dyn BasePort>)> {
        refs.next().map(|p| (Self::from(p), refs))
    }
}

// PartitionMut for BasePort arrays
impl<'a, P, const N: usize> PartitionMut<'a, dyn BasePort> for [P; N]
where
    P: From<&'a mut (dyn BasePort)>,
{
    fn part_mut(mut refs: RefsMut<'a, dyn BasePort>) -> Option<(Self, RefsMut<'a, dyn BasePort>)> {
        if refs.len() < N {
            return None;
        }

        let mut array = MaybeUninit::<[P; N]>::uninit();

        for i in 0..N {
            if let Some(r) = refs.next() {
                // Safety: should be safe since we have checked the length of the iterator and are not reading from
                // uninitialized memory.
                unsafe {
                    (*array.as_mut_ptr()).as_mut_ptr().add(i).write(P::from(r));
                }
            } else {
                // Not enough elements in the iterator.
                return None;
            }
        }

        // Safety: we have initialized all elements of the array.
        Some((unsafe { array.assume_init() }, refs))
    }
}

// PartitionMut for BaseAction scalars
impl<'a, A> PartitionMut<'a, dyn BaseAction> for A
where
    A: From<&'a mut dyn BaseAction>,
{
    fn part_mut(
        mut refs: RefsMut<'a, dyn BaseAction>,
    ) -> Option<(Self, RefsMut<'a, dyn BaseAction>)> {
        refs.next().map(|a| (Self::from(a), refs))
    }
}

// PartitionMut for BaseAction arrays
impl<'a, A, const N: usize> PartitionMut<'a, dyn BaseAction> for [A; N]
where
    A: From<&'a mut dyn BaseAction>,
{
    fn part_mut(
        mut refs: RefsMut<'a, dyn BaseAction>,
    ) -> Option<(Self, RefsMut<'a, dyn BaseAction>)> {
        if refs.len() < N {
            return None;
        }

        let mut array = MaybeUninit::<[A; N]>::uninit();

        for i in 0..N {
            if let Some(a) = refs.next() {
                // Safety: should be safe since we have checked the length of the iterator and are not reading from
                // uninitialized memory.
                unsafe {
                    (*array.as_mut_ptr()).as_mut_ptr().add(i).write(A::from(a));
                }
            } else {
                // Not enough elements in the iterator.
                return None;
            }
        }

        // Safety: we have initialized all elements of the array.
        Some((unsafe { array.assume_init() }, refs))
    }
}

macro_rules! impl_part_for_tuples {
    ($($S:ident),+) => {
        impl<'a, T: ?Sized, $($S),+> Partition<'a, T> for ($($S),+,)
        where
            $($S: Partition<'a, T>),+
        {
            fn part(refs: Refs<'a, T>) -> Option<(Self, Refs<'a, T>)> {
                let (elements, rest) = {
                    let mut rest = refs;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $S::part(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+,)
                    , rest)
                };
                Some((elements, rest))
            }
        }

        impl<'a, T: ?Sized, $($S),+> PartitionMut<'a, T> for ($($S),+,)
        where
            $($S: PartitionMut<'a, T>),+
        {
            fn part_mut(refs: RefsMut<'a, T>) -> Option<(Self, RefsMut<'a, T>)> {
                let (elements, rest) = {
                    let mut rest = refs;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $S::part_mut(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+,)
                    , rest)
                };
                Some((elements, rest))
            }
        }
    };
}

// Implement the macro for tuples of length 1 through 5
impl_part_for_tuples!(T0);
impl_part_for_tuples!(T0, T1);
impl_part_for_tuples!(T0, T1, T2);
impl_part_for_tuples!(T0, T1, T2, T3);
impl_part_for_tuples!(T0, T1, T2, T3, T4);

#[cfg(test)]
mod tests {
    use crate::{BasePort, InputRef, OutputRef, Port};

    use super::*;

    #[test]
    fn test_partition() {
        // Create some concrete ports
        let mut ports: tinymap::TinyMap<_, Box<dyn BasePort>> = tinymap::TinyMap::new();
        let k0 = ports.insert_with_key(|key| Box::new(Port::<i32>::new("p0", key)));
        let k1 = ports.insert_with_key(|key| Box::new(Port::<u32>::new("p1", key)));
        let k2 = ports.insert_with_key(|key| Box::new(Port::<bool>::new("p2a", key)));
        let k3 = ports.insert_with_key(|key| Box::new(Port::<bool>::new("p2b", key)));

        let mut ptrs = unsafe {
            ports
                .iter_many_unchecked_ptrs_mut([k0, k1, k2, k3])
                .map(|a| NonNull::new_unchecked(&mut *(*a) as *mut _))
                .collect::<Vec<_>>()
        };
        let refs = Refs::new(&mut ptrs);

        // Test the partition function
        let (p0, p1, p2a): (InputRef<i32>, InputRef<u32>, [InputRef<bool>; 2]) =
            refs.partition().unwrap();

        assert_eq!(p0.name(), "p0");
        assert_eq!(p1.name(), "p1");
        assert_eq!(p2a[0].name(), "p2a");
        assert_eq!(p2a[1].name(), "p2b");

        // Test the partition_mut function
        let mut ptrs = unsafe {
            ports
                .iter_many_unchecked_ptrs_mut([k1, k2, k3])
                .map(|a| NonNull::new_unchecked(&mut *(*a) as *mut _))
                .collect::<Vec<_>>()
        };

        let refs_mut = RefsMut::new(&mut ptrs);
        let (p1, p2a): (OutputRef<u32>, [OutputRef<bool>; 2]) = refs_mut.partition_mut().unwrap();

        assert_eq!(p1.name(), "p1");
        assert_eq!(p2a[0].name(), "p2a");
        assert_eq!(p2a[1].name(), "p2b");
    }
}
