use downcast_rs::{impl_downcast, Downcast};
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    refs::{RefsSlice, RefsSliceMut},
    refs_extract::ReactionRefsError,
    ReactorData,
};

tinymap::key_type! { pub PortKey }

pub trait BasePort: Debug + Display + Downcast + Send + Sync {
    /// Get the name of this port
    fn get_name(&self) -> &str;

    /// Get the key for this port
    fn get_key(&self) -> PortKey;

    /// Return true if the port contains a value
    fn is_set(&self) -> bool;

    /// Reset the internal value
    fn cleanup(&mut self);

    /// Get the internal type name str
    fn type_name(&self) -> &'static str;
}
impl_downcast!(BasePort);

/// Wrapper for dynamic immutable port references to support fallible conversions.
pub struct DynPortRef<'a>(pub &'a dyn BasePort);

/// Wrapper for dynamic mutable port references to support fallible conversions.
pub struct DynPortRefMut<'a>(pub &'a mut dyn BasePort);

pub struct Port<T: ReactorData> {
    name: String,
    key: PortKey,
    value: Option<T>,
}

impl<T: ReactorData> Debug for Port<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Port")
            .field("name", &self.name)
            .field("key", &self.key)
            //.field("value", &self.value)
            .finish()
    }
}

impl<T: ReactorData> Display for Port<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Port<{ty}>(\"{name}\", {key})",
            ty = std::any::type_name::<T>(),
            name = &self.name,
            key = self.key
        )
    }
}

impl<T: ReactorData> Deref for Port<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: ReactorData> DerefMut for Port<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: ReactorData> Port<T> {
    pub fn new(name: &str, key: PortKey) -> Self {
        Self {
            name: name.to_owned(),
            key,
            value: None,
        }
    }

    pub fn get(&self) -> &Option<T> {
        &self.value
    }

    pub fn get_mut(&mut self) -> &mut Option<T> {
        &mut self.value
    }

    pub fn boxed(self) -> Box<dyn BasePort> {
        Box::new(self)
    }
}

impl<T: ReactorData> BasePort for Port<T> {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_key(&self) -> PortKey {
        self.key
    }

    fn is_set(&self) -> bool {
        self.value.is_some()
    }

    fn cleanup(&mut self) {
        self.value = None;
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

/// A reference to an input port.
///
/// `InputRef` is the type that Reaction functions receive for their input ports.
///
/// See also: [`OutputRef`]
pub struct InputRef<'a, T: ReactorData = ()>(&'a Port<T>);

impl<T: ReactorData> InputRef<'_, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }

    /// # Safety
    ///
    /// The caller must ensure `port` is a `Port<T>`.
    pub unsafe fn from_unchecked(port: &dyn BasePort) -> Self {
        let ptr = port as *const dyn BasePort as *const Port<T>;
        InputRef(&*ptr)
    }
}

impl<'a, T: ReactorData> From<&'a Port<T>> for InputRef<'a, T> {
    fn from(port: &'a Port<T>) -> Self {
        Self(port)
    }
}

impl<T: ReactorData> Deref for InputRef<'_, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T: ReactorData> TryFrom<DynPortRef<'a>> for InputRef<'a, T> {
    type Error = ReactionRefsError;

    fn try_from(port: DynPortRef<'a>) -> Result<Self, Self::Error> {
        let found = port.0.type_name();

        port.0
            .downcast_ref::<Port<T>>()
            .map(InputRef::from)
            .ok_or_else(|| ReactionRefsError::type_mismatch("input port", std::any::type_name::<T>(), found))
    }
}

impl<'a, T: ReactorData> From<&'a dyn BasePort> for InputRef<'a, T> {
    fn from(port: &'a dyn BasePort) -> Self {
        InputRef::from(
            port.downcast_ref::<Port<T>>()
                .expect("Downcast failed during conversion"),
        )
    }
}

/// A reference to an output port.
///
/// `OutputRef` is the type that Reaction functions receive for their input ports.
///
/// See also: [`InputRef`]
pub struct OutputRef<'a, T: ReactorData = ()>(&'a mut Port<T>);

impl<T: ReactorData> OutputRef<'_, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }

    /// # Safety
    ///
    /// The caller must ensure `port` is a `Port<T>`.
    pub unsafe fn from_unchecked(port: &mut dyn BasePort) -> Self {
        let ptr = port as *mut dyn BasePort as *mut Port<T>;
        OutputRef(&mut *ptr)
    }
}

impl<'a, T: ReactorData> From<&'a mut Port<T>> for OutputRef<'a, T> {
    fn from(port: &'a mut Port<T>) -> Self {
        Self(port)
    }
}

impl<T: ReactorData> Deref for OutputRef<'_, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: ReactorData> DerefMut for OutputRef<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<'a, T: ReactorData> TryFrom<DynPortRefMut<'a>> for OutputRef<'a, T> {
    type Error = ReactionRefsError;

    fn try_from(port: DynPortRefMut<'a>) -> Result<Self, Self::Error> {
        let found = port.0.type_name();

        port
            .0
            .downcast_mut::<Port<T>>()
            .map(OutputRef::from)
            .ok_or_else(|| ReactionRefsError::type_mismatch("output port", std::any::type_name::<T>(), found))
    }
}

/// A reference to a bank of input ports.
pub struct InputBankRef<'a, T: ReactorData = ()> {
    ports: RefsSlice<'a, dyn BasePort>,
    _marker: PhantomData<T>,
}

impl<'a, T: ReactorData> InputBankRef<'a, T> {
    /// The caller must ensure `ports` only contains `Port<T>` instances.
    /// Port banks created via `PortBank::extract` pre-validate this invariant.
    pub fn from_slice(ports: RefsSlice<'a, dyn BasePort>) -> Self {
        Self {
            ports,
            _marker: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.ports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = InputRef<'a, T>> + '_ {
        // Safety: `PortBank::extract` validates the bank types before constructing this view.
        self.ports
            .iter()
            .map(|port| unsafe { InputRef::from_unchecked(port) })
    }

    pub fn get(&self, idx: usize) -> Option<InputRef<'a, T>> {
        // Safety: `PortBank::extract` validates the bank types before constructing this view.
        self.ports
            .get(idx)
            .map(|port| unsafe { InputRef::from_unchecked(port) })
    }
}

/// A reference to a bank of output ports.
pub struct OutputBankRef<'a, T: ReactorData = ()> {
    ports: RefsSliceMut<'a, dyn BasePort>,
    _marker: PhantomData<T>,
}

impl<'a, T: ReactorData> OutputBankRef<'a, T> {
    /// The caller must ensure `ports` only contains `Port<T>` instances.
    /// Port banks created via `PortBank::extract` pre-validate this invariant.
    pub fn from_slice(ports: RefsSliceMut<'a, dyn BasePort>) -> Self {
        Self {
            ports,
            _marker: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.ports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }

    pub fn iter(&mut self) -> impl Iterator<Item = OutputRef<'a, T>> + '_ {
        // Safety: `PortBank::extract` validates the bank types before constructing this view.
        self.ports
            .iter_mut()
            .map(|port| unsafe { OutputRef::from_unchecked(port) })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = OutputRef<'a, T>> + '_ {
        self.iter()
    }

    pub fn get(&mut self, idx: usize) -> Option<OutputRef<'a, T>> {
        let port = self.ports.get_mut(idx)?;
        // Safety: `PortBank::extract` validates the bank types before constructing this view.
        Some(unsafe { OutputRef::from_unchecked(port) })
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<OutputRef<'a, T>> {
        self.get(idx)
    }
}
