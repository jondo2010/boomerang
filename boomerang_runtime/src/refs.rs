//! Iterators over references to Reactor elements owned by the `Store`.
//!
//! The partitioning logic is used to destructure a [`Refs`] and [`RefsMut`] into a tuple of concrete types.
//! The `Partition` and `PartitionMut` traits are implemented for tuples of concrete types.
//! The `part` and `part_mut` functions are used to perform the dynamic destructuring.
//!
//! This module is typically not necessary when using the `#[reactor]` and `reaction!` macros, as the generated code
//! handles partitioning under the hood.
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

use std::{fmt::Debug, marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use crate::{BaseAction, BasePort, DynActionRefMut, DynPortRef, DynPortRefMut, ReactionRefsError};
use variadics_please::all_tuples;

/// Iterator over references to elements in a `Vec<NonNull<T>>`.
pub struct Refs<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    /// the non-null pointer to the past-the-end element.
    end: *mut NonNull<T>,
    _marker: PhantomData<&'a T>,
}

impl<T: Debug> Debug for Refs<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.len();
        let slice = unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), len) };
        f.debug_tuple("Refs").field(&slice).finish()
    }
}

impl<'a, T: 'a + ?Sized> Refs<'a, T> {
    pub fn new(vec: &mut Vec<NonNull<T>>) -> Self {
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
    /// Returns an error if the iterator is not fully consumed or a conversion fails.
    pub fn partition<S>(self) -> Result<S, ReactionRefsError>
    where
        S: Partition<'a, T>,
    {
        let (result, rest) = S::part(self)?;
        if rest.len() != 0 {
            return Err(ReactionRefsError::destructure_remaining(rest.len()));
        }

        Ok(result)
    }

    pub fn take(&mut self, n: usize) -> Result<RefsSlice<'a, T>, ReactionRefsError> {
        if self.len() < n {
            return Err(ReactionRefsError::missing("port"));
        }

        let ptr = self.ptr;
        self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(n)) };
        Ok(RefsSlice {
            ptr,
            len: n,
            _marker: PhantomData,
        })
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

impl<T: Debug> Debug for RefsMut<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.len();
        let slice = unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), len) };
        f.debug_tuple("RefsMut").field(&slice).finish()
    }
}

impl<'a, T: 'a + ?Sized> RefsMut<'a, T> {
    pub fn new(vec: &mut Vec<NonNull<T>>) -> Self {
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
    /// Returns an error if the iterator is not fully consumed or a conversion fails.
    pub fn partition_mut<S>(self) -> Result<S, ReactionRefsError>
    where
        S: PartitionMut<'a, T>,
    {
        let (result, rest) = S::part_mut(self)?;
        if rest.len() != 0 {
            return Err(ReactionRefsError::destructure_remaining(rest.len()));
        }

        Ok(result)
    }

    pub fn take(&mut self, n: usize) -> Result<RefsSliceMut<'a, T>, ReactionRefsError> {
        if self.len() < n {
            return Err(ReactionRefsError::missing("port"));
        }

        let ptr = self.ptr;
        self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(n)) };
        Ok(RefsSliceMut {
            ptr,
            len: n,
            _marker: PhantomData,
        })
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

/// Borrowed view over a contiguous subset of `Refs`.
#[derive(Clone, Copy)]
pub struct RefsSlice<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    len: usize,
    _marker: PhantomData<&'a T>,
}

impl<'a, T: 'a + ?Sized> RefsSlice<'a, T> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> RefsSliceIter<'a, T> {
        RefsSliceIter {
            ptr: self.ptr,
            remaining: self.len,
            _marker: PhantomData,
        }
    }

    pub fn get(&self, idx: usize) -> Option<&'a T> {
        if idx >= self.len {
            return None;
        }

        let ptr = unsafe { self.ptr.as_ptr().add(idx) };
        Some(unsafe { (*ptr).as_ref() })
    }
}

pub struct RefsSliceIter<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    remaining: usize,
    _marker: PhantomData<&'a T>,
}

impl<'a, T: 'a + ?Sized> Iterator for RefsSliceIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let ptr = self.ptr;
        self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(1)) };
        self.remaining -= 1;
        Some(unsafe { (*ptr.as_ptr()).as_ref() })
    }
}

impl<'a, T: 'a + ?Sized> ExactSizeIterator for RefsSliceIter<'a, T> {
    fn len(&self) -> usize {
        self.remaining
    }
}

