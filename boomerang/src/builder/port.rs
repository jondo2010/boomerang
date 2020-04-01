use std::{collections::BTreeSet, fmt::Debug, marker::PhantomData, sync::Arc};

use tracing::event;

use super::ReactorTypeIndex;
use crate::runtime::{self, PortIndex, PortValue, ReactionIndex};

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum PortType {
    Input,
    Output,
}

pub trait BasePortBuilder: Debug {
    fn get_name(&self) -> &str;
    fn get_reactor_type_idx(&self) -> ReactorTypeIndex;
    fn get_inward_binding(&self) -> &Option<PortIndex>;
    fn set_inward_binding(&mut self, inward_binding: Option<PortIndex>);
    fn get_outward_bindings(&self) -> &BTreeSet<PortIndex>;
    fn add_outward_binding(&mut self, outward_binding: PortIndex);
    fn get_port_type(&self) -> &PortType;
    fn get_deps(&self) -> &Vec<ReactionIndex>;
    fn get_antideps(&self) -> &Vec<ReactionIndex>;
    /// Get the out-going Reactions that this Port triggers
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex>;
    fn register_dependency(&mut self, reaction_idx: ReactionIndex, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_idx: ReactionIndex);
    fn build(&self, transitive_triggers: BTreeSet<ReactionIndex>) -> Arc<dyn runtime::BasePort>;
}

#[derive(Debug)]
pub struct PortBuilder<T>
where
    T: runtime::PortData,
{
    name: String,
    /// The index of the ReactorType that owns this PortBuilder
    reactor_type_idx: ReactorTypeIndex,
    /// The type of Port to build
    port_type: PortType,
    /// Reactions that this Port depends on
    deps: Vec<ReactionIndex>,
    /// Reactions that depend on this port
    antideps: Vec<ReactionIndex>,
    /// Out-going Reactions that this port triggers
    triggers: BTreeSet<ReactionIndex>,

    inward_binding: Option<PortIndex>,
    outward_bindings: BTreeSet<PortIndex>,

    _phantom: PhantomData<T>,
}

impl<T> PortBuilder<T>
where
    T: runtime::PortData,
{
    pub fn new(name: &str, container_idx: ReactorTypeIndex, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            reactor_type_idx: container_idx,
            port_type,
            deps: Vec::new(),
            antideps: Vec::new(),
            triggers: BTreeSet::new(),
            inward_binding: None,
            outward_bindings: BTreeSet::new(),
            _phantom: PhantomData,
        }
    }
}

impl<T> BasePortBuilder for PortBuilder<T>
where
    T: runtime::PortData,
{
    fn get_name(&self) -> &str {
        &self.name
    }
    fn get_reactor_type_idx(&self) -> ReactorTypeIndex {
        self.reactor_type_idx
    }
    fn get_inward_binding(&self) -> &Option<PortIndex> {
        &self.inward_binding
    }
    fn get_port_type(&self) -> &PortType {
        &self.port_type
    }
    fn get_deps(&self) -> &Vec<ReactionIndex> {
        &self.deps
    }
    fn get_antideps(&self) -> &Vec<ReactionIndex> {
        &self.antideps
    }
    fn get_triggers(&self) -> &BTreeSet<ReactionIndex> {
        &self.triggers
    }
    fn set_inward_binding(&mut self, inward_binding: Option<PortIndex>) {
        self.inward_binding = inward_binding;
    }
    fn get_outward_bindings(&self) -> &BTreeSet<PortIndex> {
        &self.outward_bindings
    }
    fn add_outward_binding(&mut self, outward_binding: PortIndex) {
        self.outward_bindings.insert(outward_binding);
    }
    fn register_dependency(&mut self, reaction_idx: ReactionIndex, is_trigger: bool) {
        assert!(
            self.outward_bindings.is_empty(),
            "Dependencies may no be declared on ports with an outward binding!"
        );

        if self.port_type == PortType::Input {
            //  VALIDATE(this->container() == reaction->container(), "Dependent input ports must
            // belong to the same reactor as the reaction");
        } else {
            //  VALIDATE(this->container()->container() == reaction->container(), "Dependent output
            // ports must belong to a contained reactor");
        }

        self.deps.push(reaction_idx);
        if is_trigger {
            self.triggers.insert(reaction_idx);
        }
    }

    fn register_antidependency(&mut self, reaction_idx: ReactionIndex) {
        assert!(
            self.inward_binding.is_none(),
            "Antidependencies may no be declared on ports with an inward binding!"
        );
        if self.port_type == PortType::Output {
            //  VALIDATE(this->container() == reaction->container(), "Antidependent output ports
            // must belong to the same reactor as the reaction");
        } else {
            //  VALIDATE(this->container()->container() == reaction->container(), "Antidependent
            // input ports must belong to a contained reactor");
        }
        self.antideps.push(reaction_idx);
    }

    fn build(&self, transitive_triggers: BTreeSet<ReactionIndex>) -> Arc<dyn runtime::BasePort> {
        event!(
            tracing::Level::DEBUG,
            "Building Port: {}, triggers: {:?}",
            self.name,
            self.triggers
        );

        Arc::new(runtime::Port::new(
            self.name.clone(),
            PortValue::new(Option::<T>::None),
            transitive_triggers,
        ))
    }
}
