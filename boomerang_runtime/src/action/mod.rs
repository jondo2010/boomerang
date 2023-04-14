use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{keys::ActionKey, Tag};
use downcast_rs::{impl_downcast, DowncastSync};

mod action_ref;
pub use action_ref::*;

pub trait ActionData: std::fmt::Debug + Send + Sync + Clone + 'static {}
impl<T> ActionData for T where T: std::fmt::Debug + Send + Sync + Clone + 'static {}

pub trait BaseActionValues: Debug + Send + Sync + DowncastSync {
    /// Remove any value at the given Tag
    fn remove(&mut self, tag: Tag);
}
impl_downcast!(sync BaseActionValues);

#[derive(Debug)]
pub(crate) struct ActionValues<T: ActionData>(HashMap<Tag, T>);
impl<T: ActionData> BaseActionValues for ActionValues<T> {
    fn remove(&mut self, tag: Tag) {
        self.0.remove(&tag);
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

    fn as_physical(&self) -> Option<&PhysicalAction> {
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
