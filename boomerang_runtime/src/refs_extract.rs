use crate::{BaseAction, BasePort, Refs, RefsMut};
use std::{mem::MaybeUninit, ptr};
use variadics_please::all_tuples;

/// Errors that can occur while extracting typed references for reaction execution.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum ReactionRefsError {
    #[error("missing {kind} reference during extraction")]
    Missing { kind: &'static str },

    #[error("type mismatch when extracting {kind}: expected {expected}, found {found}")]
    TypeMismatch {
        kind: &'static str,
        expected: &'static str,
        found: &'static str,
    },

    #[error("partitioning left {remaining} unconsumed references during extraction")]
    Remaining { remaining: usize },
}

impl ReactionRefsError {
    pub fn missing(kind: &'static str) -> Self {
        ReactionRefsError::Missing { kind }
    }

    pub fn type_mismatch(kind: &'static str, expected: &'static str, found: &'static str) -> Self {
        ReactionRefsError::TypeMismatch { kind, expected, found }
    }

    pub fn destructure_remaining(remaining: usize) -> Self {
        ReactionRefsError::Remaining { remaining }
    }
}

/// References to ports and actions for executing a reaction.
pub struct ReactionRefs<'store> {
    pub ports: Refs<'store, dyn BasePort>,
    pub ports_mut: RefsMut<'store, dyn BasePort>,
    pub actions: RefsMut<'store, dyn BaseAction>,
}

pub trait ReactionRefsExtract: Copy + 'static {
    type Ref<'store>
    where
        Self: 'store;
    fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Result<Self::Ref<'store>, ReactionRefsError>;
}

// Blanket impl for arrays of `ReactionRefsExtract` types
impl<T: ReactionRefsExtract, const N: usize> ReactionRefsExtract for [T; N] {
    type Ref<'store>
        = [T::Ref<'store>; N]
    where
        Self: 'store;
    fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Result<Self::Ref<'store>, ReactionRefsError> {
        // Manual, allocation-free equivalent of std::array::try_from_fn
        let mut array: [MaybeUninit<T::Ref<'store>>; N] = unsafe { MaybeUninit::uninit().assume_init() };

        for idx in 0..N {
            match T::extract(refs) {
                Ok(value) => {
                    array[idx].write(value);
                }
                Err(err) => {
                    // Clean up any initialized slots before returning the error
                    for slot in array.iter_mut().take(idx) {
                        unsafe { ptr::drop_in_place(slot.as_mut_ptr()) };
                    }
                    return Err(err);
                }
            }
        }

        // SAFETY: every element was written above, so transmuting to initialized array is sound.
        let initialized = unsafe { ptr::read(&array as *const _ as *const [T::Ref<'store>; N]) };
        Ok(initialized)
    }
}

// Blanket impl for slices of `ReactionRefsExtract` types
macro_rules! impl_reaction_refs_extract {
    ($($T:ident),*) => {
        impl<$($T,)*> ReactionRefsExtract for ($($T,)*)
        where
            $($T: ReactionRefsExtract,)*
        {
            type Ref<'store> = ($($T::Ref<'store>,)*) where $($T: 'store,)*;
            fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Result<Self::Ref<'store>, ReactionRefsError> {
                Ok(($($T::extract(refs)?,)*))
            }
        }
    };
}

all_tuples!(impl_reaction_refs_extract, 1, 10, T);