/// Borrowed mutable view over a contiguous subset of `RefsMut`.
pub struct RefsSliceMut<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    len: usize,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T: 'a + ?Sized> RefsSliceMut<'a, T> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> RefsSliceIter<'a, T> {
        RefsSliceIter {
            ptr: self.ptr,
            remaining: self.len,
            _marker: PhantomData,
        }
    }

    pub fn iter_mut(&mut self) -> RefsSliceIterMut<'a, T> {
        RefsSliceIterMut {
            ptr: self.ptr,
            remaining: self.len,
            _marker: PhantomData,
        }
    }

    pub fn get(&self, idx: usize) -> Option<&'a T> {
        if idx >= self.len {
            return None;
        }

        let ptr = unsafe { self.ptr.as_ptr().add(idx) };
        Some(unsafe { (*ptr).as_ref() })
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&'a mut T> {
        if idx >= self.len {
            return None;
        }

        let ptr = unsafe { self.ptr.as_ptr().add(idx) };
        Some(unsafe { (*ptr).as_mut() })
    }
}

pub struct RefsSliceIterMut<'a, T: 'a + ?Sized> {
    ptr: NonNull<NonNull<T>>,
    remaining: usize,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T: 'a + ?Sized> Iterator for RefsSliceIterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let ptr = self.ptr;
        self.ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(1)) };
        self.remaining -= 1;
        Some(unsafe { (*ptr.as_ptr()).as_mut() })
    }
}

impl<'a, T: 'a + ?Sized> ExactSizeIterator for RefsSliceIterMut<'a, T> {
    fn len(&self) -> usize {
        self.remaining
    }
}

pub trait Partition<'a, T: ?Sized>: Sized {
    fn part(refs: Refs<'a, T>) -> Result<(Self, Refs<'a, T>), ReactionRefsError>;
}

pub trait PartitionMut<'a, T: ?Sized>: Sized {
    fn part_mut(refs: RefsMut<'a, T>) -> Result<(Self, RefsMut<'a, T>), ReactionRefsError>;
}

// Partition for BasePort scalars
impl<'a, P> Partition<'a, dyn BasePort> for P
where
    P: TryFrom<DynPortRef<'a>, Error = ReactionRefsError>,
{
    fn part(mut refs: Refs<'a, dyn BasePort>) -> Result<(Self, Refs<'a, dyn BasePort>), ReactionRefsError> {
        let port = refs
            .next()
            .ok_or_else(|| ReactionRefsError::missing("port"))?;

        Ok((Self::try_from(DynPortRef(port))?, refs))
    }
}

// Partition for BasePort arrays
impl<'a, P, const N: usize> Partition<'a, dyn BasePort> for [P; N]
where
    P: TryFrom<DynPortRef<'a>, Error = ReactionRefsError>,
{
    fn part(mut refs: Refs<'a, dyn BasePort>) -> Result<(Self, Refs<'a, dyn BasePort>), ReactionRefsError> {
        if refs.len() < N {
            return Err(ReactionRefsError::missing("port"));
        }

        let mut array = MaybeUninit::<[P; N]>::uninit();

        for i in 0..N {
            let r = refs
                .next()
                .ok_or_else(|| ReactionRefsError::missing("port"))?;

            // Safety: length pre-checked; writing sequentially initialized.
            unsafe {
                (*array.as_mut_ptr())
                    .as_mut_ptr()
                    .add(i)
                    .write(P::try_from(DynPortRef(r))?);
            }
        }

        // Safety: we have initialized all elements of the array.
        Ok((unsafe { array.assume_init() }, refs))
    }
}

// PartitionMut for BasePort scalars
impl<'a, P> PartitionMut<'a, dyn BasePort> for P
where
    P: TryFrom<DynPortRefMut<'a>, Error = ReactionRefsError>,
{
    fn part_mut(mut refs: RefsMut<'a, dyn BasePort>) -> Result<(Self, RefsMut<'a, dyn BasePort>), ReactionRefsError> {
        let port = refs
            .next()
            .ok_or_else(|| ReactionRefsError::missing("port"))?;

        Ok((Self::try_from(DynPortRefMut(port))?, refs))
    }
}

// PartitionMut for BasePort arrays
impl<'a, P, const N: usize> PartitionMut<'a, dyn BasePort> for [P; N]
where
    P: TryFrom<DynPortRefMut<'a>, Error = ReactionRefsError>,
{
    fn part_mut(mut refs: RefsMut<'a, dyn BasePort>) -> Result<(Self, RefsMut<'a, dyn BasePort>), ReactionRefsError> {
        if refs.len() < N {
            return Err(ReactionRefsError::missing("port"));
        }

        let mut array = MaybeUninit::<[P; N]>::uninit();

        for i in 0..N {
            let r = refs
                .next()
                .ok_or_else(|| ReactionRefsError::missing("port"))?;

            // Safety: should be safe since we have checked the length of the iterator and are not reading from
            // uninitialized memory.
            unsafe {
                (*array.as_mut_ptr())
                    .as_mut_ptr()
                    .add(i)
                    .write(P::try_from(DynPortRefMut(r))?);
            }
        }

        // Safety: we have initialized all elements of the array.
        Ok((unsafe { array.assume_init() }, refs))
    }
}

