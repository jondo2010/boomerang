mod action;
mod env;
#[cfg(feature = "visualization")]
pub mod graphviz;
mod macros;
mod port;
mod reaction;
mod reactor;

use crate::runtime;

pub use action::*;
pub use env::*;
pub use macros::*;
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
    ActionKeyNotFound(runtime::ActionKey),

    #[error("ReactorKey not found: {}", 0)]
    ReactorKeyNotFound(runtime::ReactorKey),

    #[error("PortKey not found: {}", 0)]
    PortKeyNotFound(runtime::PortKey),

    #[error("ReactionKey not found: {}", 0)]
    ReactionKeyNotFound(runtime::ReactionKey),

    #[error("A Port named '{}' was not found.", 0)]
    NamedPortNotFound(String),

    #[error("An Action named '{}' was not found.", 0)]
    NamedActionNotFound(String),

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found.")]
    ReactionGraphCycle { what: runtime::ReactionKey },

    #[error("A cycle in the Reactor graph was found.")]
    ReactorGraphCycle { what: runtime::ReactorKey },

    #[error("Error binding ports ({:?}->{:?}): {}", port_a_key, port_b_key, what)]
    PortBindError {
        port_a_key: runtime::PortKey,
        port_b_key: runtime::PortKey,
        what: String,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
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

#[cfg(feature = "disabled")]
mod tests {
    use super::*;

    #[derive(Debug)]
    pub(crate) struct TestReactorDummy;
    impl Reactor for TestReactorDummy {
        type BuilderParts = EmptyPart;
        fn build(
            self,
            _name: &str,
            _env: &mut EnvBuilder,
            _parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::BuilderParts), BuilderError> {
            // Ok((Self, EmptyPart, EmptyPart))
            todo!()
        }

        fn build_parts(
            &self,
            _: &mut EnvBuilder,
            _: runtime::ReactorKey,
        ) -> Result<Self::BuilderParts, BuilderError> {
            Ok(EmptyPart::default())
        }
    }

    #[derive(Debug)]
    pub(crate) struct TestReactor2;
    #[derive(Clone)]
    pub(crate) struct TestReactorPorts {
        p0: BuilderPortKey,
    }
    impl ReactorPart for TestReactorPorts {
        fn build(
            _env: &mut EnvBuilder,
            _reactor_key: runtime::ReactorKey,
        ) -> Result<Self, BuilderError> {
            todo!()
        }
    }
    impl Reactor for TestReactor2 {
        type BuilderParts = TestReactorPorts;

        fn build(
            self,
            _name: &str,
            _env: &mut EnvBuilder,
            _parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::BuilderParts), BuilderError> {
            todo!()
        }

        fn build_parts(
            &self,
            env: &mut EnvBuilder,
            reactor_key: runtime::ReactorKey,
        ) -> Result<Self::BuilderParts, BuilderError> {
            let p0 = env.add_port::<()>("p0", PortType::Input, reactor_key)?;
            Ok(Self::BuilderParts { p0 })
        }
    }
}
