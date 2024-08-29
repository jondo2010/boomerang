use std::marker::PhantomData;

use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, FindElements,
    Physical, PortType, Reactor, ReactorBuilderState, TypedActionKey, TypedPortKey,
};
use crate::runtime;
use itertools::Itertools;
use slotmap::SecondaryMap;

slotmap::new_key_type! {
    pub struct BuilderReactionKey;
}

#[derive(Copy, Debug)]
pub struct TypedReactionKey<R: Reaction>(BuilderReactionKey, PhantomData<R>);

impl<R: Reaction> Clone for TypedReactionKey<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<R: Reaction> Default for TypedReactionKey<R> {
    fn default() -> Self {
        Self(Default::default(), Default::default())
    }
}

impl<R: Reaction> TypedReactionKey<R> {
    pub fn new(reaction_key: BuilderReactionKey) -> Self {
        Self(reaction_key, PhantomData::default())
    }
}

impl petgraph::graph::GraphIndex for BuilderReactionKey {
    fn index(&self) -> usize {
        self.0.as_ffi() as usize
    }

    fn is_node_index() -> bool {
        true
    }
}

/// The `Trigger` trait should be implemented by the user for each Reaction struct.
pub trait Trigger {
    /// The type of the owning Reactor
    type Reactor: Reactor;

    fn trigger(
        &mut self,
        ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    );
}

/// The Reaction trait should be automatically derived for each Reaction struct.
pub trait Reaction: Trigger {
    /// Build a `ReactionBuilderState` for this Reaction
    fn build<'builder>(
        name: &str,
        reactor: &Self::Reactor,
        builder: &'builder mut ReactorBuilderState,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError>;
}

pub trait ReactionField {
    type Key;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError>;
}

impl<T: runtime::ActionData> ReactionField for runtime::ActionRef<'_, T> {
    type Key = TypedActionKey<T>;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action(key.into(), order, trigger_mode)
    }
}

impl<T: runtime::ActionData> ReactionField for runtime::PhysicalActionRef<T> {
    type Key = TypedActionKey<T, Physical>;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action(key.into(), order, trigger_mode)
    }
}

impl<T: runtime::PortData> ReactionField for runtime::Port<T> {
    type Key = TypedPortKey<T>;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port(key.into(), order, trigger_mode)
    }
}

pub struct PortOrActionTrigger;
pub enum PortOrActionTriggerKey {
    Port(BuilderPortKey),
    Action(BuilderActionKey),
}
impl From<BuilderPortKey> for PortOrActionTriggerKey {
    fn from(key: BuilderPortKey) -> Self {
        Self::Port(key)
    }
}
impl From<BuilderActionKey> for PortOrActionTriggerKey {
    fn from(key: BuilderActionKey) -> Self {
        Self::Action(key)
    }
}

impl ReactionField for PortOrActionTrigger {
    type Key = PortOrActionTriggerKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        match key {
            PortOrActionTriggerKey::Port(port_key) => {
                builder.add_port(port_key, order, trigger_mode)
            }
            PortOrActionTriggerKey::Action(action_key) => {
                builder.add_action(action_key, order, trigger_mode)
            }
        }
    }
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ReactionBuilder {
    #[derivative(Ord = "ignore")]
    #[derivative(PartialEq = "ignore")]
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: BuilderReactorKey,
    /// The Reaction function
    #[derivative(Debug = "ignore")]
    pub(super) reaction_fn: runtime::ReactionFn,

    /// Actions that trigger this Reaction, and their relative ordering.
    pub(super) trigger_actions: SecondaryMap<BuilderActionKey, usize>,
    /// Actions that can be read or scheduled by this Reaction, and their relative ordering.
    pub(super) use_effect_actions: SecondaryMap<BuilderActionKey, usize>,

    /// Ports that can trigger this Reaction, and their relative ordering.
    pub(super) trigger_ports: SecondaryMap<BuilderPortKey, usize>,
    /// Ports that this Reaction may read the value of, and their relative ordering. These are used to build the array of [`runtime::PortRef`] in the reaction function.
    pub(super) use_ports: SecondaryMap<BuilderPortKey, usize>,
    /// Ports that this Reaction may set the value of, and their relative ordering. These are used to build the array of [`runtime::PortRefMut`]` in the reaction function.
    pub(super) effect_ports: SecondaryMap<BuilderPortKey, usize>,
}

