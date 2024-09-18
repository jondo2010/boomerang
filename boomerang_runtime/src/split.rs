use crate::BasePort;

pub trait SplitMap<'a, T, F>: Sized
where
    F: FnOnce(&T) -> Self,
{
    fn split_map(slice: &'a [T], f: F) -> Option<(Self, &'a [T])>;
}

impl<'a, T, U, F> SplitMap<'a, T, F> for U
where
    F: FnOnce(&T) -> U,
{
    fn split_map(slice: &'a [T], f: F) -> Option<(U, &'a [T])> {
        slice.split_first().map(|(first, rest)| (f(first), rest))
    }
}

pub trait Split<'a, T>: Sized {
    fn split(slice: &'a [T]) -> Option<(Self, &'a [T])>;
}

pub trait SplitMut<'a, T>: Sized {
    fn dest_mut(slice: &'a mut [T]) -> Option<(Self, &'a mut [T])>;
}

macro_rules! impl_dest_for_tuples {
    ($($S:ident),+) => {
        impl<'a, T, $($S),+> Split<'a, T> for ($($S),+)
        where
            $($S: Split<'a, T>),+
        {
            fn split(
                slice: &'a [T]
            ) -> Option<(Self, &'a [T])> {
                let (elements, rest) = {
                    let mut rest = slice;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $S::split(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+)
                    , rest)
                };
                Some((elements, rest))
            }
        }

        impl<'a, T, $($S),+> SplitMut<'a, T> for ($($S),+)
        where
            $($S: SplitMut<'a, T>),+
        {
            fn dest_mut(
                slice: &'a mut [T],
            ) -> Option<(Self, &'a mut [T])> {
                let (elements, rest) = {
                    let mut rest = slice;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $S::dest_mut(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+)
                    , rest)
                };
                Some((elements, rest))
            }
        }
    };
}

// Implement the macro for tuples of length 2,3,4,5
impl_dest_for_tuples!(T0, T1);
impl_dest_for_tuples!(T0, T1, T2);
impl_dest_for_tuples!(T0, T1, T2, T3);
impl_dest_for_tuples!(T0, T1, T2, T3, T4);

/// Wrapper function for dynamic destructuring.
///
/// This function is used to destructure a slice of type-erased references into a tuple of concrete types. The function
/// will panic if the slice is not fully consumed.
pub fn split<'a, T, S>(slice: &'a [T]) -> Option<S>
where
    S: Split<'a, T>,
{
    if let Some((result, rest)) = S::split(slice) {
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
pub fn split_mut<'a, T, S>(slice: &'a mut [T]) -> Option<S>
where
    S: SplitMut<'a, T>,
{
    if let Some((result, rest)) = S::dest_mut(slice) {
        assert!(rest.is_empty(), "Destructuring error");
        Some(result)
    } else {
        None
    }
}

// Split for BasePort scalars
impl<'a, T: BasePort + 'static> Split<'a, &'a dyn BasePort> for &'a T {
    fn split(
        slice: &'a [&'a (dyn BasePort + 'static)],
    ) -> Option<(Self, &'a [&'a (dyn BasePort + 'static)])> {
        slice
            .split_first()
            .and_then(|(first, rest)| first.downcast_ref::<T>().map(|value| (value, rest)))
    }
}

// SplitMut for BasePort scalars
impl<'a, T: BasePort + 'static> SplitMut<'a, &'a mut dyn BasePort> for &'a mut T {
    fn dest_mut(
        slice: &'a mut [&'a mut (dyn BasePort + 'static)],
    ) -> Option<(Self, &'a mut [&'a mut (dyn BasePort + 'static)])> {
        slice
            .split_first_mut()
            .and_then(|(first, rest)| first.downcast_mut().map(|value| (value, rest)))
    }
}

// Split for BasePort arrays
impl<'a, T: BasePort + 'static, const N: usize> Split<'a, &'a dyn BasePort> for [&'a T; N] {
    fn split(
        slice: &'a [&'a (dyn BasePort + 'static)],
    ) -> Option<(Self, &'a [&'a (dyn BasePort + 'static)])> {
        if slice.len() < N {
            return None;
        }

        slice.split_first_chunk::<N>().map(|(array_slice, rest)| {
            // SAFETY: We know that the slice has the correct length, since split_first_chunk would return None otherwise
            let array = unsafe {
                let mut array = std::mem::MaybeUninit::<[&T; N]>::uninit();
                array
                    .assume_init_mut()
                    .iter_mut()
                    .zip(array_slice.iter())
                    .for_each(|(elem, value)| {
                        *elem = value
                            .downcast_ref()
                            .expect("Downcast failed during destructure");
                    });
                array.assume_init()
            };
            (array, rest)
        })
    }
}

// SplitMut for BasePort arrays
impl<'a, T: BasePort, const N: usize> SplitMut<'a, &'a mut dyn BasePort> for [&'a mut T; N] {
    fn dest_mut(
        slice: &'a mut [&'a mut (dyn BasePort + 'static)],
    ) -> Option<(Self, &'a mut [&'a mut (dyn BasePort + 'static)])> {
        if slice.len() < N {
            return None;
        }

        slice
            .split_first_chunk_mut::<N>()
            .map(|(array_slice, rest)| {
                // SAFETY: We know that the slice has the correct length, since split_first_chunk would return None otherwise
                let array = unsafe {
                    let mut array = std::mem::MaybeUninit::<[&mut T; N]>::uninit();
                    array
                        .assume_init_mut()
                        .iter_mut()
                        .zip(array_slice.iter_mut())
                        .for_each(|(elem, value)| {
                            *elem = value
                                .downcast_mut()
                                .expect("Downcast failed during destructure");
                        });
                    array.assume_init()
                };
                (array, rest)
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::Port;

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

        let (p0, p1, p2a): (&Port<i32>, &Port<u32>, [&Port<bool>; 2]) =
            split(refs.as_slice()).unwrap();
        assert_eq!(p0.get_name(), "p0");
        assert_eq!(p1.get_name(), "p1");
        assert_eq!(p2a[0].get_name(), "p2a");
        assert_eq!(p2a[1].get_name(), "p2b");

        // Test the split_mut function
        let (_, refs_mut) = ports.iter_many_unchecked_split([k0], [k1, k2, k3]);
        let mut refs_mut: Vec<&mut dyn BasePort> =
            refs_mut.into_iter().map(AsMut::as_mut).collect();

        let (p1, p2a): (&mut Port<u32>, [&mut Port<bool>; 2]) =
            split_mut(refs_mut.as_mut_slice()).unwrap();

        assert_eq!(p1.get_name(), "p1");
        assert_eq!(p2a[0].get_name(), "p2a");
        assert_eq!(p2a[1].get_name(), "p2b");
    }
}
