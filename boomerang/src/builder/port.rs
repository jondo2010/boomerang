use crate::runtime;
use slotmap::{secondary, Key, SecondaryMap};
use std::{fmt::Debug, marker::PhantomData};

use super::{BuilderReactionKey, BuilderReactorKey};

slotmap::new_key_type! { pub struct BuilderPortKey; }

/// Input tag
#[derive(Copy, Clone, Debug)]
pub struct Input;

/// Output tag
#[derive(Copy, Clone, Debug)]
pub struct Output;

pub trait PortType2: Copy + Clone + Debug {
    const TYPE: PortType;
}

impl PortType2 for Input {
    const TYPE: PortType = PortType::Input;
}

impl PortType2 for Output {
    const TYPE: PortType = PortType::Output;
}

#[derive(Debug)]
pub struct TypedPortKey<T: runtime::PortData, Q: PortType2>(BuilderPortKey, PhantomData<(T, Q)>);

impl<T: runtime::PortData, Q: PortType2> Copy for TypedPortKey<T, Q> {}

impl<T: runtime::PortData, Q: PortType2> Clone for TypedPortKey<T, Q> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: runtime::PortData, Q: PortType2> TypedPortKey<T, Q> {
    pub fn new(port_key: BuilderPortKey) -> Self {
        Self(port_key, PhantomData)
    }
}

impl<T: runtime::PortData, Q: PortType2> From<BuilderPortKey> for TypedPortKey<T, Q> {
    fn from(value: BuilderPortKey) -> Self {
        Self(value, PhantomData)
    }
}

impl<T: runtime::PortData, Q: PortType2> From<TypedPortKey<T, Q>> for BuilderPortKey {
    fn from(builder_port_key: TypedPortKey<T, Q>) -> Self {
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
    fn get_reactor_key(&self) -> BuilderReactorKey;
    fn get_inward_binding(&self) -> Option<BuilderPortKey>;
    fn set_inward_binding(&mut self, inward_binding: Option<BuilderPortKey>);
    fn get_outward_bindings(&self) -> secondary::Keys<BuilderPortKey, ()>;
    fn add_outward_binding(&mut self, outward_binding: BuilderPortKey);
    fn get_port_type(&self) -> &PortType;
    fn get_deps(&self) -> Vec<BuilderReactionKey>;
    fn get_antideps(&self) -> secondary::Keys<BuilderReactionKey, ()>;
    /// Get the out-going Reactions that this Port triggers
    fn get_triggers(&self) -> Vec<BuilderReactionKey>;
    fn register_dependency(&mut self, reaction_key: BuilderReactionKey, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_key: BuilderReactionKey);
    /// Create a runtime Port from this PortBuilder
    fn create_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort>;
}

pub struct PortBuilder<T: runtime::PortData, Q: PortType2> {
    name: String,
    /// The key of the Reactor that owns this PortBuilder
    reactor_key: BuilderReactorKey,
    /// The type of Port to build
    port_type: PortType,
    /// Reactions that this Port depends on
    deps: SecondaryMap<BuilderReactionKey, ()>,
    /// Reactions that depend on this port
    antideps: SecondaryMap<BuilderReactionKey, ()>,
    /// Out-going Reactions that this port triggers
    triggers: SecondaryMap<BuilderReactionKey, ()>,

    inward_binding: Option<TypedPortKey<T, Q>>,
    outward_bindings: SecondaryMap<BuilderPortKey, ()>,
    //_phantom: PhantomData<T>,
}

impl<T: runtime::PortData, Q: PortType2> PortBuilder<T, Q> {
    pub fn new(name: &str, reactor_key: BuilderReactorKey) -> Self {
        Self {
            name: name.into(),
            reactor_key,
            port_type: Q::TYPE,
            deps: SecondaryMap::new(),
            antideps: SecondaryMap::new(),
            triggers: SecondaryMap::new(),
            inward_binding: None,
            outward_bindings: SecondaryMap::new(),
        }
    }
}

impl<T: runtime::PortData, Q: PortType2> BasePortBuilder for PortBuilder<T, Q> {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    fn get_inward_binding(&self) -> Option<BuilderPortKey> {
        self.inward_binding.as_ref().map(|port_key| port_key.0)
    }

    fn get_port_type(&self) -> &PortType {
        &self.port_type
    }

    fn get_deps(&self) -> Vec<BuilderReactionKey> {
        self.deps.keys().collect()
    }

    fn get_antideps(&self) -> secondary::Keys<BuilderReactionKey, ()> {
        self.antideps.keys()
    }

    fn get_triggers(&self) -> Vec<BuilderReactionKey> {
        self.triggers.keys().collect()
    }

    fn set_inward_binding(&mut self, inward_binding: Option<BuilderPortKey>) {
        self.inward_binding = inward_binding.map(|port_key| TypedPortKey(port_key, PhantomData));
    }

    fn get_outward_bindings(&self) -> secondary::Keys<BuilderPortKey, ()> {
        self.outward_bindings.keys()
    }

    fn add_outward_binding(&mut self, outward_binding: BuilderPortKey) {
        self.outward_bindings
            .insert(outward_binding.data().into(), ());
    }

    fn register_dependency(&mut self, reaction_key: BuilderReactionKey, is_trigger: bool) {
        assert!(
            self.outward_bindings.is_empty(),
            "Dependencies may no be declared on ports with an outward binding!"
        );
        self.deps.insert(reaction_key, ());
        if is_trigger {
            self.triggers.insert(reaction_key, ());
        }
    }

    fn register_antidependency(&mut self, reaction_key: BuilderReactionKey) {
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
