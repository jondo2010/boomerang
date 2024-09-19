use super::{
    ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey, BuilderReactionKey,
    EnvBuilder, FindElements, Input, Logical, Output, Physical, PhysicalActionKey, PortType2,
    ReactionBuilderState, TimerActionKey, TimerSpec, TriggerMode, TypedActionKey, TypedPortKey,
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

pub trait Reactor: Sized {
    type State: runtime::ReactorState;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError>;
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
    type Inner = R::State;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_child_reactor(name, inner)
    }
}

impl<R, const N: usize> ReactorField for [R; N]
where
    R: Reactor + std::fmt::Debug,
    R::State: Clone,
{
    type Inner = R::State;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_child_reactors(name, inner)
    }
}

impl<T: runtime::PortData> ReactorField for TypedPortKey<T, Input> {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_input_port(name)
    }
}

impl<T: runtime::PortData, const N: usize> ReactorField for [TypedPortKey<T, Input>; N] {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_input_ports(name)
    }
}

impl<T: runtime::PortData> ReactorField for TypedPortKey<T, Output> {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_output_port(name)
    }
}

impl<T: runtime::PortData, const N: usize> ReactorField for [TypedPortKey<T, Output>; N] {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_output_ports(name)
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

impl<T: runtime::ActionData> ReactorField for TypedActionKey<T, Logical> {
    type Inner = Option<runtime::Duration>;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_logical_action(name, inner)
    }
}

impl<T: runtime::ActionData> ReactorField for TypedActionKey<T, Physical> {
    type Inner = Option<runtime::Duration>;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_physical_action(name, inner)
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

/// ReactorBuilder is the Builder-side definition of a Reactor, and is type-erased
pub(super) struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    name: String,
    /// The user's Reactor
    state: Box<dyn runtime::ReactorState>,
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
}

impl ReactorBuilder {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this `ReactorBuilder` into a `runtime::Reactor`
    pub fn build_runtime(self) -> runtime::Reactor {
        runtime::Reactor::new(&self.name, self.state)
    }
}

impl std::fmt::Debug for ReactorBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactorBuilder")
            .field("name", &self.name)
            .field("(state) type_name", &self.type_name)
            .field("parent_reactor_key", &self.parent_reactor_key)
            .field("reactions", &self.reactions)
            .field("ports", &self.ports)
            .field("actions", &self.actions)
            .finish()
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
        self.env.get_port(port_name, self.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError> {
        self.env.find_action_by_name(action_name, self.reactor_key)
    }
}

