use super::{port::PortBuilder, reactor::ReactorBuilder, NamedBuilder};
use crate::runtime;
use derive_more::Display;
use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    hash::Hash,
    sync::{Arc, RwLock},
};
use toolshed::{list::List, Arena};

/// Map from &PortBuilder -> PortValue used during building
pub type BuilderPortValueMap<'a, T> = BTreeMap<&'a PortBuilder<'a, T>, runtime::PortValue<T>>;

pub struct BuilderStateHelper<'a, T>
where
    T: runtime::PortData,
{
    port_value_map: BuilderPortValueMap<'a, T>,
}

impl<'a, T> BuilderStateHelper<'a, T>
where
    T: runtime::PortData,
{
    pub fn get_port(&'a self, port_builder: &'a PortBuilder<'a, T>) -> runtime::PortValue<T> {
        self.port_value_map.get(port_builder).unwrap().clone()
    }
}

/// Callback function used to build the Reaction body
pub type ReactionBodyBuilderFn<T> = dyn Fn(&BuilderStateHelper<T>) -> runtime::ReactionFn;

#[derive(Display, Copy, Clone)]
#[display(fmt = "ReactionBuilder({})", "self.get_fqn()")]
pub struct ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    name: &'a str,
    // The Reactor owning this Reaction
    parent: &'a ReactorBuilder<'a, T>,
    /// Relative priority of the Reaction within a Reactor
    priority: usize,
    /// A callback used to build the actual ReactionFn body
    body_builder: &'a ReactionBodyBuilderFn<T>,
    // action_triggers: List<'a, &'a ActionBuilder<'a>>,
    port_triggers: List<'a, &'a PortBuilder<'a, T>>,
    // schedulable_actions: indextree::NodeId,
    dependencies: List<'a, &'a PortBuilder<'a, T>>,
    antidependencies: List<'a, &'a PortBuilder<'a, T>>,
}

impl<'a, T> Debug for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactionBuilder")
            .field("name", &self.name)
            .field("parent", &self.parent)
            .field("priority", &self.priority)
            .field("body_builder", &"dyn Fn()")
            .field("port_triggers", &self.port_triggers)
            .field("dependencies", &self.dependencies)
            .field("antidependencies", &self.antidependencies)
            .finish()
    }
}

impl<'a, T> Hash for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_fqn().hash(state);
        self.priority.hash(state);
    }
}

impl<'a, T> PartialEq for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn eq(&self, other: &Self) -> bool {
        self.name.eq(other.name)
            && self.parent.eq(&other.parent)
            && self.priority.eq(&other.priority)
    }
}

impl<'a, T> Eq for ReactionBuilder<'a, T> where T: runtime::PortData {}

impl<'a, T> PartialOrd for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.priority.partial_cmp(&other.priority)
    }
}

impl<'a, T> Ord for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl<'a, T> ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    pub fn new(
        arena: &'a Arena,
        name: &str,
        parent: &'a ReactorBuilder<'a, T>,
        priority: usize,
        body_builder: &'a ReactionBodyBuilderFn<T>,
    ) -> &'a Self {
        arena.alloc(Self {
            name: arena.alloc_str(name),
            parent,
            priority,
            body_builder,
            port_triggers: List::empty(),
            dependencies: List::empty(),
            antidependencies: List::empty(),
        })
    }

    /// Get the list of Ports this Reaction depends on
    pub fn get_dependencies(&self) -> &List<'a, &'a PortBuilder<'a, T>> {
        &self.dependencies
    }

    // pub fn declare_trigger_action(&self, action: &'a ActionBuilder<'a>) {
    // "Action triggers must belong to the same reactor as the triggered reaction"
    // self.action_triggers.borrow_mut().insert(action);
    // }

    pub fn declare_trigger_port(&'a self, arena: &'a Arena, port: &'a PortBuilder<'a, T>) {
        // if (port->is_input()) {
        //    VALIDATE( this->container() == port->container(), "Input port triggers must belong to
        // the same reactor as the triggered reaction");
        //} else {
        //    VALIDATE(this->container() == port->container()->container(), "Output port triggers
        // must belong to a contained reactor");
        //}
        self.port_triggers.prepend(arena, port);
        self.dependencies.prepend(arena, port);
        port.register_dependency(self, true, arena);
    }

    // pub fn declare_schedulable_action(&self, action: &ActionBuilder) {
    // "Scheduable actions must belong to the same reactor as the triggered reaction"
    // schedulable_actions.append(action.node_id, &mut env.arena);
    // action.register_scheduler(env, self)?;
    // }

    pub fn declare_dependency(&'a self, arena: &'a Arena, port: &'a PortBuilder<'a, T>) {
        // if (port->is_input()) {
        //    VALIDATE(this->container() == port->container(), "Dependent input ports must belong to
        // the same reactor as the reaction");
        //} else {
        //    VALIDATE(this->container() == port->container()->container(), "Dependent output ports
        // must belong to a contained reactor");
        //}
        self.dependencies.prepend(arena, port);
        port.register_dependency(self, false, arena);
    }

    pub fn declare_antidependency(&'a self, arena: &'a Arena, port: &'a PortBuilder<'a, T>) {
        // if (port->is_output()) {
        //  VALIDATE(this->container() == port->container(), "Antidependent output ports must belong
        // to the same reactor as the reaction");
        //} else {
        //  VALIDATE(this->container() == port->container()->container(), "Antidependent input ports
        // must belong to a contained reactor");
        //}
        self.antidependencies.prepend(arena, port);
        port.register_antidependency(self, arena);
    }
}

impl<'a, T> NamedBuilder<'a> for ReactionBuilder<'a, T>
where
    T: runtime::PortData,
{
    fn get_name(&self) -> &str {
        self.name
    }
    fn get_fqn(&self) -> String {
        format!("{}.{}", self.parent.get_fqn(), self.name)
    }
}
