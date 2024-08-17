mod action;
mod env;
mod fqn;
mod port;
mod reaction;
mod reactor;
#[cfg(test)]
pub mod tests;

#[cfg(feature = "visualization")]
pub mod graphviz;

pub use action::*;
pub use env::*;
pub use fqn::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;

#[derive(thiserror::Error, Debug)]
pub enum BuilderError {
    #[error("Duplicate Port Definition: {}.{}", reactor_name, port_name)]
    DuplicatePortDefinition {
        reactor_name: String,
        port_name: String,
    },

    #[error("Duplicate Action Definition: {}.{}", reactor_name, action_name)]
    DuplicateActionDefinition {
        reactor_name: String,
        action_name: String,
    },

    #[error("ActionKey not found: {}", 0)]
    ActionKeyNotFound(BuilderActionKey),

    #[error("ReactorKey not found: {}", 0)]
    ReactorKeyNotFound(BuilderReactorKey),

    #[error("PortKey not found: {}", 0)]
    PortKeyNotFound(BuilderPortKey),

    #[error("ReactionKey not found: {}", 0)]
    ReactionKeyNotFound(BuilderReactionKey),

    #[error("A Port named '{0}' was not found.")]
    NamedPortNotFound(String),

    #[error("A Reaction named '{0}' was not found.")]
    NamedReactionNotFound(String),

    #[error("An Action named '{0}' was not found.")]
    NamedActionNotFound(String),

    #[error("A Reactor named '{0}' was not found.")]
    NamedReactorNotFound(String),

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found.")]
    ReactionGraphCycle { what: BuilderReactionKey },

    #[error("A cycle in the Reactor graph was found.")]
    ReactorGraphCycle { what: BuilderReactorKey },

    #[error("Error binding ports ({:?}->{:?}): {}", port_a_key, port_b_key, what)]
    PortBindError {
        port_a_key: BuilderPortKey,
        port_b_key: BuilderPortKey,
        what: String,
    },

    #[error("Error building Reaction: {0}")]
    ReactionBuilderError(String),

    #[error("Invalid fully-qualified name: {0}")]
    InvalidFqn(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::convert::Infallible> for BuilderError {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
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
