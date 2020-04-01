use super::{EnvBuilderState, PortType, ReactorTypeIndex};
use crate::runtime::{self, ActionIndex, PortIndex, ReactionIndex};

#[derive(Debug)]
pub struct ReactionProto {
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_type_idx: ReactorTypeIndex,
    pub(super) reaction_fn: runtime::ReactionFn,
    trigger_actions: Vec<ActionIndex>,
    trigger_ports: Vec<PortIndex>,
    pub(super) deps: Vec<PortIndex>,
    pub(super) antideps: Vec<PortIndex>,
}

impl Ord for ReactionProto {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.reactor_type_idx.cmp(&other.reactor_type_idx))
    }
}

impl PartialOrd for ReactionProto {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

impl Eq for ReactionProto {}

impl PartialEq for ReactionProto {
    fn eq(&self, other: &Self) -> bool {
        self.priority.eq(&other.priority) && self.reactor_type_idx.eq(&other.reactor_type_idx)
    }
}

pub(crate) struct ReactionBuilderInt {
    name: String,
    /// Unique ordering of this reaction within the reactor.
    priority: usize,
    /// The owning Reactor for this Reaction
    reactor_type_idx: ReactorTypeIndex,
    reaction_idx: ReactionIndex,
    reaction_fn: runtime::ReactionFn,
    trigger_actions: Vec<ActionIndex>,
    schedulable_actions: Vec<ActionIndex>,
    trigger_ports: Vec<PortIndex>,
    deps: Vec<PortIndex>,
    antideps: Vec<PortIndex>,
}

pub struct ReactionBuilder<'a>(ReactionBuilderInt, &'a mut EnvBuilderState);

impl<'a> ReactionBuilder<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reaction_idx: ReactionIndex,
        reactor_type_idx: ReactorTypeIndex,
        reaction_fn: runtime::ReactionFn,
        env: &'a mut EnvBuilderState,
    ) -> Self {
        Self(
            ReactionBuilderInt {
                name: name.into(),
                priority,
                reactor_type_idx,
                reaction_fn,
                reaction_idx,
                trigger_ports: Vec::new(),
                schedulable_actions: Vec::new(),
                trigger_actions: Vec::new(),
                deps: Vec::new(),
                antideps: Vec::new(),
            },
            env,
        )
    }

    pub fn with_trigger_action(mut self, trigger_idx: ActionIndex) -> Self {
        let ReactionBuilder(int, env) = &mut self;
        let action = env.actions.get_mut(trigger_idx.0).unwrap();
        assert!(
            int.reactor_type_idx == action.get_reactor_type_idx(),
            "Action triggers must belong to the same reactor as the triggered reaction"
        );
        action.triggers.insert(int.reaction_idx);
        int.trigger_actions.push(trigger_idx);
        self
    }

    pub fn with_trigger_port(mut self, port_idx: PortIndex) -> Self {
        let ReactionBuilder(int, env) = &mut self;
        let port = env.ports.get_mut(port_idx.0).unwrap();

        if port.get_port_type() == &PortType::Input {
            // assert!( this->container() == port->container(), "Input port triggers must belong to
            // the same reactor as the triggered reaction");
        } else {
            // assert!(this->container() == port->container()->container(), "Output port triggers
            // must belong to a contained reactor");
        }

        int.trigger_ports.push(port_idx);
        int.deps.push(port_idx);
        port.register_dependency(int.reaction_idx, true);
        self
    }

    pub fn with_scheduable_action(mut self, action_idx: ActionIndex) -> Self {
        let ReactionBuilder(int, env) = &mut self;
        let trig = env.actions.get_mut(action_idx.0).unwrap();
        // ASSERT(this->environment() == action->environment());
        // VALIDATE(this->container() == action->container(), "Scheduable actions must belong to the
        // same reactor as the triggered reaction");
        int.schedulable_actions.push(action_idx);
        trig.schedulers.push(int.reaction_idx);
        self
    }

    pub fn with_antidependency(mut self, antidep_idx: PortIndex) -> Self {
        let ReactionBuilder(int, env) = &mut self;
        let port = env.ports.get_mut(antidep_idx.0).unwrap();
        int.antideps.push(antidep_idx);
        port.register_antidependency(int.reaction_idx);
        self
    }

    pub fn finish(self) -> ReactionIndex {
        let ReactionBuilder(int, env) = self;
        let reaction = ReactionProto {
            name: int.name,
            priority: int.priority,
            reactor_type_idx: int.reactor_type_idx,
            reaction_fn: int.reaction_fn,
            trigger_actions: int.trigger_actions,
            trigger_ports: int.trigger_ports,
            deps: int.deps,
            antideps: int.antideps,
        };
        env.reactions.push(reaction);
        int.reaction_idx
    }
}
