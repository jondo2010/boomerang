use std::time::Duration;

use super::{
    ActionBuilder, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey, EnvBuilder,
    FindElements, Logical, Physical, PortType, ReactionBuilderState, TypedActionKey, TypedPortKey,
};
use crate::runtime;
use slotmap::{SecondaryMap, SlotMap};

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
    pub actions: SlotMap<BuilderActionKey, ActionBuilder>,
}

impl ReactorBuilder {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub(super) fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this `ReactorBuilder` into a `runtime::Reactor`
    pub fn build_runtime(
        self,
        actions: tinymap::TinyMap<runtime::keys::ActionKey, runtime::Action>,
        action_triggers: tinymap::TinySecondaryMap<
            runtime::keys::ActionKey,
            Vec<runtime::LevelReactionKey>,
        >,
    ) -> runtime::Reactor {
        runtime::Reactor::new(&self.name, self.state, actions, action_triggers)
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
        self.env.find_port_by_name(port_name, self.reactor_key)
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
                actions: SlotMap::with_key(),
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
        period: Option<Duration>,
        offset: Option<Duration>,
    ) -> Result<TypedActionKey, BuilderError> {
        let action_key = self.add_logical_action(name, period)?;

        let startup_fn: Box<dyn runtime::ReactionFn> = Box::new(
            move |ctx: &mut runtime::Context, _, _, _, actions: &mut [&mut runtime::Action]| {
                let [_startup, timer]: &mut [&mut runtime::Action; 2usize] =
                    actions.try_into().unwrap();
                let mut timer: runtime::ActionRef = (*timer).into();
                ctx.schedule_action(&mut timer, None, offset);
            },
        );

        let startup_key = self.startup_action;
        self.add_reaction(&format!("_{name}_startup"), startup_fn)
            .with_trigger_action(startup_key, 0)
            .with_schedulable_action(action_key, 1)
            .finish()?;

        if period.is_some() {
            let reset_fn: Box<dyn runtime::ReactionFn> = Box::new(
                |ctx: &mut runtime::Context, _, _, _, actions: &mut [&mut runtime::Action]| {
                    let [timer]: &mut [&mut runtime::Action; 1usize] = actions.try_into().unwrap();
                    let mut timer: runtime::ActionRef = (*timer).into();
                    ctx.schedule_action(&mut timer, None, None);
                },
            );

            self.add_reaction(&format!("_{name}_reset"), reset_fn)
                .with_trigger_action(action_key, 0)
                .with_schedulable_action(action_key, 0)
                .finish()?;
        }

        Ok(action_key)
    }

    /// Add a new logical action to the reactor.
    ///
    /// This method forwards to the implementation at
    /// [`crate::builder::env::EnvBuilder::add_logical_action`].
    pub fn add_logical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.env
            .add_logical_action::<T>(name, min_delay, self.reactor_key)
    }

    pub fn add_physical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<Duration>,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.env
            .add_physical_action::<T>(name, min_delay, self.reactor_key)
    }

    /// Add a new reaction to this reactor.
    pub fn add_reaction(
        &mut self,
        name: &str,
        reaction_fn: Box<dyn runtime::ReactionFn>,
    ) -> ReactionBuilderState {
        self.env.add_reaction(name, self.reactor_key, reaction_fn)
    }

    /// Add a new port to this reactor.
    pub fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
    ) -> Result<TypedPortKey<T>, BuilderError> {
        self.env.add_port::<T>(name, port_type, self.reactor_key)
    }

    /// Add a new child reactor to this reactor.
    pub fn add_child_reactor<R: Reactor>(
        &mut self,
        name: &str,
        state: R::State,
    ) -> Result<R, BuilderError> {
        R::build(name, state, Some(self.reactor_key), self.env)
    }

    /// Bind 2 ports on this reactor. This has the logical meaning of "connecting" `port_a` to
    /// `port_b`.
    pub fn bind_port<T: runtime::PortData>(
        &mut self,
        port_a_key: TypedPortKey<T>,
        port_b_key: TypedPortKey<T>,
    ) -> Result<(), BuilderError> {
        self.env.bind_port(port_a_key, port_b_key)
    }

    pub fn finish(self) -> Result<BuilderReactorKey, BuilderError> {
        Ok(self.reactor_key)
    }
}
