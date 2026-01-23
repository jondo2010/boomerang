use std::fmt::Debug;

use super::{
    ActionTag, ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey,
    BuilderReactionKey, EnvBuilder, Input, Logical, Output, Physical, PortTag, TimerActionKey,
    TimerSpec, TypedActionKey, TypedPortKey, PortBank,
};
use crate::runtime;
use slotmap::SecondaryMap;

slotmap::new_key_type! {
    pub struct BuilderReactorKey;
}

impl petgraph::graph::GraphIndex for BuilderReactorKey {
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

/// `ParentReactorBuilder` is implemented for Reactor elements that can have a parent Reactor
pub trait ParentReactorBuilder {
    fn parent_reactor_key(&self) -> Option<BuilderReactorKey>;
}

/// ReactorBuilder is the Builder-side definition of a Reactor, and is type-erased
#[derive(Debug)]
pub struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    name: String,
    /// The user's Reactor
    state: Box<dyn BaseReactorState>,
    /// The top-level/class name of the Reactor
    type_name: String,
    /// Optional parent reactor key
    pub parent_reactor_key: Option<BuilderReactorKey>,
    /// Reactions in this ReactorType
    pub reactions: SecondaryMap<BuilderReactionKey, ()>,
    /// Ports in this Reactor
    pub ports: SecondaryMap<BuilderPortKey, ()>,
    /// Actions in this Reactor
    pub actions: SecondaryMap<BuilderActionKey, ()>,
    /// The bank info of the bank that this Reactor belongs to, if any.
    pub bank_info: Option<runtime::BankInfo>,
    /// Whether this Reactor is an enclave
    pub is_enclave: bool,
}

impl ParentReactorBuilder for ReactorBuilder {
    fn parent_reactor_key(&self) -> Option<BuilderReactorKey> {
        self.parent_reactor_key
    }
}

impl ReactorBuilder {
    /// Create a new `ReactorBuilder` with the given parameters.
    pub fn new<S: runtime::ReactorData>(
        name: &str,
        type_name: &'static str,
        reactor_state: S,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        is_enclave: bool,
    ) -> Self {
        Self {
            name: name.into(),
            state: Box::new(ReactorState(reactor_state)),
            type_name: type_name.into(),
            parent_reactor_key: parent,
            reactions: SecondaryMap::new(),
            ports: SecondaryMap::new(),
            actions: SecondaryMap::new(),
            bank_info,
            is_enclave,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bank_info(&self) -> Option<&runtime::BankInfo> {
        self.bank_info.as_ref()
    }

    #[allow(dead_code)] // TODO: use or remove this
    pub fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this [`ReactorBuilder`] into a [`Box<dyn runtime::BaseReactor>`]
    pub fn into_runtime(self, name: &str) -> Box<dyn runtime::BaseReactor> {
        self.state.into_runtime(name)
    }
}

/// Builder struct used to facilitate construction of a ReactorBuilder by user/generated code.
#[derive(Debug)]
pub struct ReactorBuilderState<'a, S: runtime::ReactorData = ()> {
    /// The ReactorKey of this Builder
    reactor_key: BuilderReactorKey,
    env: &'a mut EnvBuilder,
    startup_action: TypedActionKey,
    shutdown_action: TypedActionKey,
    phantom: std::marker::PhantomData<S>,
}

impl<'a, S: runtime::ReactorData> ReactorBuilderState<'a, S> {
    pub(super) fn new(
        name: &str,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        reactor_state: S,
        is_enclave: bool,
        env: &'a mut EnvBuilder,
    ) -> Self {
        let type_name = std::any::type_name::<S>();
        let reactor_key = env.reactor_builders.insert(ReactorBuilder::new(
            name,
            type_name,
            reactor_state,
            parent,
            bank_info,
            is_enclave,
        ));

        let startup_action = env
            .add_startup_action("__startup", reactor_key)
            .expect("Duplicate startup Action?");

        let shutdown_action = env
            .add_shutdown_action("__shutdown", reactor_key)
            .expect("Duplicate shutdown Action?");

        Self {
            reactor_key,
            env,
            startup_action,
            shutdown_action,
            phantom: std::marker::PhantomData,
        }
    }

    /// Create a new `ReactorBuilderState` for a pre-existing reactor
    pub(super) fn from_pre_existing(
        reactor_key: BuilderReactorKey,
        env: &'a mut EnvBuilder,
    ) -> Self {
        // Find the startup and shutdown actions for this reactor
        let startup_action = env
            .action_builders
            .iter()
            .find(|(_, action)| matches!(action.r#type(), ActionType::Timer(TimerSpec { period, offset }) if period.is_none() && offset.is_none()) && action.reactor_key() == reactor_key)
            .map(|(action_key, _)| action_key)
            .expect("Startup action not found");

        let shutdown_action = env
            .action_builders
            .iter()
            .find(|(_, action)| {
                matches!(action.r#type(), ActionType::Shutdown)
                    && action.reactor_key() == reactor_key
            })
            .map(|(action_key, _)| action_key)
            .expect("Shutdown action not found");

        Self {
            reactor_key,
            env,
            startup_action: TypedActionKey::from(startup_action),
            shutdown_action: TypedActionKey::from(shutdown_action),
            phantom: std::marker::PhantomData,
        }
    }

    /// Get the [`EnvBuilder`] for this `ReactorBuilder`
    pub fn env(&mut self) -> &mut EnvBuilder {
        self.env
    }

    /// Get the [`BuilderReactorKey`] for this `ReactorBuilder`
    pub fn key(&self) -> BuilderReactorKey {
        self.reactor_key
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
    ) -> Result<TimerActionKey, BuilderError> {
        self.env.add_timer_action(name, self.reactor_key, spec)
    }

    /// Add a new action to the reactor.
    ///
    /// This method forwards to the implementation at
    /// [`crate::env::EnvBuilder::internal_add_action`].
    pub fn add_action<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Q>, BuilderError> {
        self.env
            .add_action::<T, Q>(name, min_delay, self.reactor_key)
    }

    /// Add a new logical action to the reactor.
    ///
    /// This method forwards to the implementation at
    /// [`crate::env::EnvBuilder::add_logical_action`].
    pub fn add_logical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.env
            .add_action::<T, Logical>(name, min_delay, self.reactor_key)
    }

    pub fn add_physical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.env
            .add_action::<T, Physical>(name, min_delay, self.reactor_key)
    }

    /// Add a new input port to this reactor.
    pub fn add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        bank_info: Option<runtime::BankInfo>,
    ) -> Result<TypedPortKey<T, Q>, BuilderError> {
        tracing::debug!("Adding port: {name}");
        self.env
            .internal_add_port::<T, Q>(name, self.reactor_key, bank_info)
            .map(Into::into)
    }

