use downcast_rs::{impl_downcast, DowncastSync};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::{InnerType, PortData};

tinymap::key_type!(pub PortKey);

pub trait BasePort: Debug + Display + Send + Sync + DowncastSync {
    /// Get the key for this port
    fn get_key(&self) -> PortKey;

    /// Return true if the port contains a value
    fn is_set(&self) -> bool;

    /// Reset the internal value
    fn cleanup(&mut self);

    /// Get the internal type name str
    fn type_name(&self) -> &'static str;
}
impl_downcast!(sync BasePort);

#[derive(Debug)]
pub struct Port<T: PortData> {
    name: String,
    key: PortKey,
    value: Option<T>,
}

impl<T: PortData> Display for Port<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Port<{}> \"{}\"",
            std::any::type_name::<T>(),
            self.name
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

impl<T: PortData> InnerType for Port<T> {
    type Inner = T;
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

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl<T> BasePort for Port<T>
where
    T: PortData,
{
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
