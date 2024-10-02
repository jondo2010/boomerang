use std::{
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::Tag;

mod action_ref;
mod store;

pub use action_ref::*;
use store::{ActionStore, BaseActionStore};

#[cfg(not(feature = "serde"))]
pub trait ActionData: std::fmt::Debug + Send + Sync + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> ActionData for T where T: std::fmt::Debug + Send + Sync + 'static {}

#[cfg(feature = "serde")]
pub trait ActionData:
    std::fmt::Debug + Send + Sync + serde::Serialize + for<'de> serde::Deserialize<'de> + 'static
{
}

#[cfg(feature = "serde")]
impl<T> ActionData for T where
    T: std::fmt::Debug
        + Send
        + Sync
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + 'static
{
}

tinymap::key_type! {
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub ActionKey
}

/// Typed Action state, storing potentially different values at different Tags.
#[derive(Debug)]
pub struct LogicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub store: Box<dyn BaseActionStore>,
}

impl LogicalAction {
    pub fn new<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        let store = ActionStore::<T>::new();
        Self {
            name: name.into(),
            key,
            min_delay,
            store: Box::new(store),
        }
    }
}

#[derive(Debug)]
pub struct PhysicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub store: Arc<Mutex<dyn BaseActionStore>>,
}

impl PhysicalAction {
    pub fn new<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        let store = ActionStore::<T>::new();
        let store: Arc<Mutex<dyn BaseActionStore>> = Arc::new(Mutex::new(store));
        Self {
            name: name.into(),
            key,
            min_delay,
            store,
        }
    }

    /// Create a new Arrow ArrayBuilder for the data stored in this store
    #[cfg(feature = "serde")]
    pub fn new_builder(&self) -> Result<serde_arrow::ArrayBuilder, crate::RuntimeError> {
        self.store.lock().expect("lock").new_builder()
    }

    /// Serialize the latest value in the store to the given `ArrayBuilder`.
    #[cfg(feature = "serde")]
    pub fn build_value_at(
        &mut self,
        builder: &mut serde_arrow::ArrayBuilder,
        tag: Tag,
    ) -> Result<(), crate::RuntimeError> {
        self.store
            .lock()
            .expect("lock")
            .build_value_at(builder, tag)
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

    pub fn as_physical(&mut self) -> Option<&mut PhysicalAction> {
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
