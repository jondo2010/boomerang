use std::fmt::Debug;

use super::{
    ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey, BuilderReactionKey,
    EnvBuilder, FindElements, Logical, Output, Physical, PhysicalActionKey, PortTag,
    ReactionBuilderState, TimerActionKey, TimerSpec, TypedActionKey, TypedPortKey,
};
use crate::{runtime, ActionTag, BuilderRuntimeParts, Input, TriggerMode};
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

pub trait Reactor: Sized {
    type State: runtime::ReactorData;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        is_enclave: bool,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError>;

    fn iter(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self)
    }
}

/// This builder trait is implemented for fields in the Reactor struct.
pub trait ReactorField: Sized {
    type Inner;

    /// Build a `ReactionBuilderState` for this Reaction
    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError>;
}

impl<R: Reactor> ReactorField for R {
    type Inner = (R::State, bool);

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        let (state, is_enclave) = inner;
        parent.add_child_reactor(name, state, is_enclave)
    }
}

/// NOTE: `R::State: Clone` is required because state is cloned for each child reactor.
impl<R, const N: usize> ReactorField for [R; N]
where
    R: Reactor,
    R::State: Clone,
{
    type Inner = (R::State, bool);

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        let (state, is_enclave) = inner;
        parent.add_child_reactors(name, state, is_enclave)
    }
}

impl<T: runtime::ReactorData, Q: PortTag> ReactorField for TypedPortKey<T, Q> {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_port::<T, Q>(name, None)
    }
}

impl<T: runtime::ReactorData, Q: PortTag, const N: usize> ReactorField for [TypedPortKey<T, Q>; N] {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_ports::<T, Q, N>(name)
    }
}

impl ReactorField for TimerActionKey {
    type Inner = TimerSpec;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_timer(name, inner)
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> ReactorField for TypedActionKey<T, Q> {
    type Inner = Option<runtime::Duration>;

    fn build(
        name: &str,
        min_delay: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_action(name, min_delay)
    }
}

impl ReactorField for PhysicalActionKey {
    type Inner = Option<runtime::Duration>;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent
            .add_physical_action::<()>(name, inner)
            .map(Into::into)
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
pub struct ReactorBuilderState<'a> {
    /// The ReactorKey of this Builder
    reactor_key: BuilderReactorKey,
    env: &'a mut EnvBuilder,
    startup_action: TypedActionKey,
    shutdown_action: TypedActionKey,
}

impl<'a> FindElements for ReactorBuilderState<'a> {
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError> {
        self.env.find_port_by_name(port_name, self.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError> {
        self.env.find_action_by_name(action_name, self.reactor_key)
    }
}

impl<'a> ReactorBuilderState<'a> {
    pub(super) fn new<S: runtime::ReactorData>(
        name: &str,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        reactor_state: S,
        is_enclave: bool,
        env: &'a mut EnvBuilder,
    ) -> Self {
        let type_name = std::any::type_name::<S>();
        let reactor_key = env.reactor_builders.insert({
            ReactorBuilder {
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
        });

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
            .find(|(_, action)| {
                matches!(action.r#type(), ActionType::Timer(TimerSpec { period, offset }) if period.is_none() && offset.is_none())
                    && action.reactor_key() == reactor_key
            })
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
        }
    }

    /// Get the [`BuilderReactorKey`] for this `ReactorBuilder`
    pub fn get_key(&self) -> BuilderReactorKey {
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

    /// Add a new reaction to this reactor.
    pub fn add_reaction<F>(&mut self, name: &str, reaction_builder_fn: F) -> ReactionBuilderState
    where
        F: for<'any> FnOnce(&'any BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        self.env
            .add_reaction(name, self.reactor_key, reaction_builder_fn)
    }

    /// Add a new input port to this reactor.
    pub fn add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        bank_info: Option<runtime::BankInfo>,
    ) -> Result<TypedPortKey<T, Q>, BuilderError> {
        self.env
            .internal_add_port::<T, Q>(name, self.reactor_key, bank_info)
            .map(Into::into)
    }

    /// Adds a bus of input ports to this reactor.
    pub fn add_ports<T: runtime::ReactorData, Q: PortTag, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Q>; N], BuilderError> {
        let mut ports = Vec::with_capacity(N);
        for i in 0..N {
            let port = self.add_port::<T, Q>(name, Some(runtime::BankInfo { idx: i, total: N }))?;
            ports.push(port);
        }
        Ok(ports.try_into().expect("Error converting Vec to array"))
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

    /// Adds a bus of output port(s) to this reactor.
    pub fn add_output_ports<T: runtime::ReactorData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Output>; N], BuilderError> {
        self.add_ports(name)
    }

    /// Add a new child reactor to this reactor.
    pub fn add_child_reactor<R: Reactor>(
        &mut self,
        name: &str,
        state: R::State,
        is_enclave: bool,
    ) -> Result<R, BuilderError> {
        R::build(
            name,
            state,
            Some(self.reactor_key),
            None,
            is_enclave,
            self.env,
        )
    }

    /// Add multiple child reactors to this reactor.
    pub fn add_child_reactors<R, const N: usize>(
        &mut self,
        name: &str,
        state: R::State,
        is_enclave: bool,
    ) -> Result<[R; N], BuilderError>
    where
        R: Reactor,
        R::State: Clone,
    {
        let reactors = (0..N)
            .map(|i| {
                R::build(
                    name,
                    state.clone(),
                    Some(self.reactor_key),
                    Some(runtime::BankInfo { idx: i, total: N }),
                    is_enclave,
                    self.env,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        reactors
            .try_into()
            .map_err(|_| BuilderError::InternalError("Error converting Vec to array".to_owned()))
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
    pub fn connect_port<T: runtime::ReactorData + Clone, Q1: PortTag, Q2: PortTag>(
        &mut self,
        port_a_key: TypedPortKey<T, Q1>,
        port_b_key: TypedPortKey<T, Q2>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError> {
        self.env
            .add_port_connection::<T, _, _>(port_a_key, port_b_key, after, physical)
    }

    /// Connect multiple ports on this reactor. This has the logical meaning of "connecting"
    /// `ports_from` to `ports_to`.
    pub fn connect_ports<T: runtime::ReactorData + Clone, Q1: PortTag, Q2: PortTag>(
        &mut self,
        ports_from: impl Iterator<Item = TypedPortKey<T, Q1>>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2>>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError> {
        for (port_from, port_to) in ports_from.zip(ports_to) {
            self.connect_port::<T, _, _>(port_from, port_to, after, physical)?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<BuilderReactorKey, BuilderError> {
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
        // Add a recorder builder
        let action_key = action_key.into();
        let topic = self.env.action_fqn(action_key, false)?.to_string();
        let _ = self
            .add_reaction("recorder", move |runtime_parts| {
                let (enclave_key, action_key) = runtime_parts.aliases.action_aliases[action_key];
                Box::new(
                    runtime::replay::RecorderFn::<T>::new(&topic, enclave_key, action_key).unwrap(),
                )
            })
            .with_action(action_key, 0, TriggerMode::TriggersAndUses)?
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
