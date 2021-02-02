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
    DuplicatePortDefinition {
        reactor_name: String,
        port_name: String,
    },

    #[error("Duplicate Action Definition: {}.{}", reactor_name, action_name)]
    DuplicateActionDefinition {
        reactor_name: String,
        action_name: String,
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

    #[error("Error binding ports ({:?}->{:?}): {}", port_a_key, port_b_key, what)]
    PortBindError {
        port_a_key: runtime::BasePortKey,
        port_b_key: runtime::BasePortKey,
        what: String,
    },
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

#[cfg(test)]
mod tests {
    use crate::runtime::PortKey;

    use super::*;

    pub(crate) struct TestReactorDummy;
    impl Reactor for TestReactorDummy {
        type Inputs = EmptyPart;
        type Outputs = EmptyPart;
        type Actions = EmptyPart;
        fn build(
            self,
            name: &str,
            env: &mut EnvBuilder,
            parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
            //Ok((Self, EmptyPart, EmptyPart))
            todo!()
        }
    }

    pub(crate) struct TestReactor2;
    #[derive(Clone)]
    pub(crate) struct TestReactorInputs {
        p0: PortKey<u32>,
    }
    impl ReactorPart for TestReactorInputs {
        fn build(
            env: &mut EnvBuilder,
            reactor_key: runtime::ReactorKey,
        ) -> Result<Self, BuilderError> {
            let p0 = env.add_port("p0", PortType::Input, reactor_key)?;
            Ok(Self { p0 })
        }
    }
    impl Reactor for TestReactor2 {
        type Inputs = TestReactorInputs;
        type Outputs = EmptyPart;
        type Actions = EmptyPart;

        fn build(
            self,
            _name: &str,
            _env: &mut EnvBuilder,
            _parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
            todo!()
        }
    }

    pub(crate) fn foo() {}
}
