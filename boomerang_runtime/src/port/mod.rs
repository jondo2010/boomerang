use downcast_rs::{impl_downcast, Downcast};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::ReactorData;

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

impl<'a, T: ReactorData> InputRef<'a, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }
}

impl<'a, T: ReactorData> From<&'a Port<T>> for InputRef<'a, T> {
    fn from(port: &'a Port<T>) -> Self {
        Self(port)
    }
}

impl<'a, T: ReactorData> Deref for InputRef<'a, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
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

impl<'a, T: ReactorData> OutputRef<'a, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }
}

impl<'a, T: ReactorData> From<&'a mut Port<T>> for OutputRef<'a, T> {
    fn from(port: &'a mut Port<T>) -> Self {
        Self(port)
    }
}

impl<'a, T: ReactorData> Deref for OutputRef<'a, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T: ReactorData> DerefMut for OutputRef<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<'a, T: ReactorData> From<&'a mut dyn BasePort> for OutputRef<'a, T> {
    fn from(port: &'a mut dyn BasePort) -> Self {
        OutputRef::from(
            port.downcast_mut::<Port<T>>()
                .expect("Downcast failed during conversion"),
        )
    }
}
