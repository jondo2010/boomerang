use std::fmt::Debug;

use super::{
    ActionTag, ActionType, Assembly, AssemblyActionKey, AssemblyError, AssemblyFqn,
    AssemblyPortKey, AssemblyReactionKey, Input, Logical, ModeEffectSpec, Output, Physical,
    PortBank, PortTag, TimerActionKey, TimerSpec, TypedActionKey, TypedPortKey,
};
use crate::{runtime, AssemblyModeKey, ModeKind};
use slotmap::SecondaryMap;

slotmap::new_key_type! {
    pub struct AssemblyReactorKey;
}

#[cfg(feature = "federated")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederateSpec {
    pub id: String,
    pub transient: bool,
}

#[cfg(feature = "federated")]
impl FederateSpec {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            transient: false,
        }
    }

    pub fn transient(mut self, transient: bool) -> Self {
        self.transient = transient;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactorPlacement {
    Local,
    Enclave,
    #[cfg(feature = "federated")]
    Federate(FederateSpec),
}

impl ReactorPlacement {
    pub fn starts_enclave(&self) -> bool {
        match self {
            ReactorPlacement::Local => false,
            ReactorPlacement::Enclave => true,
            #[cfg(feature = "federated")]
            ReactorPlacement::Federate(_) => true,
        }
    }

    #[cfg(feature = "federated")]
    pub fn federate(id: impl Into<String>) -> Self {
        ReactorPlacement::Federate(FederateSpec::new(id))
    }

    #[cfg(feature = "federated")]
    pub fn federate_spec(&self) -> Option<&FederateSpec> {
        match self {
            ReactorPlacement::Federate(spec) => Some(spec),
            _ => None,
        }
    }
}

impl From<bool> for ReactorPlacement {
    fn from(is_enclave: bool) -> Self {
        if is_enclave {
            ReactorPlacement::Enclave
        } else {
            ReactorPlacement::Local
        }
    }
}

impl petgraph::graph::GraphIndex for AssemblyReactorKey {
    fn index(&self) -> usize {
        self.0.as_ffi() as usize
    }

    fn is_node_index() -> bool {
        true
    }
}

pub(super) struct ReactorState<T: runtime::ReactorData>(T);

pub(super) trait BaseReactorState: Debug {
    fn into_runtime(self: Box<Self>, name: &str) -> Box<dyn runtime::BaseReactor>;
}

impl<T: runtime::ReactorData> BaseReactorState for ReactorState<T> {
    fn into_runtime(self: Box<Self>, name: &str) -> Box<dyn runtime::BaseReactor> {
        runtime::Reactor::new(name, self.0).boxed()
    }
}

impl<T: runtime::ReactorData> Debug for ReactorState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(&format!("ReactorState<{}>", std::any::type_name::<T>()))
            .finish()
    }
}

/// `ParentReactorSpec` is implemented for Reactor elements that can have a parent Reactor
pub trait ParentReactorSpec {
    fn parent_reactor_key(&self) -> Option<AssemblyReactorKey>;
}

/// `ReactorSpec` is the assembly-side definition of a reactor and is type-erased.
#[derive(Debug)]
pub struct ReactorSpec {
    /// The instantiated/child name of the Reactor
    name: String,
    /// The user's Reactor
    state: Box<dyn BaseReactorState>,
    /// The top-level/class name of the Reactor
    type_name: String,
    /// Optional parent reactor key
    pub parent_reactor_key: Option<AssemblyReactorKey>,
    /// Enclosing parent mode scope, if this reactor instance was declared inside a mode.
    pub scope_mode: Option<AssemblyModeKey>,
    /// Reactions in this ReactorType
    pub reactions: SecondaryMap<AssemblyReactionKey, ()>,
    /// Modes in this Reactor
    pub modes: SecondaryMap<AssemblyModeKey, ()>,
    /// Ports in this Reactor
    pub ports: SecondaryMap<AssemblyPortKey, ()>,
    /// Actions in this Reactor
    pub actions: SecondaryMap<AssemblyActionKey, ()>,
    /// The bank info of the bank that this Reactor belongs to, if any.
    pub bank_info: Option<runtime::BankInfo>,
    /// Placement metadata for this Reactor instance.
    pub placement: ReactorPlacement,
    /// Whether this Reactor is an enclave
    pub is_enclave: bool,
    /// Initial mode for this reactor
    pub initial_mode: Option<AssemblyModeKey>,
}

