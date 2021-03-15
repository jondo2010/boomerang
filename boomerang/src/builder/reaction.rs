use super::{BuilderError, EnvBuilder, PortType};
use crate::runtime;
use slotmap::SecondaryMap;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ReactionBuilder<S> {
    #[derivative(Ord = "ignore")]
    #[derivative(PartialEq = "ignore")]
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: runtime::ReactorKey,

    #[derivative(Debug = "ignore")]
    pub(super) reaction_fn: Box<dyn runtime::ReactionFn<S>>,

    trigger_actions: SecondaryMap<runtime::ActionKey, ()>,
    schedulable_actions: SecondaryMap<runtime::ActionKey, ()>,
    trigger_ports: SecondaryMap<runtime::PortKey, ()>,
    pub(super) deps: SecondaryMap<runtime::PortKey, ()>,
    pub(super) antideps: SecondaryMap<runtime::PortKey, ()>,
}

// impl Ord for ReactionBuilder {
// fn cmp(&self, other: &Self) -> std::cmp::Ordering {
// self.priority
// .cmp(&other.priority)
// .then_with(|| self.reactor_key.cmp(&other.reactor_key))
// }
// }
//
// impl PartialOrd for ReactionBuilder {
// fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
// Some(self.cmp(&other))
// }
// }
//
// impl PartialEq for ReactionBuilder {
// fn eq(&self, other: &Self) -> bool {
// self.priority.eq(&other.priority) && self.reactor_key.eq(&other.reactor_key)
// }
// }

pub struct ReactionBuilderState<'a, S> {
    reaction: ReactionBuilder<S>,
    env: &'a mut EnvBuilder<S>,
}

impl<'a, S> ReactionBuilderState<'a, S>
where
    S: runtime::SchedulerPoint,
{
    pub fn new<F>(
        name: &str,
        priority: usize,
        reactor_key: runtime::ReactorKey,
        reaction_fn: F,
        env: &'a mut EnvBuilder<S>,
    ) -> Self
    where
        F: runtime::ReactionFn<S> + 'static,
    {
        Self {
            reaction: ReactionBuilder {
                name: name.into(),
                priority,
                reactor_key,
                reaction_fn: Box::new(reaction_fn),
                trigger_ports: SecondaryMap::new(),
                schedulable_actions: SecondaryMap::new(),
                trigger_actions: SecondaryMap::new(),
                deps: SecondaryMap::new(),
                antideps: SecondaryMap::new(),
            },
            env,
        }
    }

    /// Indicate that this Reaction can be triggered by the given Action
    pub fn with_trigger_action(mut self, trigger_key: runtime::ActionKey) -> Self {
        self.reaction.trigger_actions.insert(trigger_key, ());
        self
    }

    /// Indicate that this Reaction can be triggered by the given Port
    pub fn with_trigger_port(mut self, port_key: runtime::PortKey) -> Self {
        self.reaction.trigger_ports.insert(port_key, ());
        self.reaction.deps.insert(port_key, ());
        self
    }

    /// Indicate that this Reaction may schedule the given Action
    pub fn with_scheduable_action(mut self, action_key: runtime::ActionKey) -> Self {
        self.reaction.schedulable_actions.insert(action_key, ());
        self
    }

    pub fn with_scheduable_actions(mut self, action_keys: &[runtime::ActionKey]) -> Self {
        self.reaction
            .schedulable_actions
            .extend(action_keys.iter().map(|&key| (key, ())));
        self
    }

    /// Indicate that this Reaction may set the value of the given Port.
    pub fn with_antidependency(mut self, antidep_key: runtime::PortKey) -> Self {
        self.reaction.antideps.insert(antidep_key, ());
        self
    }

    pub fn finish(self) -> Result<runtime::ReactionKey, BuilderError> {
        let Self {
            reaction: reaction_builder,
            env,
        } = self;
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
