use std::{sync::Arc, time::Duration};

use boomerang_runtime as runtime;

use super::{
    ActionBuilder, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey, EnvBuilder,
    FindElements, Logical, Physical, PortType, ReactionBuilderState, TypedActionKey, TypedPortKey,
};
use crate::util::DebugMap;
use itertools::Itertools;
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

    /// Build a reactor into the given [`EnvBuilder`]
    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        env: &mut EnvBuilder,
    ) -> Result<(BuilderReactorKey, Self), BuilderError>;
}

/// ReactorBuilder is the Builder-side definition of a Reactor, and is type-erased
#[derive(Clone)]
pub(super) struct ReactorBuilder {
    /// The instantiated/child name of the Reactor
    pub(super) name: String,
    /// The user's Reactor
    pub(super) state: Box<dyn runtime::ReactorState>,
    /// The top-level/class name of the Reactor
    pub(super) type_name: String,
    /// Optional parent reactor key
    pub(super) parent_reactor_key: Option<BuilderReactorKey>,
    /// Reactions in this ReactorType
    pub(super) reactions: SecondaryMap<BuilderReactionKey, ()>,
    /// Ports in this Reactor
    pub(super) ports: SecondaryMap<BuilderPortKey, ()>,
    /// Actions in this Reactor
    pub(super) actions: SlotMap<BuilderActionKey, ActionBuilder>,
}

impl std::fmt::Debug for ReactorBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactorBuilder")
            .field("name", &self.name)
            .field("(state) type_name", &self.type_name)
            .field("parent_reactor_key", &self.parent_reactor_key)
            .field("reactions", &self.reactions.keys().collect_vec())
            .field("ports", &self.ports.keys().collect_vec())
            .field("actions", &DebugMap(&self.actions))
            .finish()
    }
}

impl ReactorBuilder {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub(super) fn type_name(&self) -> &str {
        self.type_name.as_ref()
    }

    /// Build this `ReactorBuilder` into a [`runtime::Reactor`]
    pub fn build_runtime(
        &self,
        actions: tinymap::TinyMap<runtime::keys::ActionKey, runtime::Action>,
        action_triggers: tinymap::TinySecondaryMap<
            runtime::keys::ActionKey,
            Vec<runtime::LevelReactionKey>,
        >,
    ) -> runtime::Reactor {
        runtime::Reactor::new(&self.name, self.state.clone(), actions, action_triggers)
    }
}