impl ParentReactorSpec for ReactorSpec {
    fn parent_reactor_key(&self) -> Option<AssemblyReactorKey> {
        self.parent_reactor_key
    }
}

impl ReactorSpec {
    /// Create a new `ReactorSpec` with the given parameters.
    pub fn new<S: runtime::ReactorData>(
        name: &str,
        type_name: &'static str,
        reactor_state: S,
        parent: Option<AssemblyReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        placement: impl Into<ReactorPlacement>,
    ) -> Self {
        let placement = placement.into();
        let is_enclave = placement.starts_enclave();
        Self {
            name: name.into(),
            state: Box::new(ReactorState(reactor_state)),
            type_name: type_name.into(),
            parent_reactor_key: parent,
            scope_mode: None,
            reactions: SecondaryMap::new(),
            modes: SecondaryMap::new(),
            ports: SecondaryMap::new(),
            actions: SecondaryMap::new(),
            bank_info,
            placement,
            is_enclave,
            initial_mode: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bank_info(&self) -> Option<&runtime::BankInfo> {
        self.bank_info.as_ref()
    }

    pub fn placement(&self) -> &ReactorPlacement {
        &self.placement
    }

    pub fn is_enclave(&self) -> bool {
        self.is_enclave
    }

    #[cfg(feature = "federated")]
    pub fn federate_spec(&self) -> Option<&FederateSpec> {
        self.placement.federate_spec()
    }

    #[allow(dead_code)] // TODO: use or remove this
    pub fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this [`ReactorSpec`] into a [`Box<dyn runtime::BaseReactor>`]
    pub fn into_runtime(self, name: &str) -> Box<dyn runtime::BaseReactor> {
        self.state.into_runtime(name)
    }
}

/// Declaration context used to record a `ReactorSpec` from user or generated code.
#[derive(Debug)]
pub struct ReactorContext<'a, S: runtime::ReactorData = ()> {
    /// The assembly key of this reactor declaration.
    reactor_key: AssemblyReactorKey,
    assembly: &'a mut Assembly,
    startup_action: TypedActionKey,
    shutdown_action: TypedActionKey,
    current_mode: Option<AssemblyModeKey>,
    phantom: std::marker::PhantomData<S>,
}

impl<'a, S: runtime::ReactorData> ReactorContext<'a, S> {
    pub(super) fn new(
        name: &str,
        parent: Option<AssemblyReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        reactor_state: S,
        placement: impl Into<ReactorPlacement>,
        assembly: &'a mut Assembly,
    ) -> Self {
        let type_name = std::any::type_name::<S>();
        let reactor_key = assembly.reactor_specs.insert(ReactorSpec::new(
            name,
            type_name,
            reactor_state,
            parent,
            bank_info,
            placement,
        ));

        let startup_action = assembly
            .add_startup_action("__startup", reactor_key)
            .expect("Duplicate startup Action?");

        let shutdown_action = assembly
            .add_shutdown_action("__shutdown", reactor_key)
            .expect("Duplicate shutdown Action?");

        Self {
            reactor_key,
            assembly,
            startup_action,
            shutdown_action,
            current_mode: None,
            phantom: std::marker::PhantomData,
        }
    }

