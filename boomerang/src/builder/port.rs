use crate::runtime;
use slotmap::{secondary, Key, SecondaryMap};
use std::{fmt::Debug, marker::PhantomData};

#[derive(Clone, Copy, Debug)]
pub struct BuilderPortKey<T: runtime::PortData>(runtime::PortKey, PhantomData<T>);

impl<T: runtime::PortData> runtime::InnerType for BuilderPortKey<T> {
    type Inner = T;
}

impl<T: runtime::PortData> BuilderPortKey<T> {
    pub fn new(port_key: runtime::PortKey) -> Self {
        Self(port_key, PhantomData)
    }
}

impl<T: runtime::PortData> From<BuilderPortKey<T>> for runtime::PortKey {
    fn from(builder_port_key: BuilderPortKey<T>) -> Self {
        builder_port_key.0
    }
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum PortType {
    Input,
    Output,
}

pub trait BasePortBuilder {
    fn get_name(&self) -> &str;
    fn get_reactor_key(&self) -> runtime::ReactorKey;
    fn get_inward_binding(&self) -> Option<runtime::PortKey>;
    fn set_inward_binding(&mut self, inward_binding: Option<runtime::PortKey>);
    fn get_outward_bindings(&self) -> secondary::Keys<runtime::PortKey, ()>;
    fn add_outward_binding(&mut self, outward_binding: runtime::PortKey);
    fn get_port_type(&self) -> &PortType;
    fn get_deps(&self) -> Vec<runtime::ReactionKey>;
    fn get_antideps(&self) -> secondary::Keys<runtime::ReactionKey, ()>;
    /// Get the out-going Reactions that this Port triggers
    fn get_triggers(&self) -> Vec<runtime::ReactionKey>;
    fn register_dependency(&mut self, reaction_key: runtime::ReactionKey, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_key: runtime::ReactionKey);
    /// Create a runtime Port from this PortBuilder
    fn create_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort>;
}

pub struct PortBuilder<T: runtime::PortData> {
    name: String,
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

    inward_binding: Option<BuilderPortKey<T>>,
    outward_bindings: SecondaryMap<runtime::PortKey, ()>,
    //_phantom: PhantomData<T>,
}

impl<T: runtime::PortData> PortBuilder<T> {
    pub fn new(name: &str, reactor_key: runtime::ReactorKey, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            reactor_key,
            port_type,
            deps: SecondaryMap::new(),
            antideps: SecondaryMap::new(),
            triggers: SecondaryMap::new(),
            inward_binding: None,
            outward_bindings: SecondaryMap::new(),
            //_phantom: PhantomData,
        }
    }
}

impl<T: runtime::PortData> BasePortBuilder for PortBuilder<T> {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_reactor_key(&self) -> runtime::ReactorKey {
        self.reactor_key
    }

    fn get_inward_binding(&self) -> Option<runtime::PortKey> {
        self.inward_binding.as_ref().map(|port_key| port_key.0)
    }

    fn get_port_type(&self) -> &PortType {
        &self.port_type
    }

    fn get_deps(&self) -> Vec<runtime::ReactionKey> {
        self.deps.keys().collect()
    }

    fn get_antideps(&self) -> secondary::Keys<runtime::ReactionKey, ()> {
        self.antideps.keys()
    }

    fn get_triggers(&self) -> Vec<runtime::ReactionKey> {
        self.triggers.keys().collect()
    }

    fn set_inward_binding(&mut self, inward_binding: Option<runtime::PortKey>) {
        self.inward_binding = inward_binding.map(|port_key| BuilderPortKey(port_key, PhantomData));
    }

    fn get_outward_bindings(&self) -> secondary::Keys<runtime::PortKey, ()> {
        self.outward_bindings.keys()
    }

    fn add_outward_binding(&mut self, outward_binding: runtime::PortKey) {
        self.outward_bindings
            .insert(outward_binding.data().into(), ());
    }

    fn register_dependency(&mut self, reaction_key: runtime::ReactionKey, is_trigger: bool) {
        assert!(
            self.outward_bindings.is_empty(),
            "Dependencies may no be declared on ports with an outward binding!"
        );
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
        self.antideps.insert(reaction_key, ());
    }

    /// Build the PortBuilder into a runtime Port
    fn create_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort> {
        Box::new(runtime::Port::<T>::new(self.name.clone(), key))
    }
}
