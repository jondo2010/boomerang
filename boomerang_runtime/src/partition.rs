//! This module contains partitioning logic for type-erased slices.
//!
//! The partitioning logic is used to destructure a slice of type-erased references into a tuple of concrete types. The
//! partitioning logic is implemented using the `Partition` and `PartitionMut` traits, which are implemented for tuples of
//! concrete types. The `part` and `part_mut` functions are used to perform the dynamic destructuring.
//!
//! # Example
//!
//! ```rust,ignore
//! use boomerang_runtime::partition::{part, part_mut};
//! use boomerang_runtime::PortRef;
//!
//! let ports: Vec<PortRef> = vec![/* ... */];
//!
//! // Destructure the slice of type-erased references into a tuple of concrete types
//! let (port0, port1, array0): (PortRef<i32>, PortRef<bool>, [PortRef<()>; 2]) = part(ports.as_slice()).unwrap();
//! ```

use crate::{InputRef, OutputRef, Port, PortData, PortRef, PortRefMut};

pub trait Partition<'a, T>: Sized {
    fn part(slice: &'a [T]) -> Option<(Self, &'a [T])>;
}

pub trait PartitionMut<'a, T>: Sized {
    fn part_mut(slice: &'a mut [T]) -> Option<(Self, &'a mut [T])>;
}

