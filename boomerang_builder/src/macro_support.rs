//! This module provides traits and implementations for building reactors
use crate::{
    runtime, BuilderError, BuilderReactorKey, EnvBuilder, PartialReactionBuilder,
    ReactorBuilderState,
};

pub trait Reactor<State: runtime::ReactorData = ()>: Sized {
    type Ports;
    fn build(
        &self,
        name: &str,
        state: State,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        is_enclave: bool,
        env: &mut EnvBuilder,
    ) -> Result<Self::Ports, BuilderError>;
}

impl<F, State, Ports> Reactor<State> for F
where
    F: Fn(
            /*name*/ &str,
            /*state*/ State,
            /*parent*/ Option<BuilderReactorKey>,
            /*bank_info*/ Option<boomerang_runtime::BankInfo>,
            /*is_enclave*/ bool,
            /*env*/ &mut EnvBuilder,
        ) -> Result<Ports, BuilderError>
        + 'static,
    State: runtime::ReactorData,
{
    type Ports = Ports;
    fn build(
        &self,
        name: &str,
        state: State,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<boomerang_runtime::BankInfo>,
        is_enclave: bool,
        env: &mut EnvBuilder,
    ) -> Result<Self::Ports, BuilderError> {
        (self)(name, state, parent, bank_info, is_enclave, env)
    }
}

/// ReactorPorts is implemented for the Ports struct of a Reactor. This trait is typically automatically derived.
pub trait ReactorPorts {
    /// The fields of the Ports struct (e.g. the ports)
    type Fields;
    /// Build the reactor with the given closure
    fn build_with<F, S>(f: F) -> impl Reactor<S, Ports = Self>
    where
        F: Fn(&mut ReactorBuilderState<'_, S>, Self::Fields) -> Result<(), BuilderError> + 'static,
        S: runtime::ReactorData;
}

impl<S: runtime::ReactorData> ReactorBuilderState<'_, S> {
    pub fn add_reaction(&mut self, name: Option<&str>) -> PartialReactionBuilder<'_, S> {
        let reactor_key = self.key();
        PartialReactionBuilder::new(name, reactor_key, self.env())
    }

    pub fn add_child_reactor<ChildState, R>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
        is_enclave: bool,
    ) -> Result<R::Ports, BuilderError>
    where
        ChildState: runtime::ReactorData,
        R: Reactor<ChildState>,
    {
        reactor.build(name, state, Some(self.key()), None, is_enclave, self.env())
    }

    pub fn add_child_reactors<R, ChildState, const N: usize>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
        is_enclave: bool,
    ) -> Result<[R::Ports; N], BuilderError>
    where
        R: Reactor<ChildState>,
        ChildState: runtime::ReactorData + Clone,
    {
        let reactors = (0..N)
            .map(|i| {
                reactor.build(
                    &format!("{name}_{i}"),
                    state.clone(),
                    Some(self.key()),
                    Some(runtime::BankInfo { idx: i, total: N }),
                    is_enclave,
                    self.env(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        reactors
            .try_into()
            .map_err(|_| BuilderError::InternalError("Error converting Vec to array".to_owned()))
    }
}
