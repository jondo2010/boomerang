use downcast_rs::{impl_downcast, Downcast};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::PortData;

tinymap::key_type!(pub PortKey);

pub trait BasePort: Debug + Display + Send + Sync + Downcast {
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

#[derive(Debug)]
pub struct Port<T: PortData> {
    name: String,
    key: PortKey,
    value: Option<T>,
}

impl<T: PortData> Display for Port<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{} : Port<{}>",
            self.name,
            std::any::type_name::<T>()
        ))
    }
}

impl<T: PortData> Deref for Port<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: PortData> DerefMut for Port<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T> Port<T>
where
    T: PortData,
{
    pub fn new(name: String, key: PortKey) -> Self {
        Self {
            name,
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
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
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

pub struct InputRef<'a, T: PortData = ()>(&'a Port<T>);

impl<'a, T: PortData> InputRef<'a, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }
}

impl<'a, T: PortData> From<&'a Port<T>> for InputRef<'a, T> {
    fn from(port: &'a Port<T>) -> Self {
        Self(port)
    }
}

impl<'a, T: PortData> Deref for InputRef<'a, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub struct OutputRef<'a, T: PortData = ()>(&'a mut Port<T>);

impl<'a, T: PortData> OutputRef<'a, T> {
    pub fn name(&self) -> &str {
        self.0.get_name()
    }

    pub fn key(&self) -> PortKey {
        self.0.get_key()
    }
}

impl<'a, T: PortData> From<&'a mut Port<T>> for OutputRef<'a, T> {
    fn from(port: &'a mut Port<T>) -> Self {
        Self(port)
    }
}

impl<'a, T: PortData> Deref for OutputRef<'a, T> {
    type Target = <Port<T> as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T: PortData> DerefMut for OutputRef<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}
