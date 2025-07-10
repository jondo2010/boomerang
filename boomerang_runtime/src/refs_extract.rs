use crate::{BaseAction, BasePort, Refs, RefsMut};

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
    fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Self::Ref<'store>;
}

// Blanket impl for arrays of `ReactionRefsExtract` types
impl<T: ReactionRefsExtract, const N: usize> ReactionRefsExtract for [T; N] {
    type Ref<'store>
        = [T::Ref<'store>; N]
    where
        Self: 'store;
    fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Self::Ref<'store> {
        std::array::from_fn(|_i| T::extract(refs))
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
            fn extract<'store>(refs: &mut ReactionRefs<'store>) -> Self::Ref<'store> {
                ($($T::extract(refs),)*)
            }
        }
    };
}

impl_reaction_refs_extract!(A);
impl_reaction_refs_extract!(A, B);
impl_reaction_refs_extract!(A, B, C);
impl_reaction_refs_extract!(A, B, C, D);
impl_reaction_refs_extract!(A, B, C, D, E);
impl_reaction_refs_extract!(A, B, C, D, E, F);
