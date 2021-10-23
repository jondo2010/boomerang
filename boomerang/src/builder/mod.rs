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

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found.")]
    ReactionGraphCycle,

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

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::SchedulerPoint;

    pub(crate) struct SchedulerDummy {}
    impl SchedulerPoint for SchedulerDummy {
        fn get_start_time(&self) -> &runtime::Instant {
            todo!()
        }
        fn get_logical_time(&self) -> &runtime::Instant {
            todo!()
        }
        fn get_physical_time(&self) -> runtime::Instant {
            todo!()
        }
        fn get_elapsed_logical_time(&self) -> runtime::Duration {
            todo!()
        }
        fn get_elapsed_physical_time(&self) -> runtime::Duration {
            todo!()
        }
        fn get_port_with<T: runtime::PortData, F: FnOnce(&T, bool)>(
            &self,
            _: runtime::PortKey,
            _: F,
        ) {
            todo!()
        }
        fn get_port_with_mut<T: runtime::PortData, F: FnOnce(&mut T, bool) -> bool>(
            &self,
            _: runtime::PortKey,
            _: F,
        ) {
            todo!()
        }
        fn schedule_action<T: runtime::PortData>(
            &self,
            _: runtime::ActionKey,
            _: T,
            _: Option<runtime::Duration>,
        ) {
            todo!()
        }
        fn schedule(&self, _: runtime::Tag, _: runtime::ActionKey) {
            todo!()
        }
        fn shutdown(&self) {
            todo!()
        }
    }

    pub(crate) struct TestReactorDummy;
    impl TestReactorDummy {
        pub fn reaction_dummy<S: SchedulerPoint>(
            &mut self,
            _sched: &S,
            _inputs: &EmptyPart,
            _outputs: &EmptyPart,
            _actions: &EmptyPart,
        ) {
        }
    }
    impl<S: runtime::SchedulerPoint> Reactor<S> for TestReactorDummy {
        type Inputs = EmptyPart;
        type Outputs = EmptyPart;
        type Actions = EmptyPart;
        fn build(
            self,
            _name: &str,
            _env: &mut EnvBuilder<S>,
            _parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
            // Ok((Self, EmptyPart, EmptyPart))
            todo!()
        }

        fn build_parts(
            &self,
            _: &mut EnvBuilder<S>,
            _: runtime::ReactorKey,
        ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
            Ok((
                EmptyPart::default(),
                EmptyPart::default(),
                EmptyPart::default(),
            ))
        }
    }

    pub(crate) struct TestReactor2;
    #[derive(Clone)]
    pub(crate) struct TestReactorInputs {
        p0: runtime::PortKey,
    }
    impl<S: SchedulerPoint> Reactor<S> for TestReactor2 {
        type Inputs = TestReactorInputs;
        type Outputs = EmptyPart;
        type Actions = EmptyPart;

        fn build(
            self,
            _name: &str,
            _env: &mut EnvBuilder<S>,
            _parent: Option<runtime::ReactorKey>,
        ) -> Result<(runtime::ReactorKey, Self::Inputs, Self::Outputs), BuilderError> {
            todo!()
        }

        fn build_parts(
            &self,
            env: &mut EnvBuilder<S>,
            reactor_key: runtime::ReactorKey,
        ) -> Result<(Self::Inputs, Self::Outputs, Self::Actions), BuilderError> {
            let p0 = env.add_port::<()>("p0", PortType::Input, reactor_key)?;
            Ok((
                Self::Inputs { p0 },
                EmptyPart::default(),
                EmptyPart::default(),
            ))
        }
    }
}
