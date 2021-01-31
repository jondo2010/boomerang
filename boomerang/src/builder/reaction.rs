use super::{BuilderError, EnvBuilder, PortType};
use crate::runtime::{self};

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

    pub fn with_trigger_port(mut self, port_key: runtime::BasePortKey) -> Self {
        self.reaction.trigger_ports.insert(port_key, ());
        self.reaction.deps.insert(port_key, ());
        self
    }

    pub fn with_scheduable_action(mut self, action_key: runtime::BaseActionKey) -> Self {
        self.reaction.schedulable_actions.insert(action_key, ());
        self
    }

    pub fn with_antidependency(mut self, antidep_key: runtime::BasePortKey) -> Self {
        self.reaction.antideps.insert(antidep_key, ());
        self
    }

    pub fn finish(self) -> Result<runtime::ReactionKey, BuilderError> {
        let Self { reaction, env } = self;
        let reactions = &mut env.reactions;
        let actions = &mut env.actions;
        let port_builders = &mut env.port_builders;

        let key = reactions.insert_with_key(|key| {
            for trigger_key in reaction.trigger_actions.keys() {
                let action = actions.get_mut(trigger_key).unwrap();
                assert!(
                    reaction.reactor_key == action.get_reactor_key(),
                    "Action triggers must belong to the same reactor as the triggered reaction"
                );
                action.triggers.insert(key, ());
            }

            for action_idx in reaction.schedulable_actions.iter() {
                let trig = actions.get_mut(action_idx.0).unwrap();
                // ASSERT(this->environment() == action->environment());
                // VALIDATE(this->container() == action->container(), "Scheduable actions must
                // belong to the same reactor as the triggered reaction");
                trig.schedulers.insert(key, ());
            }

            for port_key in reaction.deps.keys() {
                let port = port_builders.get_mut(port_key).unwrap();
                if port.get_port_type() == &PortType::Input {
                    // assert!( this->container() == port->container(), "Input port triggers must
                    // belong to the same reactor as the triggered reaction");
                } else {
                    // assert!(this->container() == port->container()->container(), "Output port
                    // triggers must belong to a contained reactor");
                }
                port.register_dependency(key, true);
            }

            for antidep_key in reaction.antideps.keys() {
                let port = port_builders.get_mut(antidep_key).unwrap();
                port.register_antidependency(key);
            }

            reaction
        });

        Ok(key)
    }
}