impl<'a> ReactorBuilderState<'a> {
    pub(super) fn new<S: runtime::ReactorState>(
        name: &str,
        parent: Option<BuilderReactorKey>,
        reactor_state: S,
        env: &'a mut EnvBuilder,
    ) -> Self {
        let type_name = std::any::type_name::<S>();
        let reactor_key = env.reactor_builders.insert({
            ReactorBuilder {
                name: name.into(),
                state: Box::new(reactor_state),
                type_name: type_name.into(),
                parent_reactor_key: parent,
                reactions: SecondaryMap::new(),
                ports: SecondaryMap::new(),
                actions: SecondaryMap::new(),
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
                action.get_reactor_key() == reactor_key && *action.get_type() == ActionType::Startup
            })
            .map(|(action_key, _)| action_key)
            .expect("Startup action not found");

        let shutdown_action = env
            .action_builders
            .iter()
            .find(|(_, action)| {
                action.get_reactor_key() == reactor_key
                    && *action.get_type() == ActionType::Shutdown
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

    /// Get the [`ReactorKey`] for this [`ReactorBuilder`]
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
        let action_key = self.add_logical_action::<()>(name, spec.period)?;

        let startup_fn: runtime::ReactionFn = Box::new(
            move |ctx: &mut runtime::Context, _, _, _, actions: &mut [&mut runtime::Action]| {
                let [timer]: &mut [&mut runtime::Action; 1usize] = actions.try_into().unwrap();
                let mut timer: runtime::ActionRef = (*timer).into();
                ctx.schedule_action(&mut timer, None, spec.offset);
            },
        );

        let startup_key = self.startup_action;
        self.add_reaction(&format!("_{name}_startup"), startup_fn)
            .with_action(startup_key, 0, TriggerMode::TriggersOnly)?
            .with_action(action_key, 1, TriggerMode::EffectsOnly)?
            .finish()?;

        if spec.period.is_some() {
            let reset_fn: runtime::ReactionFn = Box::new(
                |ctx: &mut runtime::Context, _, _, _, actions: &mut [&mut runtime::Action]| {
                    let [timer]: &mut [&mut runtime::Action; 1usize] = actions.try_into().unwrap();
                    let mut timer: runtime::ActionRef = (*timer).into();
                    ctx.schedule_action(&mut timer, None, None);
                },
            );

            self.add_reaction(&format!("_{name}_reset"), reset_fn)
                .with_action(action_key, 0, TriggerMode::TriggersAndEffects)?
                .finish()?;
        }

        Ok(TimerActionKey::from(BuilderActionKey::from(action_key)))
    }

    /// Add a new logical action to the reactor.
    ///
    /// This method forwards to the implementation at
    /// [`crate::builder::env::EnvBuilder::add_logical_action`].
    pub fn add_logical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.env
            .add_logical_action::<T>(name, min_delay, self.reactor_key)
    }

    pub fn add_physical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.env
            .add_physical_action::<T>(name, min_delay, self.reactor_key)
    }

    /// Add a new reaction to this reactor.
    pub fn add_reaction(
        &mut self,
        name: &str,
        reaction_fn: runtime::ReactionFn,
    ) -> ReactionBuilderState {
        self.env.add_reaction(name, self.reactor_key, reaction_fn)
    }

    /// Add a new input port to this reactor.
    pub fn add_input_port<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Input>, BuilderError> {
        self.env.add_input_port::<T>(name, self.reactor_key)
    }

    /// Adds a bus of input ports to this reactor.
    pub fn add_input_ports<T: runtime::PortData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Input>; N], BuilderError> {
        let mut ports = Vec::with_capacity(N);
        for i in 0..N {
            let port = self.add_input_port::<T>(&format!("{name}{i}"))?;
            ports.push(port);
        }
        Ok(ports.try_into().expect("Error converting Vec to array"))
    }

    /// Add a new output port to this reactor.
    pub fn add_output_port<T: runtime::PortData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Output>, BuilderError> {
        self.env.add_output_port::<T>(name, self.reactor_key)
    }

    /// Adds a bus of output port(s) to this reactor.
    pub fn add_output_ports<T: runtime::PortData, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Output>; N], BuilderError> {
        let mut ports = Vec::with_capacity(N);
        for i in 0..N {
            let port = self.add_output_port::<T>(&format!("{name}{i}"))?;
            ports.push(port);
        }
        Ok(ports.try_into().expect("Error converting Vec to array"))
    }

    /// Add a new child reactor to this reactor.
    pub fn add_child_reactor<R: Reactor>(
        &mut self,
        name: &str,
        state: R::State,
    ) -> Result<R, BuilderError> {
        R::build(name, state, Some(self.reactor_key), self.env)
    }

    /// Add multiple child reactors to this reactor.
    pub fn add_child_reactors<R, const N: usize>(
        &mut self,
        name: &str,
        state: R::State,
    ) -> Result<[R; N], BuilderError>
    where
        R: Reactor + std::fmt::Debug,
        R::State: Clone,
    {
        let mut reactors = Vec::with_capacity(N);
        for i in 0..N {
            let reactor = self.add_child_reactor::<R>(&format!("{name}{i}"), state.clone())?;
            reactors.push(reactor);
        }
        Ok(reactors.try_into().expect("Error converting Vec to array"))
    }

    /// Add a new child reactor using a closure to build it.
    pub fn add_child_with<F>(&mut self, f: F) -> Result<BuilderReactorKey, BuilderError>
    where
        F: FnOnce(BuilderReactorKey, &mut EnvBuilder) -> Result<BuilderReactorKey, BuilderError>,
    {
        f(self.reactor_key, self.env)
    }

    /// Bind 2 ports on this reactor. This has the logical meaning of "connecting" `port_a` to
    /// `port_b`.
    pub fn bind_port<T: runtime::PortData, Q1: PortType2, Q2: PortType2>(
        &mut self,
        port_a_key: TypedPortKey<T, Q1>,
        port_b_key: TypedPortKey<T, Q2>,
    ) -> Result<(), BuilderError> {
        self.env.bind_port(port_a_key, port_b_key)
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
}