macro_rules! impl_part_for_tuples {
    ($($S:ident),+) => {
        impl<'a, T, $($S),+> Partition<'a, T> for ($($S),+,)
        where
            $($S: Partition<'a, T>),+
        {
            fn part(
                slice: &'a [T]
            ) -> Option<(Self, &'a [T])> {
                let (elements, rest) = {
                    let mut rest = slice;
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

        impl<'a, T, $($S),+> PartitionMut<'a, T> for ($($S),+,)
        where
            $($S: PartitionMut<'a, T>),+
        {
            fn part_mut(
                slice: &'a mut [T],
            ) -> Option<(Self, &'a mut [T])> {
                let (elements, rest) = {
                    let mut rest = slice;
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

/// Wrapper function for dynamic destructuring.
///
/// This function is used to destructure a slice of type-erased references into a tuple of concrete types. The function
/// will panic if the slice is not fully consumed.
pub fn partition<'a, T, S>(slice: &'a [T]) -> Option<S>
where
    S: Partition<'a, T>,
{
    if let Some((result, rest)) = S::part(slice) {
        assert!(rest.is_empty(), "Destructuring error");
        Some(result)
    } else {
        None
    }
}

/// Wrapper function for dynamic destructuring with mutable references.
///
/// This function is used to destructure a slice of type-erased mutable references into a tuple of concrete types. The function
/// will return an error if the slice is not fully consumed.
//pub fn partition_mut<'a, T, S>(slice: &'a mut [T]) -> Option<S>
pub fn partition_mut<'a, T, S>(slice: &'a mut [T]) -> Option<S>
where
    S: PartitionMut<'a, T>,
{
    if let Some((result, rest)) = S::part_mut(slice) {
        assert!(rest.is_empty(), "Destructuring error");
        Some(result)
    } else {
        None
    }
}

impl<'a, T: PortData> From<PortRef<'a>> for InputRef<'a, T> {
    fn from(port: PortRef<'a>) -> Self {
        InputRef::from(
            port.downcast_ref::<Port<T>>()
                .expect("Downcast failed during conversion"),
        )
    }
}

impl<'a, T: PortData> From<&'a mut PortRefMut<'a>> for OutputRef<'a, T> {
    fn from(port: &'a mut PortRefMut<'a>) -> Self {
        OutputRef::from(
            port.downcast_mut::<Port<T>>()
                .expect("Downcast failed during conversion"),
        )
    }
}

// Split for BasePort scalars
impl<'a, P> Partition<'a, PortRef<'a>> for P
where
    P: From<PortRef<'a>>,
{
    fn part(slice: &'a [PortRef<'a>]) -> Option<(Self, &'a [PortRef<'a>])> {
        slice
            .split_first()
            .map(|(&first, rest)| (P::from(first), rest))
    }
}

// SplitMut for BasePort scalars
impl<'a, P> PartitionMut<'a, PortRefMut<'a>> for P
where
    P: From<&'a mut PortRefMut<'a>>,
{
    fn part_mut(slice: &'a mut [PortRefMut<'a>]) -> Option<(Self, &'a mut [PortRefMut<'a>])> {
        slice
            .split_first_mut()
            .map(|(first, rest)| (P::from(first), rest))
    }
}

// Split for BasePort arrays
impl<'a, P, const N: usize> Partition<'a, PortRef<'a>> for [P; N]
where
    P: From<PortRef<'a>>,
{
    fn part(slice: &'a [PortRef<'a>]) -> Option<(Self, &'a [PortRef<'a>])> {
        if slice.len() < N {
            return None;
        }

        slice.split_first_chunk::<N>().map(|(array_slice, rest)| {
            // SAFETY: We know that the slice has the correct length, since split_first_chunk would return None otherwise
            let array = unsafe {
                let mut array = std::mem::MaybeUninit::<[P; N]>::uninit();
                array
                    .assume_init_mut()
                    .iter_mut()
                    .zip(array_slice.iter())
                    .for_each(|(elem, value)| {
                        *elem = P::from(*value);
                    });
                array.assume_init()
            };
            (array, rest)
        })
    }
}

// SplitMut for BasePort arrays
impl<'a, P, const N: usize> PartitionMut<'a, PortRefMut<'a>> for [P; N]
where
    P: From<&'a mut PortRefMut<'a>>,
{
    fn part_mut(slice: &'a mut [PortRefMut<'a>]) -> Option<(Self, &'a mut [PortRefMut<'a>])> {
        if slice.len() < N {
            return None;
        }

        slice
            .split_first_chunk_mut::<N>()
            .map(|(array_slice, rest)| {
                // SAFETY: We know that the slice has the correct length, since split_first_chunk would return None otherwise
                let array = unsafe {
                    let mut array = std::mem::MaybeUninit::<[P; N]>::uninit();
                    array
                        .assume_init_mut()
                        .iter_mut()
                        .zip(array_slice.iter_mut())
                        .for_each(|(elem, value)| {
                            *elem = P::from(value);
                        });
                    array.assume_init()
                };
                (array, rest)
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::{BasePort, Port};

    use super::*;

    #[test]
    fn test_split() {
        // Create some concrete ports
        let mut ports: tinymap::TinyMap<_, Box<dyn BasePort>> = tinymap::TinyMap::new();
        let k0 = ports.insert_with_key(|key| Box::new(Port::<i32>::new("p0".to_owned(), key)));
        let k1 = ports.insert_with_key(|key| Box::new(Port::<u32>::new("p1".to_owned(), key)));
        let k2 = ports.insert_with_key(|key| Box::new(Port::<bool>::new("p2a".to_owned(), key)));
        let k3 = ports.insert_with_key(|key| Box::new(Port::<bool>::new("p2b".to_owned(), key)));

        // Test the split function
        let (refs, _) = ports.iter_many_unchecked_split([k0, k1, k2, k3], []);
        let refs: Vec<&dyn BasePort> = refs.into_iter().map(AsRef::as_ref).collect();

        let (p0, p1, p2a): (InputRef<i32>, InputRef<u32>, [InputRef<bool>; 2]) =
            partition(refs.as_slice()).unwrap();
        assert_eq!(p0.name(), "p0");
        assert_eq!(p1.name(), "p1");
        assert_eq!(p2a[0].name(), "p2a");
        assert_eq!(p2a[1].name(), "p2b");

        // Test the split_mut function
        let (_, refs_mut) = ports.iter_many_unchecked_split([k0], [k1, k2, k3]);
        let mut refs_mut: Vec<&mut dyn BasePort> =
            refs_mut.into_iter().map(AsMut::as_mut).collect();

        let (p1, p2a): (OutputRef<u32>, [OutputRef<bool>; 2]) =
            partition_mut(refs_mut.as_mut_slice()).unwrap();

        assert_eq!(p1.name(), "p1");
        assert_eq!(p2a[0].name(), "p2a");
        assert_eq!(p2a[1].name(), "p2b");
    }
}
