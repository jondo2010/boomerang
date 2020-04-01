use super::{reaction::ReactionBuilder, reactor::ReactorBuilder, NamedBuilder};
use crate::{
    runtime::{self, PortType},
    Error,
};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt::{Debug, Display},
    rc::Rc,
    sync::{Arc, RwLock},
};
use toolshed::{list::List, Arena, CopyCell};
use tracing::event;

#[derive(Copy, Clone)]
pub struct PortBuilder<'a, T>
where
    T: runtime::PortData,
{
    name: &'a str,
    parent: &'a ReactorBuilder<'a, T>,
    port_type: PortType,
    dependencies: List<'a, &'a ReactionBuilder<'a, T>>,
    triggers: List<'a, &'a ReactionBuilder<'a, T>>,
    antidependencies: List<'a, &'a ReactionBuilder<'a, T>>,
    inward_binding: CopyCell<Option<&'a PortBuilder<'a, T>>>,
    outward_bindings: List<'a, &'a PortBuilder<'a, T>>,
}

impl<T: runtime::PortData> Ord for PortBuilder<'_, T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get_name()
            .cmp(other.get_name())
            .then_with(|| self.get_port_type().cmp(other.get_port_type()))
    }
}
impl<T: runtime::PortData> PartialOrd for PortBuilder<'_, T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.get_name()
            .partial_cmp(other.get_name())
            .and(self.get_port_type().partial_cmp(other.get_port_type()))
    }
}
impl<T: runtime::PortData> Eq for PortBuilder<'_, T> {}
impl<T: runtime::PortData> PartialEq for PortBuilder<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.get_name().eq(other.get_name()) && self.get_port_type().eq(other.get_port_type())
    }
}
impl<'a, T: runtime::PortData> Debug for PortBuilder<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasePortBuilder")
            .field("name", &self.get_name())
            .field("port_type", self.get_port_type())
            //.field("inward_binding", &self.get_inward_binding())
            //.field("antidependencies", self.get_antidependencies())
            .finish()
    }
}
impl<T: runtime::PortData> Display for PortBuilder<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "BasePortBuilder({}, {})",
            self.get_name(),
            self.get_port_type()
        ))
    }
}


impl<'a, T> PortBuilder<'a, T>
where
    T: runtime::PortData,
{
    pub fn new(
        arena: &'a Arena,
        name: &str,
        parent: &'a ReactorBuilder<'a, T>,
        port_type: PortType,
    ) -> &'a Self {
        arena.alloc(Self {
            name: arena.alloc_str(name),
            parent,
            port_type,
            dependencies: List::empty(),
            triggers: List::empty(),
            antidependencies: List::empty(),
            inward_binding: CopyCell::new(None),
            outward_bindings: List::empty(),
        })
    }
    pub fn register_dependency(
        &self,
        reaction: &'a ReactionBuilder<'a, T>,
        is_trigger: bool,
        arena: &'a Arena,
    ) {
        // VALIDATE(!this->has_outward_bindings(), "Dependencies may no be declared on ports with an
        // outward binding!"); if (this->is_input()) {
        //  VALIDATE(this->container() == reaction->container(), "Dependent input ports must belong
        // to the same reactor as the reaction");
        //} else {
        //  VALIDATE(this->container()->container() == reaction->container(), "Dependent output
        // ports must belong to a contained reactor");
        //}
        self.dependencies.prepend(arena, reaction);
        if is_trigger {
            self.triggers.prepend(arena, reaction);
        }
    }

    pub fn register_antidependency(&self, reaction: &'a ReactionBuilder<'a, T>, arena: &'a Arena) {
        // VALIDATE( !this->has_inward_binding(), "Antidependencies may no be declared on ports with
        // an inward binding!"); if (this->is_output()) {
        //  VALIDATE(this->container() == reaction->container(), "Antidependent output ports must
        // belong to the same reactor as the reaction");
        //} else {
        //  VALIDATE(this->container()->container() == reaction->container(), "Antidependent input
        // ports must belong to a contained reactor");
        //}
        self.antidependencies.prepend(arena, reaction);
    }

    pub fn bind_to(&'a self, arena: &'a Arena, port: &'a Self) {
        // TODO validation
        port.inward_binding.set(Some(self));
        self.outward_bindings.prepend(arena, port);
    }

    fn get_port_type(&self) -> &PortType {
        &self.port_type
    }
    fn get_inward_binding(&self) -> Option<&'a PortBuilder<T>> {
        self.inward_binding
            .get()
            .map(|inward| inward as &PortBuilder<T>)
    }
    pub fn follow_inward_binding(&'a self) -> &PortBuilder<T> {
        self.inward_binding
            .get()
            .map(|inward| inward.follow_inward_binding())
            .unwrap_or(self)
    }
    pub fn get_antidependencies(&'a self) -> &List<'a, &'a ReactionBuilder<T>> {
        &self.antidependencies
    }
    pub fn build<'b>(
        &'a self,
        port_map: &'b mut BTreeMap<&'a PortBuilder<'a, T>, Rc<dyn Any>>,
        reaction_map: &'b mut BTreeMap<&'a ReactionBuilder<'a, T>, Rc<runtime::Reaction>>,
    ) -> &'b dyn runtime::BasePort {
        let binding = self.follow_inward_binding();
        let any_port = port_map.entry(binding).or_insert_with(|| {
            println!("Building {}", &binding);

            let port_value: runtime::PortValue<T> = Arc::new(RwLock::new(None));

            // let x = port_value as Arc<dyn Any>;

            let port = runtime::Port::<T>::new(
                binding.get_name().to_owned(),
                *binding.get_port_type(),
                port_value,
                BTreeSet::new(),
            );

            for adp in binding.get_antidependencies().iter() {
                println!("{} AD: {}", binding, adp);
            }

            for dep in self.dependencies.iter() {
                println!("{} DEP: {}", self.get_fqn(), dep);
            }

            event!(tracing::Level::DEBUG, %port, "Creating port");
            Rc::new(port)
        });

        let port = any_port
            .downcast_ref::<runtime::Port<T>>()
            .expect("Should not fail");
        port
    }
}

impl<'a, T> NamedBuilder<'a> for PortBuilder<'a, T>
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

/*

/// A non-templated trait interface for PortBuilder
pub trait BasePortBuilder<'a>: NamedBuilder<'a> {
    fn get_port_type(&self) -> &PortType;
    fn get_inward_binding(&'a self) -> Option<&'a dyn BasePortBuilder>;
    /// Follow chained inward bindings
    fn follow_inward_binding(&'a self) -> &'a dyn BasePortBuilder;
    /// Get a Ref to the ReactionBuilders that depend on this PortBuilder
    fn get_antidependencies(&'a self) -> &List<'a, &'a ReactionBuilder<T>>;

    /// Build a runtime Port
    fn build<'b>(
        &'a self,
        port_list: &'b mut BTreeMap<&'a dyn BasePortBuilder<'a>, Rc<dyn Any>>,
        reaction_map: &'b mut BTreeMap<&'a ReactionBuilder<'a, T>, Rc<runtime::Reaction>>,
    ) -> &'b dyn runtime::BasePort;
}


impl<'a, T> From<&'a PortBuilder<'a, T>> for &'a dyn BasePortBuilder<'a>
where
    T: runtime::PortData,
{
    fn from(port: &'a PortBuilder<'a, T>) -> Self {
        port
    }
}

impl<'a, T> BasePortBuilder<'a> for PortBuilder<'a, T>
where
    T: runtime::PortData,
{
    
}

*/