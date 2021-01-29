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
    ReactorKeyNotFound(runtime::ReactorKey),

    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(runtime::BasePortKey),

    #[error("Reaction Key not found: {}", 0)]
    ReactionKeyNotFound(runtime::ReactionKey),

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
