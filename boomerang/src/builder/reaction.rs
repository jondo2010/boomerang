use super::{BuilderError, EnvBuilder, PortType};
use crate::runtime;

use runtime::PortData;
use slotmap::SecondaryMap;

#[derive(Derivative, Eq)]
#[derivative(Debug)]
pub struct ReactionBuilder {
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: runtime::ReactorKey,
    #[derivative(Debug = "ignore")]
    pub(super) reaction_fn: runtime::ReactionFn,
    trigger_actions: SecondaryMap<runtime::BaseActionKey, ()>,
    schedulable_actions: SecondaryMap<runtime::BaseActionKey, ()>,
    trigger_ports: SecondaryMap<runtime::BasePortKey, ()>,
    pub(super) deps: SecondaryMap<runtime::BasePortKey, ()>,
    pub(super) antideps: SecondaryMap<runtime::BasePortKey, ()>,
}

impl Ord for ReactionBuilder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.reactor_key.cmp(&other.reactor_key))
    }
}

impl PartialOrd for ReactionBuilder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

impl PartialEq for ReactionBuilder {
    fn eq(&self, other: &Self) -> bool {
        self.priority.eq(&other.priority) && self.reactor_key.eq(&other.reactor_key)
    }
}

pub struct ReactionBuilderState<'a> {
    reaction: ReactionBuilder,
    env: &'a mut EnvBuilder,
}

impl<'a> ReactionBuilderState<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reactor_key: runtime::ReactorKey,
        reaction_fn: runtime::ReactionFn,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            reaction: ReactionBuilder {
                name: name.into(),
                priority,
                reactor_key,
                reaction_fn,
                trigger_ports: SecondaryMap::new(),
                schedulable_actions: SecondaryMap::new(),
                trigger_actions: SecondaryMap::new(),
                deps: SecondaryMap::new(),
                antideps: SecondaryMap::new(),
            },
            env,
        }
    }

    pub fn with_trigger_action(mut self, trigger_key: runtime::BaseActionKey) -> Self {
        self.reaction.trigger_actions.insert(trigger_key, ());
        self
    }

    pub fn with_trigger_port<T: PortData>(mut self, port_key: runtime::PortKey<T>) -> Self {
        self.reaction.trigger_ports.insert(port_key.into(), ());
        self.reaction.deps.insert(port_key.into(), ());
        self
    }

    pub fn with_scheduable_action(mut self, action_key: runtime::BaseActionKey) -> Self {
        self.reaction.schedulable_actions.insert(action_key, ());
        self
    }

    pub fn with_antidependency<T: PortData>(mut self, antidep_key: runtime::PortKey<T>) -> Self {
        self.reaction.antideps.insert(antidep_key.into(), ());
        self
    }

    pub fn finish(self) -> Result<runtime::ReactionKey, BuilderError> {
        let Self { reaction: reaction_builder, env } = self;
        let reactor = &mut env.reactors[reaction_builder.reactor_key];
        let reactions = &mut env.reaction_builders;
        let actions = &mut env.action_builders;
        let ports = &mut env.port_builders;

        let key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());

            for trigger_key in reaction_builder.trigger_actions.keys() {
                let action = actions.get_mut(trigger_key).unwrap();
                assert!(
                    reaction_builder.reactor_key == action.get_reactor_key(),
                    "Action triggers must belong to the same reactor as the triggered reaction"
                );
                action.triggers.insert(key, ());
            }

            for action_key in reaction_builder.schedulable_actions.keys() {
                let action = actions.get_mut(action_key).unwrap();
                assert!(
                    action.get_reactor_key() == reaction_builder.reactor_key,
                    "Scheduable actions must belong to the same reactor as the triggered reaction"
                );
                action.schedulers.insert(key, ());
            }

            for port_key in reaction_builder.deps.keys() {
                let port = ports.get_mut(port_key).unwrap();
                if port.get_port_type() == &PortType::Input {
                    assert!(reaction_builder.reactor_key == port.get_reactor_key(), "Input port triggers must belong to the same reactor as the triggered reaction");
                } else {
                    //assert!(reaction.reactor_key == env.reactors[port.get_reactor_key()].parent_reactor_key.unwrap(), "Output port triggers must belong to a contained reactor");
                    todo!();
                }
                port.register_dependency(key, true);
            }

            for antidep_key in reaction_builder.antideps.keys() {
                let port = ports.get_mut(antidep_key).unwrap();
                port.register_antidependency(key);
            }

            reaction_builder
        });

        Ok(key)
    }
}