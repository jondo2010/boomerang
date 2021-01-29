use crate::runtime::{self};
use slotmap::{Key, SecondaryMap};
use std::{fmt::Debug, sync::Arc};
use tracing::event;

use super::EnvBuilder;

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum PortType {
    Input,
    Output,
}

pub trait BasePortBuilder: Debug {
    fn get_name(&self) -> &str;
    fn get_port_key(&self) -> runtime::BasePortKey;
    fn get_reactor_key(&self) -> runtime::ReactorKey;
    fn get_inward_binding(&self) -> Option<runtime::BasePortKey>;
    fn set_inward_binding(&mut self, inward_binding: Option<runtime::BasePortKey>);
    fn get_outward_bindings(&self) -> &Vec<runtime::BasePortKey>;
    fn add_outward_binding(&mut self, outward_binding: runtime::BasePortKey);
    fn get_port_type(&self) -> &PortType;
    fn get_deps(&self) -> Vec<runtime::ReactionKey>;
    fn get_antideps(&self) -> Vec<runtime::ReactionKey>;
    /// Get the out-going Reactions that this Port triggers
    fn get_triggers(&self) -> Vec<runtime::ReactionKey>;
    fn register_dependency(&mut self, reaction_key: runtime::ReactionKey, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_key: runtime::ReactionKey);
    // fn build(
    // &self,
    // transitive_triggers: SecondaryMap<runtime::ReactionKey, ()>,
    // ) -> Arc<dyn runtime::BasePort>;
}

#[derive(Debug)]
pub struct PortBuilder<T>
where
    T: runtime::PortData,
{
    name: String,
    /// The key of the runtime Port
    port_key: runtime::PortKey<T>,
    /// The key of the Reactor that owns this PortBuilder
    reactor_key: runtime::ReactorKey,
    /// The type of Port to build
    port_type: PortType,
    /// Reactions that this Port depends on
    deps: SecondaryMap<runtime::ReactionKey, ()>,
    /// Reactions that depend on this port
    antideps: SecondaryMap<runtime::ReactionKey, ()>,
    /// Out-going Reactions that this port triggers
    triggers: SecondaryMap<runtime::ReactionKey, ()>,

    inward_binding: Option<runtime::PortKey<T>>,
    outward_bindings: SecondaryMap<runtime::PortKey<T>, ()>,
}

impl<T> PortBuilder<T>
where
    T: runtime::PortData,
{
    pub fn new(
        name: &str,
        port_key: runtime::PortKey<T>,
        reactor_key: runtime::ReactorKey,
        port_type: PortType,
    ) -> Self {
        Self {
            name: name.into(),
            port_key,
            reactor_key,
            port_type,
            deps: SecondaryMap::new(),
            antideps: SecondaryMap::new(),
            triggers: SecondaryMap::new(),
            inward_binding: None,
            outward_bindings: SecondaryMap::new(),
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
    fn get_port_key(&self) -> runtime::BasePortKey {
        self.port_key.data().into()
    }
    fn get_reactor_key(&self) -> runtime::ReactorKey {
        self.reactor_key
    }
    fn get_inward_binding(&self) -> Option<runtime::BasePortKey> {
        self.inward_binding.map(|key| key.data().into())
    }
    fn get_port_type(&self) -> &PortType {
        &self.port_type
    }
    fn get_deps(&self) -> Vec<runtime::ReactionKey> {
        self.deps.keys().collect()
    }
    fn get_antideps(&self) -> Vec<runtime::ReactionKey> {
        self.antideps.keys().collect()
    }
    fn get_triggers(&self) -> Vec<runtime::ReactionKey> {
        self.triggers.keys().collect()
    }
    fn set_inward_binding(&mut self, inward_binding: Option<runtime::BasePortKey>) {
        self.inward_binding = inward_binding.map(|key| key.data().into());
    }
    fn get_outward_bindings(&self) -> &Vec<runtime::BasePortKey> {
        &self
            .outward_bindings
            .keys()
            .map(|key| key.data().into())
            .collect()
    }
    fn add_outward_binding(&mut self, outward_binding: runtime::BasePortKey) {
        self.outward_bindings
            .insert(outward_binding.data().into(), ());
    }
    fn register_dependency(&mut self, reaction_key: runtime::ReactionKey, is_trigger: bool) {
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

        self.deps.insert(reaction_key, ());
        if is_trigger {
            self.triggers.insert(reaction_key, ());
        }
    }

    fn register_antidependency(&mut self, reaction_key: runtime::ReactionKey) {
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
        self.antideps.insert(reaction_key, ());
    }

    // fn build(
    // &self,
    // transitive_triggers: SecondaryMap<runtime::ReactionKey, ()>,
    // env: &mut EnvBuilder,
    // ) {
    // event!(
    // tracing::Level::DEBUG,
    // "Building Port: {}, triggers: {:?}",
    // self.name,
    // self.triggers
    // );
    //
    // env.ports[self.get_port_key()].
    // }
}