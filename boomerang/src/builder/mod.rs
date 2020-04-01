use derive_more::Display;

mod action;
mod env;
mod port;
mod reaction;
mod reactor;

pub use action::*;
pub use env::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;

use crate::runtime;

#[derive(Display, Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub struct ReactorTypeBuilderIndex(usize);
#[derive(Display, Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub struct ReactorTypeIndex(usize);
#[derive(Display, Debug, Copy, Clone, Eq, PartialEq)]
pub struct ReactorTypeBuilderChildRefIndex(usize);

#[derive(thiserror::Error, Debug, Eq, PartialEq)]
pub enum BuilderError {
    #[error("Duplicate Port Definition: {}.{}", reactor_name, port_name)]
    DuplicatedPortDefinition {
        reactor_name: String,
        port_name: String,
    },
    #[error("Port Definition not found: {}.{}", reactor_name, port_name)]
    PortNotFound {
        reactor_name: String,
        port_name: String,
    },

    #[error("ReactorType Index not found: {}", 0)]
    ReactorTypeIndexNotFound(ReactorTypeIndex),

    #[error("Port Index not found: {}", 0)]
    PortIndexNotFound(runtime::PortIndex),

    #[error("Reaction Index not found: {}", 0)]
    ReactionIndexNotFound(runtime::ReactionIndex),

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found.")]
    ReactionGraphCycle,
}

trait TupleSlice {
    type Item;
    fn tuple_at_mut(&mut self, idxs: (usize, usize)) -> (&mut Self::Item, &mut Self::Item);
}

impl<T: Sized> TupleSlice for [T] {
    type Item = T;
    fn tuple_at_mut(&mut self, idx: (usize, usize)) -> (&mut Self::Item, &mut Self::Item) {
        let len = self.len();
        assert!(idx.0 != idx.1 && idx.0 <= len && idx.1 <= len);
        // SAFETY: [ptr; idx0] and [ptr; idx1] are non-overlapping and within `self`
        let ptr = self.as_mut_ptr();
        let slice = std::ptr::slice_from_raw_parts_mut(ptr, len);
        unsafe { (&mut (*slice)[idx.0], &mut (*slice)[idx.1]) }
    }
}
