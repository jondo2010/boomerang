use downcast_rs::{impl_downcast, Downcast};
use std::{
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use crate::{
    data::{ParallelData, SerdeDataObj},
    ReactorData,
};

tinymap::key_type!(pub PortKey);

pub trait BasePort: Debug + Display + ParallelData + Downcast + SerdeDataObj {
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
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Port<T: ReactorData> {
    name: String,
    key: PortKey,
    value: Option<T>,
}

#[cfg(feature = "serde")]
impl<'de, T: ReactorData> serde::Deserialize<'de> for Port<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Name,
            Key,
            Value,
        }

        struct PortVisitor<T: ReactorData>(std::marker::PhantomData<T>);

        impl<'de, T: ReactorData> serde::de::Visitor<'de> for PortVisitor<T> {
            type Value = Port<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a struct with fields `name`, `key`, and `value`")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut name = None;
                let mut key = None;
                let mut value = None;
                while let Some(map_key) = map.next_key()? {
                    match map_key {
                        Field::Name => {
                            if name.is_some() {
                                return Err(serde::de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        Field::Key => {
                            if key.is_some() {
                                return Err(serde::de::Error::duplicate_field("key"));
                            }
                            key = Some(map.next_value()?);
                        }
                        Field::Value => {
                            if value.is_some() {
                                return Err(serde::de::Error::duplicate_field("value"));
                            }
                            value = Some(map.next_value()?);
                        }
                    }
                }

                let name = name.ok_or_else(|| serde::de::Error::missing_field("name"))?;
                let key = key.ok_or_else(|| serde::de::Error::missing_field("key"))?;
                let value = value.ok_or_else(|| serde::de::Error::missing_field("value"))?;

                Ok(Port { name, key, value })
            }
        }

        const FIELDS: &[&str] = &["name", "key", "value"];
        deserializer.deserialize_struct("Port", FIELDS, PortVisitor::<T>(std::marker::PhantomData))
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

impl<'a, T: ReactorData> From<&'a (dyn BasePort)> for InputRef<'a, T> {
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
