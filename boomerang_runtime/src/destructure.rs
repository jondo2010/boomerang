use downcast_rs::Downcast;

use crate::RuntimeError;

trait Dest<'a>: Sized {
    fn dest(slice: &'a [&dyn Downcast]) -> Result<(Self, &'a [&'a dyn Downcast]), RuntimeError>;
}

trait DestMut<'a>: Sized {
    fn dest_mut(
        slice: &'a mut [&'a mut dyn Downcast],
    ) -> Result<(Self, &'a mut [&'a mut dyn Downcast]), RuntimeError>;
}

// Destructuring for scalars (single elements)
impl<'a, T: Downcast> Dest<'a> for &'a T {
    fn dest(slice: &'a [&dyn Downcast]) -> Result<(Self, &'a [&'a dyn Downcast]), RuntimeError> {
        let (elem, rest) = slice
            .split_first()
            .ok_or_else(|| RuntimeError::DestrError)?;
        let elem = elem
            .as_any()
            .downcast_ref()
            .ok_or_else(|| RuntimeError::DestrError)?;
        Ok((elem, rest))
    }
}

impl<'a, T: Downcast> DestMut<'a> for &'a mut T {
    fn dest_mut(
        slice: &'a mut [&'a mut dyn Downcast],
    ) -> Result<(Self, &'a mut [&'a mut dyn Downcast]), RuntimeError> {
        let (elem, rest) = slice
            .split_first_mut()
            .ok_or_else(|| RuntimeError::DestrError)?;
        let elem = elem
            .as_any_mut()
            .downcast_mut()
            .ok_or_else(|| RuntimeError::DestrError)?;
        Ok((elem, rest))
    }
}

// Destructuring for fixed-size arrays
impl<'a, T: Downcast, const N: usize> Dest<'a> for [&'a T; N] {
    fn dest(slice: &'a [&dyn Downcast]) -> Result<(Self, &'a [&'a dyn Downcast]), RuntimeError> {
        if slice.len() < N {
            return Err(RuntimeError::DestrError);
        }

        let (array_slice, rest) = slice
            .split_first_chunk::<N>()
            .ok_or_else(|| RuntimeError::DestrError)?;

        //let (array_slice, rest) = slice.split_at(N);
        // SAFETY: We know that the slice has the correct length, since split_at would panic otherwise
        let mut array: [&T; N] = unsafe { std::mem::MaybeUninit::uninit().assume_init() };
        for (i, elem) in array_slice.iter().enumerate() {
            array[i] = elem
                .as_any()
                .downcast_ref()
                .ok_or_else(|| RuntimeError::DestrError)?;
        }

        Ok((array, rest))
    }
}

impl<'a, T: Downcast, const N: usize> DestMut<'a> for [&'a mut T; N] {
    fn dest_mut(
        slice: &'a mut [&'a mut dyn Downcast],
    ) -> Result<(Self, &'a mut [&'a mut dyn Downcast]), RuntimeError> {
        if slice.len() < N {
            return Err(RuntimeError::DestrError);
        }

        let (array_slice, rest) = slice
            .split_first_chunk_mut::<N>()
            .ok_or_else(|| RuntimeError::DestrError)?;
        // SAFETY: We know that the slice has the correct length, since split_at would panic otherwise
        let mut array: [&mut T; N] = unsafe { std::mem::MaybeUninit::uninit().assume_init() };
        for (i, elem) in array_slice.iter_mut().enumerate() {
            array[i] = elem
                .as_any_mut()
                .downcast_mut()
                .ok_or_else(|| RuntimeError::DestrError)?;
        }
        Ok((array, rest))
    }
}

