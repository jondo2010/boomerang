use crate::{runtime, ParentReactorBuilder};
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

pub trait PortTag: Copy + Clone + Debug + 'static {
    const TYPE: PortType;
}

impl PortTag for Input {
    const TYPE: PortType = PortType::Input;
}

impl PortTag for Output {
    const TYPE: PortType = PortType::Output;
}

pub struct TypedPortKey<T: runtime::ReactorData, Q: PortTag>(BuilderPortKey, PhantomData<(T, Q)>);

impl<T: runtime::ReactorData, Q: PortTag> Debug for TypedPortKey<T, Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TypedPortKey")
            .field(&self.0)
            .field(&self.1)
            .finish()
    }
}

impl<T: runtime::ReactorData, Q: PortTag> Copy for TypedPortKey<T, Q> {}

impl<T: runtime::ReactorData, Q: PortTag> Clone for TypedPortKey<T, Q> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: runtime::ReactorData, Q: PortTag> TypedPortKey<T, Q> {
    pub fn new(port_key: BuilderPortKey) -> Self {
        Self(port_key, PhantomData)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Self> + Clone {
        std::iter::once(self)
    }
}

impl<T: runtime::ReactorData, Q: PortTag> From<BuilderPortKey> for TypedPortKey<T, Q> {
    fn from(value: BuilderPortKey) -> Self {
        Self(value, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: PortTag> From<TypedPortKey<T, Q>> for BuilderPortKey {
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
    fn name(&self) -> &str;
    fn get_reactor_key(&self) -> BuilderReactorKey;
    fn get_inward_binding(&self) -> Option<BuilderPortKey>;
    fn set_inward_binding(&mut self, inward_binding: Option<BuilderPortKey>);
    fn get_outward_bindings(&self) -> secondary::Keys<BuilderPortKey, ()>;
    fn add_outward_binding(&mut self, outward_binding: BuilderPortKey);
    fn port_type(&self) -> &PortType;
    fn bank_info(&self) -> Option<&runtime::BankInfo>;
    fn deps(&self) -> Vec<BuilderReactionKey>;
    fn antideps(&self) -> secondary::Keys<BuilderReactionKey, ()>;
    /// Get the out-going Reactions that this Port triggers
    fn triggers(&self) -> Vec<BuilderReactionKey>;
    fn register_dependency(&mut self, reaction_key: BuilderReactionKey, is_trigger: bool);
    fn register_antidependency(&mut self, reaction_key: BuilderReactionKey);
    /// Create a runtime Port from this PortBuilder
    fn build_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort>;
}

impl ParentReactorBuilder for Box<dyn BasePortBuilder> {
    fn parent_reactor_key(&self) -> Option<BuilderReactorKey> {
        Some(self.get_reactor_key())
    }
}

pub struct PortBuilder<T: runtime::ReactorData, Q: PortTag> {
    name: String,
    /// The key of the Reactor that owns this PortBuilder
    reactor_key: BuilderReactorKey,
    /// The type of Port to build
    port_type: PortType,
    /// Optional BankInfo for this Port
    bank_info: Option<runtime::BankInfo>,
    /// Reactions that this Port depends on
    deps: SecondaryMap<BuilderReactionKey, ()>,
    /// Reactions that depend on this port
    antideps: SecondaryMap<BuilderReactionKey, ()>,
    /// Out-going Reactions that this port triggers
    triggers: SecondaryMap<BuilderReactionKey, ()>,

    inward_binding: Option<TypedPortKey<T, Q>>,
    outward_bindings: SecondaryMap<BuilderPortKey, ()>,
}

impl<T: runtime::ReactorData, Q: PortTag> PortBuilder<T, Q> {
    pub fn new(
        name: &str,
        reactor_key: BuilderReactorKey,
        bank_info: Option<runtime::BankInfo>,
    ) -> Self {
        Self {
            name: name.into(),
            reactor_key,
            port_type: Q::TYPE,
            bank_info,
            deps: SecondaryMap::new(),
            antideps: SecondaryMap::new(),
            triggers: SecondaryMap::new(),
            inward_binding: None,
            outward_bindings: SecondaryMap::new(),
        }
    }
}

impl<T: runtime::ReactorData, Q: PortTag> BasePortBuilder for PortBuilder<T, Q> {
    fn name(&self) -> &str {
        &self.name
    }

    fn get_reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    fn get_inward_binding(&self) -> Option<BuilderPortKey> {
        self.inward_binding.as_ref().map(|port_key| port_key.0)
    }

    fn port_type(&self) -> &PortType {
        &self.port_type
    }

    fn bank_info(&self) -> Option<&runtime::BankInfo> {
        self.bank_info.as_ref()
    }

    fn deps(&self) -> Vec<BuilderReactionKey> {
        self.deps.keys().collect()
    }

    fn antideps(&self) -> secondary::Keys<BuilderReactionKey, ()> {
        self.antideps.keys()
    }

    fn triggers(&self) -> Vec<BuilderReactionKey> {
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
    fn build_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort> {
        Box::new(runtime::Port::<T>::new(&self.name, key))
    }
}
