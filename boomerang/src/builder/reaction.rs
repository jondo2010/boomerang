use super::{BuilderActionKey, BuilderError, BuilderPortKey, EnvBuilder, FindElements, PortType};
use crate::runtime;
use slotmap::SecondaryMap;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ReactionBuilder {
    #[derivative(Ord = "ignore")]
    #[derivative(PartialEq = "ignore")]
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: runtime::ReactorKey,
    /// The Reaction function
    #[derivative(Debug = "ignore")]
    pub(super) reaction_fn: Box<dyn runtime::ReactionFn>,
    /// Actions that trigger this Reaction, and their relative ordering.
    pub(super) trigger_actions: SecondaryMap<runtime::ActionKey, usize>,
    /// Actions that can be scheduled by this Reaction, and their relative ordering.
    pub(super) schedulable_actions: SecondaryMap<runtime::ActionKey, usize>,
    /// Ports that can trigger this Reaction, and their relative ordering.
    pub(super) input_ports: SecondaryMap<runtime::PortKey, usize>,
    /// Ports that this Reaction may set the value of, and their relative ordering.
    pub(super) output_ports: SecondaryMap<runtime::PortKey, usize>,
}

impl From<ReactionBuilder> for runtime::Reaction {
    fn from(builder: ReactionBuilder) -> Self {
        Self::new(builder.name, builder.reactor_key, builder.reaction_fn, None)
    }
}

pub struct ReactionBuilderState<'a> {
    builder: ReactionBuilder,
    env: &'a mut EnvBuilder,
}

impl<'a> FindElements for ReactionBuilderState<'a> {
    /// Find the PortKey with a given name within the parent Reactor
    fn get_port_by_name(&self, port_name: &str) -> Result<runtime::PortKey, BuilderError> {
        self.env.get_port(port_name, self.builder.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<runtime::ActionKey, BuilderError> {
        self.env.get_action(action_name, self.builder.reactor_key)
    }
}

impl<'a> ReactionBuilderState<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reactor_key: runtime::ReactorKey,
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
                schedulable_actions: SecondaryMap::new(),
                input_ports: SecondaryMap::new(),
                output_ports: SecondaryMap::new(),
            },
            env,
        }
    }

    /// Indicate that this Reaction can be triggered by the given Action
    pub fn with_trigger_action<T: runtime::PortData>(
        mut self,
        trigger_key: BuilderActionKey<T>,
        order: usize,
    ) -> Self {
        self.builder
            .trigger_actions
            .insert(trigger_key.into(), order);
        self
    }

    /// Indicate that this Reaction can be triggered by the given Port
    pub fn with_trigger_port<T: runtime::PortData>(
        mut self,
        port_key: BuilderPortKey<T>,
        order: usize,
    ) -> Self {
        self.builder.input_ports.insert(port_key.into(), order);
        self
    }

    /// Indicate that this Reaction may schedule the given Action
    pub fn with_scheduable_action<T: runtime::PortData>(
        mut self,
        action_key: BuilderActionKey<T>,
        order: usize,
    ) -> Self {
        self.builder
            .schedulable_actions
            .insert(action_key.into(), order);
        self
    }

    // pub fn with_scheduable_actions(mut self, action_keys: &[runtime::ActionKey]) -> Self {
    //    self.builder
    //        .schedulable_actions
    //        .extend(action_keys.iter().map(|&key| (key, ())));
    //    self
    //}

    /// Indicate that this Reaction may set the value of the given Port (uses keyword).
    pub fn with_antidependency<T: runtime::PortData>(
        mut self,
        antidep_key: BuilderPortKey<T>,
        order: usize,
    ) -> Self {
        self.builder.output_ports.insert(antidep_key.into(), order);
        self
    }

    pub fn finish(self) -> Result<runtime::ReactionKey, BuilderError> {
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

        for trigger_key in reaction_builder.trigger_actions.keys() {
            let action = actions.get_mut(trigger_key).unwrap();
            assert_eq!(
                reaction_builder.reactor_key,
                action.get_reactor_key(),
                "Action triggers must belong to the same reactor as the triggered reaction"
            );
            action.triggers.insert(reaction_key, ());
        }

        for action_key in reaction_builder.schedulable_actions.keys() {
            let action = actions.get_mut(action_key).unwrap();
            assert_eq!(
                action.get_reactor_key(),
                reaction_builder.reactor_key,
                "Scheduable actions must belong to the same reactor as the triggered reaction"
            );
            action.schedulers.insert(reaction_key, ());
        }

        for port_key in reaction_builder.output_ports.keys() {
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

        for port_key in reaction_builder.input_ports.keys() {
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