/// Builder struct used to facilitate construction of a ReactorBuilder by user/generated code.
pub struct ReactorBuilderState<'a> {
    /// The ReactorKey of this Builder
    pub(crate) reactor_key: BuilderReactorKey,
    pub(crate) env: &'a mut EnvBuilder,
    /// The startup action for this reactor
    startup_action: TypedActionKey,
    /// The shutdown action for this reactor
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
        let builder = ReactorBuilder {
            name: name.into(),
            state: Box::new(reactor_state),
            type_name: type_name.into(),
            parent_reactor_key: parent,
            reactions: SecondaryMap::new(),
            ports: SecondaryMap::new(),
            actions: SlotMap::with_key(),
        };

        Self::from_reactor(builder, env)
    }

    pub(super) fn from_reactor(builder: ReactorBuilder, env: &'a mut EnvBuilder) -> Self {
        let reactor_key = env.reactor_builders.insert(builder);

        let startup_action = env
            .find_action_by_name("_start", reactor_key)
            .map(|action| action.into())
            .unwrap_or_else(|_| {
                env.add_startup_action("_start", reactor_key)
                    .expect("Duplicate startup Action?")
            });

        let shutdown_action = env
            .find_action_by_name("_stop", reactor_key)
            .map(|action| action.into())
            .unwrap_or_else(|_| {
                env.add_shutdown_action("_stop", reactor_key)
                    .expect("Duplicate shutdown Action?")
            });

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

        let timer_startup = Arc::new(
            move |ctx: &mut runtime::Context,
                  _state: &mut dyn runtime::ReactorState,
                  _inputs: &[runtime::IPort],
                  _outputs: &mut [runtime::OPort],
                  actions: &mut [&mut runtime::Action]| {
                let [_startup, timer]: &mut [&mut runtime::Action; 2usize] =
                    actions.try_into().unwrap();
                let mut timer: runtime::ActionRef = (*timer).into();
                ctx.schedule_action(&mut timer, None, offset);
            },
        );

        let timer_reset = Arc::new(
            move |ctx: &mut runtime::Context,
                  _state: &mut dyn runtime::ReactorState,
                  _inputs: &[runtime::IPort],
                  _outputs: &mut [runtime::OPort],
                  actions: &mut [&mut runtime::Action]| {
                let [timer]: &mut [&mut runtime::Action; 1usize] = actions.try_into().unwrap();
                let mut timer: runtime::ActionRef = (*timer).into();
                ctx.schedule_action(&mut timer, None, period);
            },
        );

        let startup_key = self.startup_action;
        self.add_reaction(&format!("_{name}_startup"), timer_startup)
            .with_trigger_action(startup_key, 0)
            .with_schedulable_action(action_key, 1)
            .finish()?;

        if period.is_some() {
            self.add_reaction(&format!("_{name}_reset"), timer_reset)
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
        reaction_fn: Arc<dyn runtime::ReactionFn>,
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
    ) -> Result<(BuilderReactorKey, R), BuilderError> {
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

    /// Adopt an existing child reactor into this reactor.
    pub fn adopt_existing_child(&mut self, child_reactor_key: BuilderReactorKey) {
        self.env.reactor_builders[child_reactor_key].parent_reactor_key = Some(self.reactor_key);
    }

    /// Clone existing actions from `reactor_key` into this reactor.
    ///
    /// Returns a mapping of existing action keys to new action keys.
    pub fn clone_reactor_actions(
        &mut self,
        reactor_key: BuilderReactorKey,
    ) -> SecondaryMap<BuilderActionKey, BuilderActionKey> {
        let mut mapping = SecondaryMap::new();

        // Find the startup and shutdown actions in the existing reactor
        let existing_reactor = &self.env.reactor_builders[reactor_key];
        let existing_startup = self
            .get_action_by_name("_start")
            .expect("Reactor has no startup action?");
        let existing_shutdown = self
            .get_action_by_name("_stop")
            .expect("Reactor has no shutdown action?");

        mapping.insert(existing_startup, self.startup_action.into());
        mapping.insert(existing_shutdown, self.shutdown_action.into());

        let cloned_actions = existing_reactor
            .actions
            .iter()
            .filter_map(|(action_key, action)| {
                if action_key == existing_startup || action_key == existing_shutdown {
                    None
                } else {
                    Some((action_key, action.clone()))
                }
            })
            .collect_vec();

        for (action_key, action) in cloned_actions {
            let new_action_key = self.env.reactor_builders[self.reactor_key]
                .actions
                .insert(action);

            mapping.insert(action_key, new_action_key);
        }

        mapping
    }

    /// Clone existing reactions into this reactor.
    pub fn clone_existing_reactions(
        &mut self,
        reactions: impl Iterator<Item = BuilderReactionKey>,
        action_mapping: &SecondaryMap<BuilderActionKey, BuilderActionKey>,
    ) {
        for reaction_key in reactions {
            let mut cloned_reaction = self.env.reaction_builders[reaction_key].clone();
            cloned_reaction.reactor_key = self.reactor_key;

            // replace any trigger actions in the cloned reaction with the new action keys from `action_mapping`
            let new_triggers = cloned_reaction
                .trigger_actions
                .iter()
                .map(|(action_key, order)| (action_mapping[action_key], *order))
                .collect();

            // replace any schedulable actions in the cloned reaction with the new action keys from `action_mapping`
            let new_schedulable = cloned_reaction
                .schedulable_actions
                .iter()
                .map(|(action_key, order)| (action_mapping[action_key], *order))
                .collect();

            cloned_reaction.trigger_actions = new_triggers;
            cloned_reaction.schedulable_actions = new_schedulable;

            let cloned_reaction_key = self.env.reaction_builders.insert_with_key(|reaction_key| {
                // update the action's triggers to include the new reaction
                for action_key in cloned_reaction.trigger_actions.keys() {
                    self.env.reactor_builders[self.reactor_key].actions[action_key]
                        .triggers
                        .insert(reaction_key, ());
                }

                cloned_reaction
            });

            self.env.reactor_builders[self.reactor_key]
                .reactions
                .insert(cloned_reaction_key, ());
        }
    }

    pub fn finish(self) -> Result<BuilderReactorKey, BuilderError> {
        Ok(self.reactor_key)
    }
}
