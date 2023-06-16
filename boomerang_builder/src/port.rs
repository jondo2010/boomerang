use slotmap::{secondary, Key, SecondaryMap};
use std::{fmt::Debug, marker::PhantomData, rc::Rc, time::Duration};

use boomerang_runtime as runtime;

use super::{ActionBuilderFn, BuilderReactionKey, BuilderReactorKey};

slotmap::new_key_type! {
    pub struct BuilderPortKey;
}

#[derive(Clone, Copy, Debug)]
pub struct TypedPortKey<T: runtime::PortData>(BuilderPortKey, PhantomData<T>);

impl<T: runtime::PortData> runtime::InnerType for TypedPortKey<T> {
    type Inner = T;
}

impl<T: runtime::PortData> TypedPortKey<T> {
    pub fn new(port_key: BuilderPortKey) -> Self {
        Self(port_key, PhantomData)
    }
}

impl<T: runtime::PortData> From<TypedPortKey<T>> for BuilderPortKey {
    fn from(builder_port_key: TypedPortKey<T>) -> Self {
        builder_port_key.0
    }
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum PortType {
    Input,
    Output,
}

pub trait BasePortBuilder {
    /// Get the name of this Port
    fn get_name(&self) -> &str;
    /// Get the key of the Reactor that owns this PortBuilder
    fn get_reactor_key(&self) -> BuilderReactorKey;
    /// Set the key of the Reactor that owns this PortBuilder
    fn set_reactor_key(&mut self, reactor_key: BuilderReactorKey);
    /// Get the key of the Port that this Port is bound to (inwards)
    fn get_inward_binding(&self) -> Option<BuilderPortKey>;
    /// Set the key of the Port that this Port is bound to (inwards)
    fn set_inward_binding(&mut self, inward_binding: Option<BuilderPortKey>);
    /// Clear the inward binding of this Port
    fn clear_inward_binding(&mut self);
    /// Get the keys of the Ports that this Port is bound to (outwards)
    fn get_outward_bindings(&self) -> secondary::Keys<BuilderPortKey, ()>;
    /// Add a key of the Port that this Port is bound to (outwards)
    fn add_outward_binding(&mut self, outward_binding: BuilderPortKey);
    /// Clear the outward bindings of this Port
    fn clear_outward_bindings(&mut self);
    /// Clear any bindings to the given Port
    fn clear_bindings_to(&mut self, port_key: BuilderPortKey);
    /// Get the type of this Port
    fn get_port_type(&self) -> PortType;
    /// Get the Reactions that this Port depends on
    fn get_deps(&self) -> Vec<BuilderReactionKey>;
    /// Get the Reactions that depend on this Port
    fn get_antideps(&self) -> secondary::Keys<BuilderReactionKey, ()>;
    /// Get the out-going Reactions that this Port triggers
    fn get_triggers(&self) -> Vec<BuilderReactionKey>;
    fn register_dependency(&mut self, reaction_key: BuilderReactionKey, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_key: BuilderReactionKey);
    /// Create a runtime Port from this PortBuilder
    fn create_runtime_port(&self) -> Box<dyn runtime::BasePort>;
    /// Create an ActionBuilderFn that will produce an action of the same type as this PortBuilder
    fn create_same_typed_action_builder(
        &self,
        min_delay: Option<Duration>,
    ) -> Rc<dyn ActionBuilderFn>;
}

pub struct PortBuilder<T: runtime::PortData> {
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

    inward_binding: Option<TypedPortKey<T>>,
    outward_bindings: SecondaryMap<BuilderPortKey, ()>,
}

impl<T: runtime::PortData> PortBuilder<T> {
    pub fn new(name: &str, reactor_key: BuilderReactorKey, port_type: PortType) -> Self {
        Self {
            name: name.into(),
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

impl<T: runtime::PortData> BasePortBuilder for PortBuilder<T> {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    fn set_reactor_key(&mut self, reactor_key: BuilderReactorKey) {
        self.reactor_key = reactor_key;
    }

    fn get_inward_binding(&self) -> Option<BuilderPortKey> {
        self.inward_binding.as_ref().map(|port_key| port_key.0)
    }

    fn get_port_type(&self) -> PortType {
        self.port_type
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

    fn clear_inward_binding(&mut self) {
        self.inward_binding = None;
    }

    fn get_outward_bindings(&self) -> secondary::Keys<BuilderPortKey, ()> {
        self.outward_bindings.keys()
    }

    fn add_outward_binding(&mut self, outward_binding: BuilderPortKey) {
        self.outward_bindings
            .insert(outward_binding.data().into(), ());
    }

    fn clear_outward_bindings(&mut self) {
        self.outward_bindings.clear();
    }

    fn clear_bindings_to(&mut self, port_key: BuilderPortKey) {
        self.outward_bindings.retain(|k, _| k != port_key);
        if let Some(inward) = self.inward_binding.as_ref() {
            if inward.0 == port_key {
                self.inward_binding = None;
            }
        }
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
    fn create_runtime_port(&self) -> Box<dyn runtime::BasePort> {
        Box::new(runtime::Port::<T>::new(self.name.clone()))
    }

    fn create_same_typed_action_builder(
        &self,
        min_delay: Option<Duration>,
    ) -> Rc<dyn ActionBuilderFn> {
        Rc::new(move |name: &'_ str, key: runtime::keys::ActionKey| {
            runtime::Action::Logical(runtime::LogicalAction::new::<T>(
                name,
                key,
                min_delay.unwrap_or_default(),
            ))
        })
    }
}
