use std::{fmt::Debug, time::Duration};

use super::{
    ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey, BuilderReactionKey,
    EnvBuilder, FindElements, Logical, Output, Physical, PhysicalActionKey, PortTag,
    ReactionBuilderState, TimerActionKey, TimerSpec, TriggerMode, TypedActionKey, TypedPortKey,
};
use crate::{runtime, ActionTag};
use boomerang_runtime::BoxedReactionFn;
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
    type Inner = R::State;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_child_reactor(name, inner)
    }
}

/// NOTE: `R::State: Clone` is required because state is cloned for each child reactor.
impl<R, const N: usize> ReactorField for [R; N]
where
    R: Reactor,
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

impl<T: runtime::ReactorData, Q: PortTag> ReactorField for TypedPortKey<T, Q> {
    type Inner = ();

    fn build(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_port::<T, Q>(name)
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
    type Inner = Option<Duration>;

    fn build(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState,
    ) -> Result<Self, BuilderError> {
        parent.add_action(name, inner)
    }
}

impl ReactorField for PhysicalActionKey {
    type Inner = Option<Duration>;

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
pub(super) struct ReactorBuilder {
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

    #[allow(dead_code)] // TODO: use or remove this
    pub fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this [`ReactorBuilder`] into a [`Box<dyn runtime::BaseReactor>`]
    pub fn into_runtime(self) -> Box<dyn runtime::BaseReactor> {
        self.state.into_runtime(&self.name)
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
                matches!(action.r#type(), ActionType::Startup)
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
        let action_key = self.add_logical_action::<()>(name, None)?;

        let startup_key = self.startup_action;

        let trigger_mode = if spec.period.is_some() {
            // If the timer has a period, it should be triggered by the action_key
            TriggerMode::TriggersAndEffects
        } else {
            // Otherwise, it should only be triggered by the startup action
            TriggerMode::EffectsOnly
        };

        self.add_reaction(
            &format!("_{name}_startup"),
            runtime::reaction::TimerFn(spec.period),
        )
        .with_action(startup_key, 0, TriggerMode::TriggersOnly)?
        .with_action(action_key, 1, trigger_mode)?
        .finish()?;

        Ok(TimerActionKey::from(BuilderActionKey::from(action_key)))
    }

    /// Add a new action to the reactor.
    ///
    /// This method forwards to the implementation at [`crate::env::EnvBuilder::internal_add_action`].
    pub fn add_action<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
    ) -> Result<TypedActionKey<T, Q>, BuilderError> {
        self.env
            .internal_add_action::<T, Q>(name, min_delay, self.reactor_key)
    }

    /// Add a new logical action to the reactor.
    ///
    /// This method forwards to the implementation at
    /// [`crate::env::EnvBuilder::add_logical_action`].
    pub fn add_logical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.env
            .internal_add_action::<T, Logical>(name, min_delay, self.reactor_key)
    }

    pub fn add_physical_action<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.env
            .internal_add_action::<T, Physical>(name, min_delay, self.reactor_key)
    }

    /// Add a new reaction to this reactor.
    pub fn add_reaction(
        &mut self,
        name: &str,
        reaction_fn: impl Into<BoxedReactionFn>,
    ) -> ReactionBuilderState {
        self.env
            .add_reaction(name, self.reactor_key, reaction_fn.into())
    }

    /// Add a new input port to this reactor.
    pub fn add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Q>, BuilderError> {
        self.env
            .internal_add_port::<T, Q>(name, self.reactor_key)
            .map(Into::into)
    }

    /// Adds a bus of input ports to this reactor.
    pub fn add_ports<T: runtime::ReactorData, Q: PortTag, const N: usize>(
        &mut self,
        name: &str,
    ) -> Result<[TypedPortKey<T, Q>; N], BuilderError> {
        let mut ports = Vec::with_capacity(N);
        for i in 0..N {
            let port = self.add_port::<T, Q>(&format!("{name}{i}"))?;
            ports.push(port);
        }
        Ok(ports.try_into().expect("Error converting Vec to array"))
    }

    /// Add a new output port to this reactor.
    pub fn add_output_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
    ) -> Result<TypedPortKey<T, Output>, BuilderError> {
        self.env.add_output_port::<T>(name, self.reactor_key)
    }

    /// Adds a bus of output port(s) to this reactor.
    pub fn add_output_ports<T: runtime::ReactorData, const N: usize>(
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
        R::build(name, state, Some(self.reactor_key), None, self.env)
    }

    /// Add multiple child reactors to this reactor.
    pub fn add_child_reactors<R, const N: usize>(
        &mut self,
        name: &str,
        state: R::State,
    ) -> Result<[R; N], BuilderError>
    where
        R: Reactor,
        R::State: Clone,
    {
        let reactors = (0..N)
            .map(|i| {
                R::build(
                    &format!("{name}{i}"),
                    state.clone(),
                    Some(self.reactor_key),
                    Some(runtime::BankInfo { idx: i, total: N }),
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
        after: Option<Duration>,
        physical: bool,
    ) -> Result<(), BuilderError> {
        self.env
            .connect_ports::<T, _, _>(port_a_key, port_b_key, after, physical)
    }

    /// Connect multiple ports on this reactor. This has the logical meaning of "connecting" `ports_from` to `ports_to`.
    pub fn connect_ports<T: runtime::ReactorData + Clone, Q1: PortTag, Q2: PortTag>(
        &mut self,
        ports_from: impl Iterator<Item = TypedPortKey<T, Q1>>,
        ports_to: impl Iterator<Item = TypedPortKey<T, Q2>>,
        after: Option<Duration>,
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
}
