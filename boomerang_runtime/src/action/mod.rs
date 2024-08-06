use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
};

use crate::{Duration, Tag};
use downcast_rs::{impl_downcast, DowncastSync};

mod action_ref;
pub use action_ref::*;

#[cfg(not(feature = "serde"))]
pub trait ActionData: std::fmt::Debug + Send + Sync + Clone + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> ActionData for T where T: std::fmt::Debug + Send + Sync + Clone + 'static {}

#[cfg(feature = "serde")]
pub trait ActionData:
    std::fmt::Debug
    + Send
    + Sync
    + Clone
    + erased_serde::Serialize
    + for<'de> serde::Deserialize<'de>
    + 'static
{
}

#[cfg(feature = "serde")]
impl<T> ActionData for T where
    T: std::fmt::Debug
        + Send
        + Sync
        + Clone
        + erased_serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + 'static
{
}

tinymap::key_type! { pub ActionKey }

#[cfg(feature = "serde")]
impl serde::Serialize for ActionKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ActionKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self(serde::Deserialize::deserialize(deserializer)?))
    }
}

pub trait BaseActionValues: Debug + Send + Sync + DowncastSync {
    /// Remove any value at the given Tag
    fn remove(&mut self, tag: Tag);

    /// Get the value at the given Tag as a serializable value
    #[cfg(feature = "serde")]
    fn get_serializable_value(&self, tag: Tag) -> Option<&dyn erased_serde::Serialize>;

    /// Try to pull a value from the deserializer and store it at the given Tag
    #[cfg(feature = "serde")]
    fn deserialize(
        &mut self,
        tag: Tag,
        des: &mut dyn erased_serde::Deserializer<'_>,
    ) -> Result<(), erased_serde::Error>;
}
impl_downcast!(sync BaseActionValues);

#[derive(Debug)]
pub(crate) struct ActionValues<T: ActionData>(HashMap<Tag, T>);
impl<T: ActionData> BaseActionValues for ActionValues<T> {
    fn remove(&mut self, tag: Tag) {
        self.0.remove(&tag);
    }

    #[cfg(feature = "serde")]
    fn get_serializable_value(&self, tag: Tag) -> Option<&dyn erased_serde::Serialize> {
        self.0.get(&tag).map(|v| v as &dyn erased_serde::Serialize)
    }

    #[cfg(feature = "serde")]
    fn deserialize(
        &mut self,
        tag: Tag,
        des: &mut dyn erased_serde::Deserializer<'_>,
    ) -> Result<(), erased_serde::Error> {
        let value = T::deserialize(des)?;
        self.set_value(Some(value), tag);
        Ok(())
    }
}

impl<T: ActionData> ActionValues<T> {
    pub fn get_value(&self, tag: Tag) -> Option<&T> {
        self.0.get(&tag)
    }

    pub fn set_value(&mut self, value: Option<T>, new_tag: Tag) {
        match (self.0.entry(new_tag), value) {
            // Replace the previous value with a new one
            (Entry::Occupied(mut entry), Some(value)) => {
                entry.insert(value);
            }
            // Remove a previous value
            (Entry::Occupied(entry), None) => {
                entry.remove();
            }
            // Insert a new value
            (Entry::Vacant(entry), Some(value)) => {
                entry.insert(value);
            }
            _ => {}
        }
    }
}

/// Typed Action state, storing potentially different values at different Tags.
#[derive(Debug)]
pub struct LogicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub values: Box<dyn BaseActionValues>,
}

impl LogicalAction {
    pub fn new<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        Self {
            name: name.into(),
            key,
            min_delay,
            values: Box::new(ActionValues::<T>(HashMap::new())),
        }
    }
}

#[derive(Debug)]
pub struct PhysicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub values: Arc<Mutex<dyn BaseActionValues>>,
}

impl PhysicalAction {
    pub fn new<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        Self {
            name: name.into(),
            key,
            min_delay,
            values: Arc::new(Mutex::new(ActionValues::<T>(HashMap::new()))),
        }
    }
}

#[derive(Debug)]
pub enum Action {
    /// Startup is a special action that fires when the scheduler starts up.
    Startup,
    /// Shutdown is a special action that fires when the scheduler shuts down.
    Shutdown,
    Logical(LogicalAction),
    Physical(PhysicalAction),
}

impl Action {
    pub fn as_valued(&self) -> Option<&LogicalAction> {
        if let Self::Logical(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_valued_mut(&mut self) -> Option<&mut LogicalAction> {
        if let Self::Logical(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_physical(&self) -> Option<&PhysicalAction> {
        if let Self::Physical(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Startup => f.write_fmt(format_args!("Action::Startup")),
            Action::Shutdown => f.write_fmt(format_args!("Action::Shutdown")),
            Action::Logical(LogicalAction { name, .. }) => {
                f.write_fmt(format_args!("Action::Logical<{name}>"))
            }
            Action::Physical(PhysicalAction { name, .. }) => {
                f.write_fmt(format_args!("Action::Physical<{name}>"))
            }
        }
    }
}

impl AsRef<Action> for Action {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<Action> for Action {
    fn as_mut(&mut self) -> &mut Action {
        self
    }
}
