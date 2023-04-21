use itertools::Itertools;
use slotmap::SecondaryMap;
use std::{fmt::Debug, sync::Arc};

use boomerang_runtime as runtime;

use crate::util::DebugMap;

use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, FindElements,
    PortType,
};

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

#[derive(Derivative, Clone)]
pub struct ReactionBuilder {
    #[derivative(Ord = "ignore")]
    #[derivative(PartialEq = "ignore")]
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: BuilderReactorKey,
    /// The Reaction function
    pub(super) reaction_fn: Arc<dyn runtime::ReactionFn>,
    /// Actions that trigger this Reaction, and their relative ordering.
    pub(super) trigger_actions: SecondaryMap<BuilderActionKey, usize>,
    /// Actions that can be scheduled by this Reaction, and their relative ordering.
    pub(super) schedulable_actions: SecondaryMap<BuilderActionKey, usize>,
    /// Ports that can trigger this Reaction, and their relative ordering.
    pub(super) input_ports: SecondaryMap<BuilderPortKey, usize>,
    /// Ports that this Reaction may set the value of, and their relative ordering.
    pub(super) output_ports: SecondaryMap<BuilderPortKey, usize>,
}

impl Debug for ReactionBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactionBuilder")
            .field("name", &self.name)
            .field("priority", &self.priority)
            .field("reactor_key", &self.reactor_key)
            .field("trigger_actions", &DebugMap(&self.trigger_actions))
            .field("schedulable_actions", &DebugMap(&self.schedulable_actions))
            .field("input_ports", &DebugMap(&self.input_ports))
            .field("output_ports", &DebugMap(&self.output_ports))
            .finish()
    }
}

impl ReactionBuilder {
    /// Build a [`runtime::Reaction`] from this `ReactionBuilder`.
    pub fn build_reaction(
        &self,
        reactor_key: runtime::keys::ReactorKey,
        port_aliases: &SecondaryMap<BuilderPortKey, runtime::keys::PortKey>,
        action_aliases: &SecondaryMap<BuilderActionKey, runtime::keys::ActionKey>,
    ) -> runtime::Reaction {
        // Create the Vec of input ports for this reaction sorted by order
        let inputs = self
            .input_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        // Create the Vec of output ports for this reaction sorted by order
        let outputs = self
            .output_ports
            .iter()
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_port_key, _)| port_aliases[builder_port_key])
            .collect();

        let actions = self
            .trigger_actions
            .iter()
            .chain(self.schedulable_actions.iter())
            .sorted_by_key(|(_, &order)| order)
            .map(|(builder_action_key, _)| action_aliases[builder_action_key])
            .dedup()
            .collect();

        runtime::Reaction::new(
            self.name.clone(),
            reactor_key,
            inputs,
            outputs,
            actions,
            self.reaction_fn.clone(),
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
        self.env
            .find_port_by_name(port_name, self.builder.reactor_key)
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
        reaction_fn: Arc<dyn runtime::ReactionFn>,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: name.into(),
                priority,
                reactor_key,
                reaction_fn,
                trigger_actions: SecondaryMap::new(),
                schedulable_actions: SecondaryMap::new(),
                input_ports: SecondaryMap::new(),
                output_ports: SecondaryMap::new(),
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
        self.builder.input_ports.insert(port_key.into(), order);
        self
    }

    /// Indicate that this Reaction may schedule the given Action
    pub fn with_schedulable_action(
        mut self,
        action_key: impl Into<BuilderActionKey>,
        order: usize,
    ) -> Self {
        self.builder
            .schedulable_actions
            .insert(action_key.into(), order);
        self
    }

    /// Indicate that this Reaction may set the value of the given Port (uses keyword).
    pub fn with_antidependency(
        mut self,
        antidep_key: impl Into<BuilderPortKey>,
        order: usize,
    ) -> Self {
        self.builder.output_ports.insert(antidep_key.into(), order);
        self
    }

    pub fn finish(self) -> Result<BuilderReactionKey, BuilderError> {
        let Self {
            builder: reaction_builder,
            env,
        } = self;

        let reaction_key = env.reaction_builders.insert_with_key(|key| {
            env.reactor_builders[reaction_builder.reactor_key]
                .reactions
                .insert(key, ());
            reaction_builder
        });

        let reaction_builder = &env.reaction_builders[reaction_key];

        for trigger_key in reaction_builder.trigger_actions.keys() {
            let action = env.reactor_builders[reaction_builder.reactor_key]
                .actions
                .get_mut(trigger_key)
                .unwrap();
            action.triggers.insert(reaction_key, ());
        }

        for action_key in reaction_builder.schedulable_actions.keys() {
            let action = env.reactor_builders[reaction_builder.reactor_key]
                .actions
                .get_mut(action_key)
                .unwrap();
            action.schedulers.insert(reaction_key, ());
        }

        for port_key in reaction_builder.output_ports.keys() {
            let port = env.port_builders.get_mut(port_key).unwrap();

            if port.get_port_type() == PortType::Output {
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

        for port_key in reaction_builder.input_ports.keys() {
            let port = &env.port_builders[port_key];

            if port.get_port_type() == PortType::Input {
                assert_eq!(
                    reaction_builder.reactor_key,
                    port.get_reactor_key(),
                    "Input port triggers must belong to the same reactor as the triggered reaction"
                );
            } else {
                let parent_reactor_key =
                    env.reactor_builders[port.get_reactor_key()].parent_reactor_key;
                let port_fqn = env.port_fqn(port_key)?;
                let reactor_fqn = env.reactor_fqn(reaction_builder.reactor_key)?;
                let reaction_fqn = env.reaction_fqn(reaction_key)?;

                if let Some(parent_reactor_key) = parent_reactor_key {
                    if parent_reactor_key != reaction_builder.reactor_key {
                        let parent_fqn = env.reactor_fqn(parent_reactor_key)?;
                        return Err(BuilderError::Other(anyhow::anyhow!(
                            "Output port triggers must belong to a contained reactor. Port '{port_fqn}' is an `PortType::Output` set as a trigger for '{reaction_fqn}', but should be within '{parent_fqn}'.",
                        )));
                    }
                } else {
                    return Err(BuilderError::Other(anyhow::anyhow!(
                        "Output port triggers must belong to a contained reactor. Port '{port_fqn}' is an output port contained in {reactor_fqn}, but it has no containing reactor.",
                    )));
                }
            }

            env.port_builders[port_key].register_dependency(reaction_key, true);
        }

        Ok(reaction_key)
    }
}
