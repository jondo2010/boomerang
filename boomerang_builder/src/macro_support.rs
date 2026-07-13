//! This module provides traits and implementations for building reactors
use crate::{
    runtime, Assembly, AssemblyError, AssemblyModeKey, AssemblyReactorKey, ReactionDeclaration,
    ReactorContext, ReactorPlacement,
};

pub trait Reactor<State: runtime::ReactorData = ()>: Sized {
    type Ports;
    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        name: &str,
        state: State,
        parent: Option<AssemblyReactorKey>,
        scope_mode: Option<AssemblyModeKey>,
        bank_info: Option<runtime::BankInfo>,
        is_enclave: bool,
        assembly: &mut Assembly,
    ) -> Result<Self::Ports, AssemblyError> {
        self.build_with_placement(
            name,
            state,
            parent,
            scope_mode,
            bank_info,
            ReactorPlacement::from(is_enclave),
            assembly,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_with_placement(
        &self,
        name: &str,
        state: State,
        parent: Option<AssemblyReactorKey>,
        scope_mode: Option<AssemblyModeKey>,
        bank_info: Option<runtime::BankInfo>,
        placement: ReactorPlacement,
        assembly: &mut Assembly,
    ) -> Result<Self::Ports, AssemblyError>;
}

impl<F, State, Ports> Reactor<State> for F
where
    F: Fn(
            /*name*/ &str,
            /*state*/ State,
            /*parent*/ Option<AssemblyReactorKey>,
            /*scope_mode*/ Option<AssemblyModeKey>,
            /*bank_info*/ Option<boomerang_runtime::BankInfo>,
            /*placement*/ ReactorPlacement,
            /*assembly*/ &mut Assembly,
        ) -> Result<Ports, AssemblyError>
        + 'static,
    State: runtime::ReactorData,
{
    type Ports = Ports;
    fn build_with_placement(
        &self,
        name: &str,
        state: State,
        parent: Option<AssemblyReactorKey>,
        scope_mode: Option<AssemblyModeKey>,
        bank_info: Option<boomerang_runtime::BankInfo>,
        placement: ReactorPlacement,
        assembly: &mut Assembly,
    ) -> Result<Self::Ports, AssemblyError> {
        (self)(
            name, state, parent, scope_mode, bank_info, placement, assembly,
        )
    }
}

/// ReactorPorts is implemented for the Ports struct of a Reactor. This trait is typically automatically derived.
pub trait ReactorPorts {
    /// The fields of the Ports struct (e.g. the ports)
    type Fields;
    /// Build the reactor with the given closure
    fn build_with<F, S>(f: F) -> impl Reactor<S, Ports = Self>
    where
        F: Fn(&mut ReactorContext<'_, S>, Self::Fields) -> Result<(), AssemblyError> + 'static,
        S: runtime::ReactorData;
}

impl<S: runtime::ReactorData> ReactorContext<'_, S> {
    pub fn add_reaction(&mut self, name: Option<&str>) -> ReactionDeclaration<'_, S> {
        let reactor_key = self.key();
        let current_mode = self.current_mode();
        let builder = ReactionDeclaration::new(name, reactor_key, self.assembly());
        if let Some(mode) = current_mode {
            builder.in_mode_scope(mode)
        } else {
            builder
        }
    }

    pub fn add_child_reactor<ChildState, R>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
        is_enclave: bool,
    ) -> Result<R::Ports, AssemblyError>
    where
        ChildState: runtime::ReactorData,
        R: Reactor<ChildState>,
    {
        self.add_child_reactor_with_placement(reactor, name, state, is_enclave)
    }

    pub fn add_child_reactor_with_placement<ChildState, R>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
        placement: impl Into<ReactorPlacement>,
    ) -> Result<R::Ports, AssemblyError>
    where
        ChildState: runtime::ReactorData,
        R: Reactor<ChildState>,
    {
        let scope_mode = self.current_mode();
        reactor.build_with_placement(
            name,
            state,
            Some(self.key()),
            scope_mode,
            None,
            placement.into(),
            self.assembly(),
        )
    }

    #[cfg(feature = "federated")]
    pub fn add_child_federate<ChildState, R>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
    ) -> Result<R::Ports, AssemblyError>
    where
        ChildState: runtime::ReactorData,
        R: Reactor<ChildState>,
    {
        self.add_child_reactor_with_placement(
            reactor,
            name,
            state,
            ReactorPlacement::federate(name),
        )
    }

    pub fn add_child_reactors<R, ChildState, const N: usize>(
        &mut self,
        reactor: R,
        name: &str,
        state: ChildState,
        is_enclave: bool,
    ) -> Result<[R::Ports; N], AssemblyError>
    where
        R: Reactor<ChildState>,
        ChildState: runtime::ReactorData + Clone,
    {
        let scope_mode = self.current_mode();
        let reactors = (0..N)
            .map(|i| {
                reactor.build_with_placement(
                    &format!("{name}_{i}"),
                    state.clone(),
                    Some(self.key()),
                    scope_mode,
                    Some(runtime::BankInfo { idx: i, total: N }),
                    ReactorPlacement::from(is_enclave),
                    self.assembly(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        reactors
            .try_into()
            .map_err(|_| AssemblyError::InternalError("Error converting Vec to array".to_owned()))
    }
}
