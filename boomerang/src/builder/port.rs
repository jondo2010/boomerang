use crate::runtime;
use slotmap::{secondary, Key, SecondaryMap};
use std::{fmt::Debug, marker::PhantomData};

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum PortType {
    Input,
    Output,
}

pub use slotmap::DefaultKey as PortKey;

pub trait BasePortBuilder: Debug {
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
    // fn build(
    // &self,
    // transitive_triggers: SecondaryMap<runtime::ReactionKey, ()>,
    // ) -> Arc<dyn runtime::BasePort>;
}

#[derive(Debug)]
pub struct PortBuilder<T> {
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

    inward_binding: Option<runtime::PortKey>,
    outward_bindings: SecondaryMap<runtime::PortKey, ()>,

    _phantom: PhantomData<T>,
}

impl<T> PortBuilder<T> {
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
            _phantom: PhantomData,
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
        self.inward_binding.map(|key| key.data().into())
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
        self.inward_binding = inward_binding.map(|port_key| port_key.data().into());
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
