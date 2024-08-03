use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, FindElements,
    PortType, Reactor, ReactorBuilderState, TypedActionKey, TypedPortKey,
};
use crate::runtime;
use itertools::Itertools;
use slotmap::SecondaryMap;

slotmap::new_key_type! {
    pub struct BuilderReactionKey;
}

impl petgraph::graph::GraphIndex for BuilderReactionKey {
    fn index(&self) -> usize {
        self.0.as_ffi() as usize
    }

    fn is_node_index() -> bool {
        true
    }
}

pub trait Reaction {
    type BuilderReactor: Reactor;

    /// Build a `ReactionBuilderState` for this Reaction
    fn build<'builder>(
        name: &str,
        reactor: &Self::BuilderReactor,
        builder: &'builder mut ReactorBuilderState,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError>;

    /// Marshall the runtime queried inputs, outputs, and actions into this Reaction struct
    fn marshall(
        inputs: &[runtime::IPort],
        outputs: &mut [runtime::OPort],
        actions: &mut [&mut runtime::Action],
    ) -> Self;
}

pub trait ReactionField {
    type Key;
    fn build(
        builder: ReactionBuilderState,
        key: Self::Key,
        order: usize,
        triggers: bool,
        effects: bool,
    ) -> ReactionBuilderState;
}

impl<T: runtime::ActionData> ReactionField for runtime::ActionRef<'_, T> {
    type Key = TypedActionKey<T>;

    fn build(
        builder: ReactionBuilderState,
        key: Self::Key,
        order: usize,
        triggers: bool,
        effects: bool,
    ) -> ReactionBuilderState {
        match (triggers, effects) {
            (true, false) => builder.with_trigger_action(key, order),
            (false, true) => builder.with_effect_action(key, order),
            (true, true) => builder
                .with_trigger_action(key.clone(), order)
                .with_effect_action(key, order),
            _ => builder,
        }
    }
}

impl<T: runtime::PortData> ReactionField for runtime::Port<T> {
    type Key = TypedPortKey<T>;

    fn build(
        builder: ReactionBuilderState,
        key: Self::Key,
        order: usize,
        triggers: bool,
        effects: bool,
    ) -> ReactionBuilderState {
        match (triggers, effects) {
            (true, false) => builder.with_trigger_port(key, order),
            (false, true) => builder.with_effect_port(key, order),
            (true, true) => panic!("Cannot have a port that is both a trigger and an effect"),
            _ => builder,
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
    pub(super) reaction_fn: Box<dyn runtime::ReactionFn>,

    /// Actions that trigger this Reaction, and their relative ordering.
    pub(super) trigger_actions: SecondaryMap<BuilderActionKey, usize>,
    /// Actions that this Reaction may read the value of, and their relative ordering.
    pub(super) use_actions: SecondaryMap<BuilderActionKey, usize>,
    /// Actions that can be scheduled by this Reaction, and their relative ordering.
    pub(super) effect_actions: SecondaryMap<BuilderActionKey, usize>,

    /// Ports that can trigger this Reaction, and their relative ordering.
    pub(super) trigger_ports: SecondaryMap<BuilderPortKey, usize>,
    /// Ports that this Reaction may read the value of, and their relative ordering.
    pub(super) use_ports: SecondaryMap<BuilderPortKey, usize>,
    /// Ports that this Reaction may set the value of, and their relative ordering.
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
    pub fn build_reaction(
        self,
        reactor_key: runtime::ReactorKey,
        port_aliases: &SecondaryMap<BuilderPortKey, runtime::PortKey>,
        action_aliases: &SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    ) -> runtime::Reaction {
        // Create the Vec of input ports for this reaction sorted by order
        let inputs = self
            .trigger_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        // Create the Vec of output ports for this reaction sorted by order
        let outputs = self
            .effect_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        let actions = self
            .trigger_actions
            .iter()
            .chain(self.effect_actions.iter())
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_action_key, _)| action_aliases[builder_action_key])
            .dedup()
            .collect();

        runtime::Reaction::new(
            self.name,
            reactor_key,
            inputs,
            outputs,
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

impl<'a> ReactionBuilderState<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reactor_key: BuilderReactorKey,
        reaction_fn: Box<dyn runtime::ReactionFn>,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: name.into(),
                priority,
                reactor_key,
                reaction_fn,
                trigger_actions: SecondaryMap::new(),
                use_actions: SecondaryMap::new(),
                effect_actions: SecondaryMap::new(),
                trigger_ports: SecondaryMap::new(),
                use_ports: SecondaryMap::new(),
                effect_ports: SecondaryMap::new(),
            },
            env,
        }
    }

    /// Indicate that this Reaction can be triggered by the given Action
    pub fn with_trigger_action(
        mut self,
        trigger_key: impl Into<BuilderActionKey>,
        order: usize,
    ) -> Self {
        self.builder
            .trigger_actions
            .insert(trigger_key.into(), order);
        self
    }

    /// Indicate that this Reaction can be triggered by the given Port
    pub fn with_trigger_port(mut self, port_key: impl Into<BuilderPortKey>, order: usize) -> Self {
        self.builder.trigger_ports.insert(port_key.into(), order);
        self
    }

    /// Indicate that this Reaction may read the value of the given action.
    pub fn with_uses_action<T: runtime::ActionData, Q>(
        mut self,
        action_key: TypedActionKey<T, Q>,
        order: usize,
    ) -> Self {
        self.builder.use_actions.insert(action_key.into(), order);
        self
    }

    /// Indicate that this Reaction may read the value of the given port.
    pub fn with_uses_port<T: runtime::PortData>(
        mut self,
        port_key: TypedPortKey<T>,
        order: usize,
    ) -> Self {
        self.builder.use_ports.insert(port_key.into(), order);
        self
    }

    /// Indicate that this Reaction may schedule the given action.
    pub fn with_effect_action<T: runtime::PortData, Q>(
        mut self,
        action_key: TypedActionKey<T, Q>,
        order: usize,
    ) -> Self {
        self.builder.effect_actions.insert(action_key.into(), order);
        self
    }

    /// Indicate that this Reaction may set the value of the given port.
    pub fn with_effect_port<T: runtime::PortData>(
        mut self,
        antidep_key: TypedPortKey<T>,
        order: usize,
    ) -> Self {
        self.builder.effect_ports.insert(antidep_key.into(), order);
        self
    }

    pub fn finish(self) -> Result<BuilderReactionKey, BuilderError> {
        let Self {
            builder: reaction_builder,
            env,
        } = self;

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

        for action_key in reaction_builder.effect_actions.keys() {
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
        for port_key in reaction_builder
            .trigger_ports
            .keys()
            .chain(reaction_builder.use_ports.keys())
        {
            let port = ports.get_mut(port_key).unwrap();
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

            port.register_dependency(reaction_key, true);
        }

        Ok(reaction_key)
    }
}
