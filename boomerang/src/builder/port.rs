use crate::runtime;
use slotmap::{secondary, Key, SecondaryMap};
use std::{fmt::Debug, marker::PhantomData};

use super::{BuilderError, ReactorBuilderState, ReactorPart};

#[derive(Clone, Copy, Debug)]
pub struct BuilderInputPort<T: runtime::PortData>(runtime::PortKey, PhantomData<T>);

impl<T: runtime::PortData> From<BuilderInputPort<T>> for runtime::PortKey {
    fn from(builder_port: BuilderInputPort<T>) -> Self {
        builder_port.0
    }
}

impl<T: runtime::PortData> ReactorPart for BuilderInputPort<T> {
    type Args = ();
    fn build_part<S: runtime::ReactorState>(
        builder: &mut ReactorBuilderState<S>,
        name: &str,
        _args: Self::Args,
    ) -> Result<Self, BuilderError> {
        builder
            .add_port::<T>(name, PortType::Input)
            .map(|port_key| Self(port_key, PhantomData))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuilderOutputPort<T: runtime::PortData>(runtime::PortKey, PhantomData<T>);

impl<T: runtime::PortData> From<BuilderOutputPort<T>> for runtime::PortKey {
    fn from(builder_port: BuilderOutputPort<T>) -> Self {
        builder_port.0
    }
}

impl<T: runtime::PortData> ReactorPart for BuilderOutputPort<T> {
    type Args = ();
    fn build_part<S: runtime::ReactorState>(
        builder: &mut ReactorBuilderState<S>,
        name: &str,
        _args: Self::Args,
    ) -> Result<Self, BuilderError> {
        builder
            .add_port::<T>(name, PortType::Output)
            .map(|port_key| Self(port_key, PhantomData))
    }
}

impl<T: runtime::PortData> runtime::InnerType for BuilderInputPort<T> {
    type Inner = T;
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

    fn into_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort>;
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

    inward_binding: Option<BuilderInputPort<T>>,
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
        self.inward_binding
            .as_ref()
            .map(|port_key| port_key.0.into())
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
        self.inward_binding =
            inward_binding.map(|port_key| BuilderInputPort(port_key, PhantomData));
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

    /// Build the PortBuilder into a runtime Port
    fn into_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort> {
        Box::new(runtime::Port::<T>::new(self.name.clone(), key))
    }
}