    /// Create a new `ReactorContext` for a pre-existing reactor
    pub(super) fn from_pre_existing(
        reactor_key: AssemblyReactorKey,
        assembly: &'a mut Assembly,
    ) -> Self {
        // Find the startup and shutdown actions for this reactor
        let startup_action = assembly
            .action_specs
            .iter()
            .find(|(_, action)| matches!(action.r#type(), ActionType::Timer(TimerSpec { period, offset }) if period.is_none() && offset.is_none()) && action.reactor_key() == reactor_key)
            .map(|(action_key, _)| action_key)
            .expect("Startup action not found");

        let shutdown_action = assembly
            .action_specs
            .iter()
            .find(|(_, action)| {
                matches!(action.r#type(), ActionType::Shutdown)
                    && action.reactor_key() == reactor_key
            })
            .map(|(action_key, _)| action_key)
            .expect("Shutdown action not found");

        Self {
            reactor_key,
            assembly,
            startup_action: TypedActionKey::from(startup_action),
            shutdown_action: TypedActionKey::from(shutdown_action),
            current_mode: None,
            phantom: std::marker::PhantomData,
        }
    }

    /// Get the [`Assembly`] for this `ReactorSpec`
    pub fn assembly(&mut self) -> &mut Assembly {
        self.assembly
    }

    /// Get the [`AssemblyReactorKey`] for this `ReactorSpec`
    pub fn key(&self) -> AssemblyReactorKey {
        self.reactor_key
    }

    #[doc(hidden)]
    pub fn set_scope_mode(&mut self, mode: AssemblyModeKey) -> Result<(), AssemblyError> {
        let mode_spec = self.assembly.mode_specs.get(mode).ok_or_else(|| {
            AssemblyError::ReactionDeclarationError(format!("Unknown mode key {mode:?}"))
        })?;
        let reactor_spec = &self.assembly.reactor_specs[self.reactor_key];
        if Some(mode_spec.reactor_key) != reactor_spec.parent_reactor_key {
            return Err(AssemblyError::ReactionDeclarationError(format!(
                "Mode '{}' does not enclose reactor '{}'",
                mode_spec.name,
                reactor_spec.name()
            )));
        }
        self.assembly.reactor_specs[self.reactor_key].scope_mode = Some(mode);
        Ok(())
    }

    pub fn current_mode(&self) -> Option<AssemblyModeKey> {
        self.current_mode
    }

    /// Get the startup action for this reactor
    pub fn get_startup_action(&self) -> TypedActionKey {
        self.startup_action
    }

    /// Get the shutdown action for this reactor
    pub fn get_shutdown_action(&self) -> TypedActionKey {
        self.shutdown_action
    }

    /// Add a new timer action to the reactor.
    pub fn add_timer(
        &mut self,
        name: &str,
        spec: TimerSpec,
    ) -> Result<TimerActionKey, AssemblyError> {
        self.assembly
            .add_timer_action_in_scope(name, self.reactor_key, self.current_mode, spec)
    }

    /// Add a new action to the reactor.
    ///
    /// This method records the action in the underlying assembly.
    pub fn add_action<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Q>, AssemblyError> {
        self.assembly.add_action_in_scope::<T, Q>(
            name,
            min_delay,
            self.reactor_key,
            self.current_mode,
        )
    }

    /// Add a new logical action to the reactor.
    ///
    /// This method records a logical action in the underlying assembly.
    pub fn add_logical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Logical>, AssemblyError> {
        self.assembly.add_action_in_scope::<T, Logical>(
            name,
            min_delay,
            self.reactor_key,
            self.current_mode,
        )
    }

    pub fn add_physical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Physical>, AssemblyError> {
        self.assembly.add_action_in_scope::<T, Physical>(
            name,
            min_delay,
            self.reactor_key,
            self.current_mode,
        )
    }

    /// Add a new mode to this reactor.
    pub fn add_mode(
        &mut self,
        name: &str,
        kind: impl Into<ModeKind>,
    ) -> Result<AssemblyModeKey, AssemblyError> {
        self.assembly.add_mode(name, self.reactor_key, kind)
    }

    pub fn mode_effect(
        &self,
        mode: AssemblyModeKey,
        transition: runtime::TransitionKind,
    ) -> Result<ModeEffectSpec, AssemblyError> {
        self.assembly
            .mode_effect(self.reactor_key, mode, transition)
    }

    pub fn reset_mode_effect(
        &self,
        mode: AssemblyModeKey,
    ) -> Result<ModeEffectSpec, AssemblyError> {
        self.mode_effect(mode, runtime::TransitionKind::Reset)
    }

    pub fn history_mode_effect(
        &self,
        mode: AssemblyModeKey,
    ) -> Result<ModeEffectSpec, AssemblyError> {
        self.mode_effect(mode, runtime::TransitionKind::History)
    }

    pub fn in_mode<R>(
        &mut self,
        mode: AssemblyModeKey,
        f: impl FnOnce(&mut Self) -> Result<R, AssemblyError>,
    ) -> Result<R, AssemblyError> {
        let mode_spec = self.assembly.mode_specs.get(mode).ok_or_else(|| {
            AssemblyError::ReactionDeclarationError(format!("Unknown mode key {mode:?}"))
        })?;
        if mode_spec.reactor_key != self.reactor_key {
            return Err(AssemblyError::ReactionDeclarationError(format!(
                "Mode '{}' does not belong to reactor '{}'",
                mode_spec.name,
                self.assembly.reactor_specs[self.reactor_key].name()
            )));
        }
        if self.current_mode.is_some() {
            return Err(AssemblyError::ReactionDeclarationError(
                "Nested mode blocks are not supported".to_owned(),
            ));
        }

        let previous_mode = self.current_mode.replace(mode);
        let result = f(self);
        self.current_mode = previous_mode;
        result
    }

    /// Add a new input port to this reactor.
    pub fn add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        bank_info: Option<runtime::BankInfo>,
    ) -> Result<TypedPortKey<T, Q>, AssemblyError> {
        if self.current_mode.is_some() {
            return Err(AssemblyError::ReactionDeclarationError(format!(
                "Port '{name}' cannot be declared inside a mode"
            )));
        }
        tracing::debug!("Adding port: {name}");
        self.assembly
            .internal_add_port::<T, Q>(name, self.reactor_key, bank_info)
            .map(Into::into)
    }

    /// Adds a bus of input ports to this reactor.
    pub fn add_ports<T: runtime::ReactorData, Q: PortTag, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Q>; N], AssemblyError> {
        let bank = self.add_ports_bank::<T, Q>(name, N)?;
        Ok(bank
            .into_vec()
            .try_into()
            .expect("Error converting Vec to array"))
    }

    /// Adds a runtime-sized bank of ports to this reactor.
    pub fn add_ports_bank<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        len: usize,
    ) -> Result<PortBank<T, Q>, AssemblyError> {
        let mut ports = Vec::with_capacity(len);
        for i in 0..len {
            let port =
                self.add_port::<T, Q>(name, Some(runtime::BankInfo { idx: i, total: len }))?;
            ports.push(port);
        }
        Ok(PortBank::new(ports))
    }

