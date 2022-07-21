use std::marker::PhantomData;

use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, EnvBuilder, FindElements, PortType,
    ReactionBuilderState,
};
use crate::runtime;
use slotmap::SecondaryMap;

pub trait Reactor: Sized {
    fn build<S: runtime::ReactorState>(
        name: &str,
        state: S,
        parent: Option<runtime::ReactorKey>,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError>;
}

/// ReactorBuilder is the Builder-side definition of a Reactor, and is type-erased
pub(super) struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    pub name: String,
    /// The user's Reactor
    pub state: Box<dyn runtime::ReactorState>,
    /// The top-level/class name of the Reactor
    pub type_name: String,
    /// Optional parent reactor key
    pub parent_reactor_key: Option<runtime::ReactorKey>,
    /// Reactions in this ReactorType
    pub reactions: SecondaryMap<runtime::ReactionKey, ()>,
    /// Ports in this Reactor
    pub ports: SecondaryMap<runtime::PortKey, ()>,
    /// Actions in this Reactor
    pub actions: SecondaryMap<runtime::ActionKey, ()>,
}

impl ReactorBuilder {
    pub(super) fn type_name(&self) -> &str {
        self.type_name.as_ref()
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

impl From<ReactorBuilder> for Box<dyn runtime::ReactorState> {
    fn from(builder: ReactorBuilder) -> Self {
        builder.state
    }
}

/// Builder struct used to facilitate construction of a ReactorBuilder by user/generated code.
pub struct ReactorBuilderState<'a, S: runtime::ReactorState> {
    /// The ReactorKey of this Builder
    reactor_key: runtime::ReactorKey,
    env: &'a mut EnvBuilder,
    startup_action: BuilderActionKey,
    shutdown_action: BuilderActionKey,
    phantom: PhantomData<S>,
}

impl<'a, S: runtime::ReactorState> FindElements for ReactorBuilderState<'a, S> {
    fn get_port_by_name(&self, port_name: &str) -> Result<runtime::PortKey, BuilderError> {
        self.env.get_port(port_name, self.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<runtime::ActionKey, BuilderError> {
        self.env.get_action(action_name, self.reactor_key)
    }
}

impl<'a, S: runtime::ReactorState> ReactorBuilderState<'a, S> {
    pub(super) fn new(
        name: &str,
        parent: Option<runtime::ReactorKey>,
        state: S,
        env: &'a mut EnvBuilder,
    ) -> Self {
        let type_name = std::any::type_name::<S>();
        let reactor_key = env.reactor_builders.insert({
            ReactorBuilder {
                name: name.into(),
                state: Box::new(state),
                type_name: type_name.into(),
                parent_reactor_key: parent,
                reactions: SecondaryMap::new(),
                ports: SecondaryMap::new(),
                actions: SecondaryMap::new(),
            }
        });

        let startup_action = env
            .add_timer(
                "__startup",
                runtime::Duration::from_micros(0),
                runtime::Duration::from_micros(0),
                reactor_key,
            )
            .expect("Duplicate startup Action?");

        let shutdown_action = env
            .add_shutdown_action("__shutdown", reactor_key)
            .expect("Duplicate shutdown Action?");

        Self {
            reactor_key,
            env,
            startup_action,
            shutdown_action,
            phantom: PhantomData,
        }
    }

    /// Get the ReactorKey for this ReactorBuilder
    pub fn get_key(&self) -> runtime::ReactorKey {
        self.reactor_key
    }

    pub fn get_startup_action(&self) -> BuilderActionKey {
        self.startup_action
    }

    pub fn get_shutdown_action(&self) -> BuilderActionKey {
        self.shutdown_action
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.env.add_timer(name, period, offset, self.reactor_key)
    }

    pub fn add_logical_action<T: runtime::PortData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
    ) -> Result<BuilderActionKey<T>, BuilderError> {
        self.env
            .add_logical_action::<T>(name, min_delay, self.reactor_key)
    }

    pub fn add_reaction(
        &mut self,
        name: &str,
        reaction_fn: Box<dyn runtime::ReactionFn>,
    ) -> ReactionBuilderState {
        let priority = self.env.reactor_builders[self.reactor_key].reactions.len();
        ReactionBuilderState::new(name, priority, self.reactor_key, reaction_fn, self.env)
    }

    pub fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
    ) -> Result<BuilderPortKey<T>, BuilderError> {
        self.env.add_port::<T>(name, port_type, self.reactor_key)
    }

    pub fn add_child_reactor<R: Reactor, CS: runtime::ReactorState>(
        &mut self,
        name: &str,
        // state: R::State,
        state: CS,
    ) -> Result<R, BuilderError> {
        R::build(name, state, Some(self.reactor_key), self.env)
    }

    pub fn bind_port<T: runtime::PortData>(
        &mut self,
        port_a_key: BuilderPortKey<T>,
        port_b_key: BuilderPortKey<T>,
    ) -> Result<(), BuilderError> {
        self.env.bind_port(port_a_key, port_b_key)
    }

    pub fn finish(self) -> Result<runtime::ReactorKey, BuilderError> {
        Ok(self.reactor_key)
    }
}