macro_rules! impl_dest_for_tuples {
    ($($T:ident),+) => {
        impl<'a, $($T),+> Dest<'a> for ($($T),+)
        where
            $($T: Dest<'a>),+
        {
            fn dest(slice: &'a [&dyn Downcast]) -> Result<(Self, &'a [&'a dyn Downcast]), RuntimeError> {
                let (elements, rest) = {
                    let mut rest = slice;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $T::dest(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+)
                    , rest)
                };
                Ok((elements, rest))
            }
        }

        impl<'a, $($T),+> DestMut<'a> for ($($T),+)
        where
            $($T: DestMut<'a>),+
        {
            fn dest_mut(slice: &'a mut [&'a mut dyn Downcast]) -> Result<(Self, &'a mut [&'a mut dyn Downcast]), RuntimeError> {
                let (elements, rest) = {
                    let mut rest = slice;
                    (
                        ($(
                            {
                                let (elem, new_rest) = $T::dest_mut(rest)?;
                                rest = new_rest;
                                elem
                            }
                        ),+)
                    , rest)
                };
                Ok((elements, rest))
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
/// will return an error if the slice is not fully consumed.
pub fn destructure<'a, T>(slice: &'a [&dyn Downcast]) -> Result<T, RuntimeError>
where
    T: Dest<'a>,
{
    let (result, rest) = T::dest(slice)?;
    if rest.len() > 0 {
        return Err(RuntimeError::DestrError);
    }
    Ok(result)
}

/// Wrapper function for dynamic destructuring with mutable references.
///
/// This function is used to destructure a slice of type-erased mutable references into a tuple of concrete types. The function
/// will return an error if the slice is not fully consumed.
pub fn destructure_mut<'a, T>(slice: &'a mut [&'a mut dyn Downcast]) -> Result<T, RuntimeError>
where
    T: DestMut<'a>,
{
    let (result, rest) = T::dest_mut(slice)?;
    if rest.len() > 0 {
        return Err(RuntimeError::DestrError);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    trait TestTrait: Downcast {}

    downcast_rs::impl_downcast!(TestTrait);

    struct TypeA;
    struct TypeB;
    struct TypeC;

    #[test]
    fn test_destructure_success() -> Result<(), Box<dyn Error>> {
        // Create some concrete instances
        let a = TypeA;
        let b1 = TypeB;
        let b2 = TypeB;
        let b3 = TypeB;
        let c = TypeC;

        // Create a store with type-erased references
        let store: Vec<&dyn Downcast> = vec![&a, &b1, &b2, &b3, &c];

        // Destructure into a heterogeneous tuple with any combination of scalars and arrays
        let (scalar0, array1, scalar1): (&TypeA, [&TypeB; 3], &TypeC) =
            destructure(store.as_slice())?;

        assert_eq!(scalar0 as *const _, &a as *const _);
        assert_eq!(array1[0] as *const _, &b1 as *const _);
        assert_eq!(array1[1] as *const _, &b2 as *const _);
        assert_eq!(array1[2] as *const _, &b3 as *const _);
        assert_eq!(scalar1 as *const _, &c as *const _);

        Ok(())
    }

    #[test]
    fn test_destructure_error() {
        // Create some concrete instances
        let a = TypeA;
        let b1 = TypeB;
        let b2 = TypeB;

        // Create a store with type-erased references
        let store: Vec<&dyn Downcast> = vec![&a, &b1, &b2];

        // Attempt to destructure into a tuple that requires more elements than provided
        let result: Result<(&TypeA, [&TypeB; 3]), RuntimeError> = destructure(store.as_slice());

        assert!(result.is_err());
    }

    #[test]
    fn test_destructure_mut() -> Result<(), Box<dyn Error>> {
        // Create some concrete instances
        let mut a = TypeA;
        let mut b1 = TypeB;
        let mut b2 = TypeB;
        let mut b3 = TypeB;
        let mut c = TypeC;

        // Create a store with type-erased references
        let mut store: Vec<&mut dyn Downcast> = vec![&mut a, &mut b1, &mut b2, &mut b3, &mut c];

        // Destructure into a heterogeneous tuple with any combination of scalars and arrays
        let (scalar0, array1, scalar1): (&mut TypeA, [&mut TypeB; 3], &mut TypeC) =
            destructure_mut(store.as_mut_slice())?;

        /*
        assert_eq!(scalar0 as *const _, &mut a as *const _);
        assert_eq!(array1[0] as *const _, &mut b1 as *const _);
        assert_eq!(array1[1] as *const _, &mut b2 as *const _);
        assert_eq!(array1[2] as *const _, &mut b3 as *const _);
        assert_eq!(scalar1 as *const _, &mut c as *const _);
        */

        Ok(())
    }
}