// PartitionMut for BaseAction scalars
impl<'a, A> PartitionMut<'a, dyn BaseAction> for A
where
    A: TryFrom<DynActionRefMut<'a>, Error = ReactionRefsError>,
{
    fn part_mut(
        mut refs: RefsMut<'a, dyn BaseAction>,
    ) -> Result<(Self, RefsMut<'a, dyn BaseAction>), ReactionRefsError> {
        let action = refs
            .next()
            .ok_or_else(|| ReactionRefsError::missing("action"))?;

        Ok((Self::try_from(DynActionRefMut(action))?, refs))
    }
}

// PartitionMut for BaseAction arrays
impl<'a, A, const N: usize> PartitionMut<'a, dyn BaseAction> for [A; N]
where
    A: TryFrom<DynActionRefMut<'a>, Error = ReactionRefsError>,
{
    fn part_mut(
        mut refs: RefsMut<'a, dyn BaseAction>,
    ) -> Result<(Self, RefsMut<'a, dyn BaseAction>), ReactionRefsError> {
        if refs.len() < N {
            return Err(ReactionRefsError::missing("action"));
        }

        let mut array = MaybeUninit::<[A; N]>::uninit();

        for i in 0..N {
            let a = refs
                .next()
                .ok_or_else(|| ReactionRefsError::missing("action"))?;

            // Safety: should be safe since we have checked the length of the iterator and are not reading from
            // uninitialized memory.
            unsafe {
                (*array.as_mut_ptr())
                    .as_mut_ptr()
                    .add(i)
                    .write(A::try_from(DynActionRefMut(a))?);
            }
        }

        // Safety: we have initialized all elements of the array.
        Ok((unsafe { array.assume_init() }, refs))
    }
}

macro_rules! impl_part_for_tuples {
    ($($S:ident),+) => {
        impl<'a, T: ?Sized, $($S),+> Partition<'a, T> for ($($S),+,)
        where
            $($S: Partition<'a, T>),+
        {
            fn part(refs: Refs<'a, T>) -> Result<(Self, Refs<'a, T>), ReactionRefsError> {
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
                Ok((elements, rest))
            }
        }

        impl<'a, T: ?Sized, $($S),+> PartitionMut<'a, T> for ($($S),+,)
        where
            $($S: PartitionMut<'a, T>),+
        {
            fn part_mut(refs: RefsMut<'a, T>) -> Result<(Self, RefsMut<'a, T>), ReactionRefsError> {
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
                Ok((elements, rest))
            }
        }
    };
}

// Implement the macro for tuples of length 1 through 10
all_tuples!(impl_part_for_tuples, 1, 10, S);

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
            refs.partition().expect("partition");

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
        let (p1, p2a): (OutputRef<u32>, [OutputRef<bool>; 2]) = refs_mut.partition_mut().expect("partition_mut");

        assert_eq!(p1.name(), "p1");
        assert_eq!(p2a[0].name(), "p2a");
        assert_eq!(p2a[1].name(), "p2b");
    }

    /// Test partitioning with empty refs.
    #[test]
    fn test_empty_refs() {
        // Trying to partition an empty refs should return None
        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs = Refs::new(&mut empty_vec);
        let result: Result<InputRef<i32>, ReactionRefsError> = refs.partition();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));

        // Testing with arrays
        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs = Refs::new(&mut empty_vec);
        let result: Result<[InputRef<i32>; 2], ReactionRefsError> = refs.partition();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));

        // Testing with tuples
        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs = Refs::new(&mut empty_vec);
        let result: Result<(InputRef<i32>, InputRef<u32>), ReactionRefsError> = refs.partition();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));

        // Test for RefsMut
        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs_mut = RefsMut::new(&mut empty_vec);
        let result: Result<OutputRef<i32>, ReactionRefsError> = refs_mut.partition_mut();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));

        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs_mut = RefsMut::new(&mut empty_vec);
        let result: Result<[OutputRef<i32>; 2], ReactionRefsError> = refs_mut.partition_mut();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));

        let mut empty_vec: Vec<NonNull<dyn BasePort>> = Vec::new();
        let refs_mut = RefsMut::new(&mut empty_vec);
        let result: Result<(OutputRef<i32>, OutputRef<u32>), ReactionRefsError> = refs_mut.partition_mut();
        assert!(matches!(result, Err(ReactionRefsError::Missing { .. })));
    }
}