    /// Adds a bus of input ports to this reactor.
    pub fn add_ports<T: runtime::ReactorData, Q: PortTag, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Q>; N], BuilderError> {
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
    ) -> Result<PortBank<T, Q>, BuilderError> {
        let mut ports = Vec::with_capacity(len);
        for i in 0..len {
            let port = self.add_port::<T, Q>(
                name,
                Some(runtime::BankInfo { idx: i, total: len }),
            )?;
            ports.push(port);
        }
        Ok(PortBank::new(ports))
    }

    /// Add a new input port to this reactor.
    pub fn add_input_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Input>, BuilderError> {
        self.add_port::<T, Input>(name, None)
    }

    /// Add a new output port to this reactor.
    pub fn add_output_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Output>, BuilderError> {
        self.add_port::<T, Output>(name, None)
    }

    /// Adds a bus of input port(s) to this reactor.
    pub fn add_input_ports<T: runtime::ReactorData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Input>; N], BuilderError> {
        self.add_ports(name)
    }

    /// Adds a runtime-sized bank of input ports to this reactor.
    pub fn add_input_bank<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        len: usize,
    ) -> Result<PortBank<T, Input>, BuilderError> {
        self.add_ports_bank(name, len)
    }

    /// Adds a bus of output port(s) to this reactor.
    pub fn add_output_ports<T: runtime::ReactorData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Output>; N], BuilderError> {
        self.add_ports(name)
    }

    /// Adds a runtime-sized bank of output ports to this reactor.
    pub fn add_output_bank<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        len: usize,
    ) -> Result<PortBank<T, Output>, BuilderError> {
        self.add_ports_bank(name, len)
    }

    /// Add a new child reactor using a closure to build it.
    pub fn add_child_with<F>(&mut self, f: F) -> Result<BuilderReactorKey, BuilderError>
    where
        F: FnOnce(BuilderReactorKey, &mut EnvBuilder) -> Result<BuilderReactorKey, BuilderError>,
    {
        f(self.reactor_key, self.env)
    }

    /// Connect 2 ports on this reactor. This has the logical meaning of "connecting" `port_a` to
    /// `port_b`.
    pub fn connect_port<T, Q1, Q2, A1, A2>(
        &mut self,
        port_a_key: TypedPortKey<T, Q1, A1>,
        port_b_key: TypedPortKey<T, Q2, A2>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        self.env
            .add_port_connection::<T, _, _>(port_a_key, port_b_key, after, physical)
    }

    /// Connect multiple ports on this reactor. This has the logical meaning of "connecting"
    /// `ports_from` to `ports_to`.
    pub fn connect_ports<T, Q1, Q2, A1, A2>(
        &mut self,
        ports_from: impl Iterator<Item = TypedPortKey<T, Q1, A1>>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2, A2>>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        Q1: PortTag,
        Q2: PortTag,
    {
        let ports_from: Vec<_> = ports_from.collect();
        let ports_to: Vec<_> = ports_to.collect();

        if ports_from.len() != ports_to.len() {
            return Err(BuilderError::PortConnectionLengthMismatch {
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
    ) -> Result<(), BuilderError>
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
    ) -> Result<(), BuilderError>
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

    pub fn finish(self) -> Result<BuilderReactorKey, BuilderError> {
        self.env.validate_reactions()?;
        Ok(self.reactor_key)
    }

    /// Find a PhysicalAction globally in the EnvBuilder given its fully-qualified name
    pub fn find_physical_action_by_fqn(
        &self,
        action_fqn: impl Into<BuilderFqn>,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.env.find_physical_action_by_fqn(action_fqn)
    }

    /// Add a recorder for the given action key.
    #[cfg(feature = "replay")]
    pub fn add_action_recorder<T, Q>(
        &mut self,
        action_key: TypedActionKey<T, Q>,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + serde::Serialize,
        Q: ActionTag,
    {
        let topic = self.env.fqn_for(action_key, false)?.to_string();
        tracing::debug!("Adding recorder for action {topic}",);
        let _ = self
            .add_reaction(Some("recorder"))
            .with_trigger(action_key)
            .with_defered_reaction_fn(move |runtime_parts| {
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
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + for<'de> serde::de::Deserialize<'de>,
        Q: ActionTag,
    {
        // Add a replayer builder
        self.env.add_replayer(action_key, move |runtime_parts| {
            let (_enclave_key, action_key) =
                runtime_parts.aliases.action_aliases[action_key.into()];
            Box::new(runtime::replay::TypedReplayer::<T>::new(action_key))
        })?;
        Ok(())
    }
}