impl ReactionBuilder {
    /// Get the name of this Reaction
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Get the BuilderReactorKey of this Reaction
    pub fn get_reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    /// Build a [`runtime::Reaction`] from this `ReactionBuilder`.
    pub fn build_runtime_reaction(
        self,
        reactor_key: runtime::ReactorKey,
        port_aliases: &SecondaryMap<BuilderPortKey, runtime::PortKey>,
        action_aliases: &SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    ) -> runtime::Reaction {
        // Create the Vec of readable ports for this reaction sorted by order
        let use_ports = self
            .use_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        // Create the Vec of writable ports for this reaction sorted by order
        let effect_ports = self
            .effect_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        // Create the Vec of actions for this reaction sorted by order
        let actions = self
            .use_effect_actions
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_action_key, _)| action_aliases[builder_action_key])
            .dedup()
            .collect();

        runtime::Reaction::new(
            self.name,
            reactor_key,
            use_ports,
            effect_ports,
            actions,
            self.reaction_fn,
            None,
        )
    }
}

pub struct ReactionBuilderState<'a> {
    builder: ReactionBuilder,
    env: &'a mut EnvBuilder,
}

impl<'a> FindElements for ReactionBuilderState<'a> {
    /// Find the PortKey with a given name within the parent Reactor
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError> {
        self.env.get_port(port_name, self.builder.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError> {
        self.env
            .find_action_by_name(action_name, self.builder.reactor_key)
    }
}

/// Describes how an action is used by a reaction
pub enum TriggerMode {
    /// The action/port triggers the reaction, but is not provided as input
    TriggersOnly,
    /// The action/port triggers the reaction and is provided as input in the actions/ports arrays
    TriggersAndUses,
    /// The action/port triggers the reaction and is provided to the reaction in the actions/mut ports arrays
    TriggersAndEffects,
    /// The action/port does not trigger the reaction, but is provided as input in the actions/ports arrays
    UsesOnly,
    /// The action/port does not trigger the reaction, but is provided to the reaction in the actions/mut ports arrays
    EffectsOnly,
}

impl<'a> ReactionBuilderState<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reactor_key: BuilderReactorKey,
        reaction_fn: runtime::ReactionFn,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: name.into(),
                priority,
                reactor_key,
                reaction_fn,
                trigger_actions: SecondaryMap::new(),
                use_effect_actions: SecondaryMap::new(),
                trigger_ports: SecondaryMap::new(),
                use_ports: SecondaryMap::new(),
                effect_ports: SecondaryMap::new(),
            },
            env,
        }
    }

    pub fn add_action(
        &mut self,
        key: BuilderActionKey,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        let action = &self.env.action_builders[key];
        if action.get_reactor_key() != self.builder.reactor_key {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Cannot add action '{}' to ReactionBuilder '{}', it must belong to the same reactor as the reaction",
                action.get_name(), &self.builder.name
            )));
        }

        match trigger_mode {
            TriggerMode::TriggersOnly => {
                self.builder.trigger_actions.insert(key, order);
            }
            TriggerMode::TriggersAndEffects | TriggerMode::TriggersAndUses => {
                self.builder.trigger_actions.insert(key, order);
                self.builder.use_effect_actions.insert(key, order);
            }
            TriggerMode::UsesOnly | TriggerMode::EffectsOnly => {
                self.builder.use_effect_actions.insert(key, order);
            }
        }
        Ok(())
    }

    /// Indicate how this Reaction interacts with the given Action
    ///
    /// There must be at least one trigger for each reaction.
    pub fn with_action(
        mut self,
        action_key: impl Into<BuilderActionKey>,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_action(action_key.into(), order, trigger_mode)?;
        Ok(self)
    }

    /// For triggers: valid ports are input ports in this reactor, (or output ports of contained reactors).
    /// For uses: valid ports are input ports in this reactor, (or output ports of contained reactors).
    /// for effects: valid ports are output ports in this reactor, (or input ports of contained reactors).
    pub fn add_port(
        &mut self,
        key: BuilderPortKey,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        let port_builder = &self.env.port_builders[key];
        let port_reactor_key = port_builder.get_reactor_key();
        let port_parent_reactor_key =
            self.env.reactor_builders[port_reactor_key].parent_reactor_key;

        #[cfg(feature = "fixme")]
        match (port_builder.get_port_type(), trigger_mode) {
            (PortType::Input, TriggerMode::TriggersOnly | TriggerMode::TriggersAndUses | TriggerMode::TriggersAndEffects) if port_reactor_key != self.builder.reactor_key => {
                Err(BuilderError::ReactionBuilderError(format!(
                    "Reaction {} cannot 'trigger on' input port '{}', it must belong to the same reactor as the reaction",
                    self.builder.get_name(),
                    port_builder.get_name()
                )))
            }

            PortType::Output if (triggers || uses) && (port_parent_reactor_key != Some(self.builder.reactor_key)) => {
                Err(BuilderError::ReactionBuilderError(format!(
                    "Reaction {} cannot 'trigger on' output port '{}', it must belong to a contained reactor",
                    self.builder.get_name(),
                    port_builder.get_name()
                )))
            }

            PortType::Input if effects && port_parent_reactor_key != Some(self.builder.reactor_key) => {
                Err(BuilderError::ReactionBuilderError(format!(
                    "Reaction {} cannot 'effect' input port '{}', it must belong to a contained reactor",
                    self.builder.get_name(),
                    port_builder.get_name()
                )))
            }

            PortType::Output if effects && port_reactor_key != self.builder.reactor_key => {
                Err(BuilderError::ReactionBuilderError(format!(
                    "Reaction {} cannot 'effect' output port '{}', it must belong to the same reactor as the reaction",
                    self.builder.get_name(),
                    port_builder.get_name()
                )))
            }

            _ => Ok(())
        }?;

        match trigger_mode {
            TriggerMode::TriggersOnly => {
                self.builder.trigger_ports.insert(key, order);
                Ok(())
            }

            TriggerMode::TriggersAndUses => {
                self.builder.trigger_ports.insert(key, order);
                self.builder.use_ports.insert(key, order);
                Ok(())
            }

            TriggerMode::TriggersAndEffects => {
                self.builder.trigger_ports.insert(key, order);
                self.builder.effect_ports.insert(key, order);
                Ok(())
            }

            TriggerMode::UsesOnly => {
                self.builder.use_ports.insert(key, order);
                Ok(())
            }

            TriggerMode::EffectsOnly => {
                self.builder.effect_ports.insert(key, order);
                Ok(())
            }
        }
    }

    /// Indicate how this Reaction interacts with the given Port
    ///
    /// There must be at least one trigger for each reaction.
    pub fn with_port(
        mut self,
        port_key: impl Into<BuilderPortKey>,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_port(port_key.into(), order, trigger_mode)?;
        Ok(self)
    }

    pub fn finish(self) -> Result<BuilderReactionKey, BuilderError> {
        let Self {
            builder: reaction_builder,
            env,
        } = self;

        // Ensure there is at least one trigger declared
        if reaction_builder.trigger_actions.is_empty() && reaction_builder.trigger_ports.is_empty()
        {
            return Err(BuilderError::ReactionBuilderError(
                "Reactions must have at least one trigger".to_string(),
            ));
        }

        let reactor = &mut env.reactor_builders[reaction_builder.reactor_key];
        let reactions = &mut env.reaction_builders;
        let actions = &mut env.action_builders;
        let ports = &mut env.port_builders;

        let reaction_key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());
            reaction_builder
        });

        let reaction_builder = &reactions[reaction_key];

        for action_key in reaction_builder.trigger_actions.keys() {
            let action = &mut actions[action_key];
            action.triggers.insert(reaction_key, ());
        }

        for action_key in reaction_builder.use_effect_actions.keys() {
            let action = &mut actions[action_key];
            action.schedulers.insert(reaction_key, ());
        }

        for port_key in reaction_builder.effect_ports.keys() {
            let port = ports.get_mut(port_key).unwrap();

            if port.get_port_type() == &PortType::Output {
                assert_eq!(
                    reaction_builder.reactor_key,
                    port.get_reactor_key(),
                    "Antidependent output ports must belong to the same reactor as the reaction"
                );
            } else {
                assert_eq!(
                    reaction_builder.reactor_key,
                    env.reactor_builders[port.get_reactor_key()]
                        .parent_reactor_key
                        .unwrap(),
                    "Antidependent input ports must belong to a contained reactor"
                );
            }

            port.register_antidependency(reaction_key);
        }

        // Both trigger_ports and use_ports are treated as dependencies
        for (port_key, is_trigger) in reaction_builder
            .trigger_ports
            .keys()
            .map(|key| (key, true))
            .chain(reaction_builder.use_ports.keys().map(|key| (key, false)))
        {
            let port = ports.get_mut(port_key).unwrap();
            // Note, these assertions are the same as the ones on the builder methods
            if port.get_port_type() == &PortType::Input {
                assert_eq!(
                    reaction_builder.reactor_key,
                    port.get_reactor_key(),
                    "Input port triggers must belong to the same reactor as the triggered reaction"
                );
            } else {
                assert_eq!(
                    reaction_builder.reactor_key,
                    env.reactor_builders[port.get_reactor_key()]
                        .parent_reactor_key
                        .unwrap(),
                    "Output port triggers must belong to a contained reactor"
                );
            }

            port.register_dependency(reaction_key, is_trigger);
        }

        Ok(reaction_key)
    }
}