    /// Add a new input port to this reactor.
    pub fn add_input_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Input>, AssemblyError> {
        self.add_port::<T, Input>(name, None)
    }

    /// Add a new output port to this reactor.
    pub fn add_output_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Output>, AssemblyError> {
        self.add_port::<T, Output>(name, None)
    }

    /// Adds a bus of input port(s) to this reactor.
    pub fn add_input_ports<T: runtime::ReactorData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Input>; N], AssemblyError> {
        self.add_ports(name)
    }

    /// Adds a runtime-sized bank of input ports to this reactor.
    pub fn add_input_bank<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        len: usize,
    ) -> Result<PortBank<T, Input>, AssemblyError> {
        self.add_ports_bank(name, len)
    }

    /// Adds a bus of output port(s) to this reactor.
    pub fn add_output_ports<T: runtime::ReactorData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Output>; N], AssemblyError> {
        self.add_ports(name)
    }

    /// Adds a runtime-sized bank of output ports to this reactor.
    pub fn add_output_bank<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        len: usize,
    ) -> Result<PortBank<T, Output>, AssemblyError> {
        self.add_ports_bank(name, len)
    }

    /// Add a new child reactor using a closure to build it.
    pub fn add_child_with<F>(&mut self, f: F) -> Result<AssemblyReactorKey, AssemblyError>
    where
        F: FnOnce(AssemblyReactorKey, &mut Assembly) -> Result<AssemblyReactorKey, AssemblyError>,
    {
        let child = f(self.reactor_key, self.assembly)?;
        if self.assembly.reactor_specs[child].parent_reactor_key != Some(self.reactor_key) {
            return Err(AssemblyError::ReactionDeclarationError(format!(
                "Child declaration returned reactor '{}' that is not contained by '{}'",
                self.assembly.reactor_specs[child].name(),
                self.assembly.reactor_specs[self.reactor_key].name()
            )));
        }
        if let Some(mode) = self.current_mode {
            self.assembly.reactor_specs[child].scope_mode = Some(mode);
        }
        Ok(child)
    }

    /// Connect 2 ports on this reactor. This has the logical meaning of "connecting" `port_a` to
    /// `port_b`.
    pub fn connect_port<T, Q1, Q2, A1, A2>(
        &mut self,
        port_a_key: TypedPortKey<T, Q1, A1>,
        port_b_key: TypedPortKey<T, Q2, A2>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        self.assembly.add_port_connection_in_scope::<T, _, _>(
            port_a_key,
            port_b_key,
            self.current_mode,
            after,
            physical,
        )
    }

    /// Connect multiple ports on this reactor. This has the logical meaning of "connecting"
    /// `ports_from` to `ports_to`.
    pub fn connect_ports<T, Q1, Q2, A1, A2>(
        &mut self,
        ports_from: impl Iterator<Item = TypedPortKey<T, Q1, A1>>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2, A2>>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        let ports_from: Vec<_> = ports_from.collect();
        let ports_to: Vec<_> = ports_to.collect();

        if ports_from.len() != ports_to.len() {
            return Err(AssemblyError::PortConnectionLengthMismatch {
                from: ports_from.len(),
                to: ports_to.len(),
            });
        }

        for (port_from, port_to) in ports_from.into_iter().zip(ports_to) {
            self.connect_port::<T, _, _, _, _>(port_from, port_to, after, physical)?;
        }
        Ok(())
    }

    /// Connect a single source port to every port in the target iterator.
    pub fn connect_broadcast<T, Q1, Q2, A1, A2>(
        &mut self,
        port_from: TypedPortKey<T, Q1, A1>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2, A2>>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        for port_to in ports_to {
            self.connect_port::<T, _, _, _, _>(port_from, port_to, after, physical)?;
        }
        Ok(())
    }

    /// Connect every source port to every target port.
    pub fn connect_cartesian<T, Q1, Q2, A1, A2>(
        &mut self,
        ports_from: impl Iterator<Item = TypedPortKey<T, Q1, A1>>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2, A2>>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        let ports_from: Vec<_> = ports_from.collect();
        let ports_to: Vec<_> = ports_to.collect();

        for port_from in ports_from {
            for port_to in ports_to.iter().copied() {
                self.connect_port::<T, _, _, _, _>(port_from, port_to, after, physical)?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<AssemblyReactorKey, AssemblyError> {
        self.assembly.validate_reactions()?;
        Ok(self.reactor_key)
    }

    /// Find a PhysicalAction globally in the Assembly given its fully-qualified name
    pub fn find_physical_action_by_fqn(
        &self,
        action_fqn: impl Into<AssemblyFqn>,
    ) -> Result<AssemblyActionKey, AssemblyError> {
        self.assembly.find_physical_action_by_fqn(action_fqn)
    }

    /// Add a recorder for the given action key.
    #[cfg(feature = "replay")]
    pub fn add_action_recorder<T, Q>(
        &mut self,
        action_key: TypedActionKey<T, Q>,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + serde::Serialize,
        Q: ActionTag,
    {
        let topic = self.assembly.fqn_for(action_key, false)?.to_string();
        tracing::debug!("Adding recorder for action {topic}",);
        let _ = self
            .add_reaction(Some("recorder"))
            .with_trigger(action_key)
            .with_deferred_reaction_factory(move |runtime_parts| {
                let (enclave_key, action_key) =
                    runtime_parts.aliases.action_aliases[action_key.into()];
                Box::new(
                    runtime::replay::RecorderFn::<T>::new(&topic, enclave_key, action_key).unwrap(),
                )
            })
            .finish()?;

        Ok(())
    }

    /// Add a replayer for the given action key.
    #[cfg(feature = "replay")]
    pub fn add_action_replayer<T, Q>(
        &mut self,
        action_key: TypedActionKey<T, Q>,
    ) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData + for<'de> serde::de::Deserialize<'de>,
        Q: ActionTag,
    {
        // Add a replay factory.
        self.assembly
            .add_replayer(action_key, move |runtime_parts| {
                let (_enclave_key, action_key) = runtime_parts.action_aliases[action_key.into()];
                Box::new(runtime::replay::TypedReplayer::<T>::new(action_key))
            })?;
        Ok(())
    }
}
