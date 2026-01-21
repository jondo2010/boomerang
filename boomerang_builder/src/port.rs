use std::{fmt::Debug, marker::PhantomData};

use crate::{runtime, BuilderReactorKey, ParentReactorBuilder};

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

/// A port that is local to the Reactor
#[derive(Copy, Clone, Debug)]
pub struct Local;

/// A port of a contained Reactor
#[derive(Copy, Clone, Debug)]
pub struct Contained;

pub struct TypedPortKey<T: runtime::ReactorData, Q: PortTag, A = Local>(
    BuilderPortKey,
    PhantomData<(T, Q, A)>,
);

impl<T: runtime::ReactorData, Q: PortTag> TypedPortKey<T, Q, Local> {
    /// Convert this port to a contained port
    pub fn contained(self) -> TypedPortKey<T, Q, Contained> {
        TypedPortKey(self.0, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: PortTag> Debug for TypedPortKey<T, Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TypedPortKey")
            .field(&self.0)
            .field(&self.1)
            .finish()
    }
}

impl<T: runtime::ReactorData, Q: PortTag, A: Copy> Copy for TypedPortKey<T, Q, A> {}

impl<T: runtime::ReactorData, Q: PortTag, A: Clone> Clone for TypedPortKey<T, Q, A> {
    fn clone(&self) -> Self {
        Self(self.0, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: PortTag, A: Copy> TypedPortKey<T, Q, A> {
    pub fn new(port_key: BuilderPortKey) -> Self {
        Self(port_key, PhantomData)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Self> + Clone {
        std::iter::once(self)
    }
}

impl<T: runtime::ReactorData, Q: PortTag, A> From<BuilderPortKey> for TypedPortKey<T, Q, A> {
    fn from(value: BuilderPortKey) -> Self {
        Self(value, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: PortTag, A> From<TypedPortKey<T, Q, A>> for BuilderPortKey {
    fn from(builder_port_key: TypedPortKey<T, Q, A>) -> Self {
        builder_port_key.0
    }
}

impl<T: runtime::ReactorData> runtime::ReactionRefsExtract for TypedPortKey<T, Input, Local> {
    type Ref<'store>
        = runtime::InputRef<'store, T>
    where
        Self: 'store;
    fn extract<'store>(refs: &mut runtime::ReactionRefs<'store>) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let port = refs
            .ports
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("input port"))?;

        runtime::InputRef::try_from(runtime::DynPortRef(port))
    }
}

/// An input port on a contained reactor extracts as an output port in a parent
/// reaction.
impl<T: runtime::ReactorData> runtime::ReactionRefsExtract for TypedPortKey<T, Input, Contained> {
    type Ref<'store>
        = runtime::OutputRef<'store, T>
    where
        Self: 'store;
    fn extract<'store>(refs: &mut runtime::ReactionRefs<'store>) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let port = refs
            .ports_mut
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("contained input port"))?;

        runtime::OutputRef::try_from(runtime::DynPortRefMut(port))
    }
}

impl<T: runtime::ReactorData> runtime::ReactionRefsExtract for TypedPortKey<T, Output, Local> {
    type Ref<'store>
        = runtime::OutputRef<'store, T>
    where
        Self: 'store;
    fn extract<'store>(refs: &mut runtime::ReactionRefs<'store>) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let port = refs
            .ports_mut
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("output port"))?;

        runtime::OutputRef::try_from(runtime::DynPortRefMut(port))
    }
}

/// An output port on a contained reactor extracts as an input port in a parent
/// reaction.
impl<T: runtime::ReactorData> runtime::ReactionRefsExtract for TypedPortKey<T, Output, Contained> {
    type Ref<'store>
        = runtime::InputRef<'store, T>
    where
        Self: 'store;
    fn extract<'store>(refs: &mut runtime::ReactionRefs<'store>) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let port = refs
            .ports
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("contained output port"))?;

        runtime::InputRef::try_from(runtime::DynPortRef(port))
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
    fn port_type(&self) -> &PortType;
    fn bank_info(&self) -> Option<&runtime::BankInfo>;
    /// Create a runtime Port from this PortBuilder
    fn build_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort>;
    fn inner_type_id(&self) -> std::any::TypeId;
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
    _phantom: PhantomData<fn() -> (T, Q)>,
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
            _phantom: PhantomData,
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

    fn port_type(&self) -> &PortType {
        &self.port_type
    }

    fn bank_info(&self) -> Option<&runtime::BankInfo> {
        self.bank_info.as_ref()
    }

    /// Build the PortBuilder into a runtime Port
    fn build_runtime_port(&self, key: runtime::PortKey) -> Box<dyn runtime::BasePort> {
        Box::new(runtime::Port::<T>::new(&self.name, key))
    }

    fn inner_type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<(T, Q)>()
    }
}
