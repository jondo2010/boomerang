use std::{
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{ActionData, Tag};

mod action_ref;
mod registry;
pub mod store;

pub use action_ref::*;
use store::{ActionStore, BaseActionStore};

tinymap::key_type! { pub ActionKey }

/// Typed Action state, storing potentially different values at different Tags.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PhysicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    #[cfg_attr(feature = "serde", serde(with = "serialize_physical_action_store"))]
    pub store: Arc<Mutex<dyn BaseActionStore>>,
}

mod serialize_physical_action_store {
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::BaseActionStore;

    pub fn serialize<S>(
        store: &Arc<Mutex<dyn BaseActionStore>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let store = store.lock().expect("Failed to lock action store");
        store.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<Mutex<dyn BaseActionStore>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let store = Box::<dyn BaseActionStore>::deserialize(deserializer)?;
        Ok(store.boxed_to_mutex())
    }
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

impl<'a> From<&'a mut Action> for &'a mut PhysicalAction {
    fn from(value: &'a mut Action) -> Self {
        value.as_physical().expect("Action is not physical")
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Action {
    /// Startup is a special action that fires when the scheduler starts up.
    Startup,
    /// Shutdown is a special action that fires when the scheduler shuts down.
    Shutdown,
    Logical(LogicalAction),
    Physical(PhysicalAction),
}

impl Action {
    pub fn new_logical<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        Self::Logical(LogicalAction::new::<T>(name, key, min_delay))
    }

    pub fn new_physical<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        Self::Physical(PhysicalAction::new::<T>(name, key, min_delay))
    }

    pub fn as_logical(&self) -> Option<&LogicalAction> {
        if let Self::Logical(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_logical_mut(&mut self) -> Option<&mut LogicalAction> {
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
            Action::Startup => write!(f, "Action::Startup"),
            Action::Shutdown => write!(f, "Action::Shutdown"),
            Action::Logical(logical) => write!(f, "Action::Logical({name})", name = logical.name),
            Action::Physical(physical) => {
                write!(f, "Action::Physical({name})", name = physical.name)
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

pub trait ActionCommon {
    fn name(&self) -> &str;
    fn key(&self) -> ActionKey;
    fn min_delay(&self) -> Duration;
}

impl ActionCommon for LogicalAction {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> ActionKey {
        self.key
    }

    fn min_delay(&self) -> Duration {
        self.min_delay
    }
}

impl ActionCommon for PhysicalAction {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> ActionKey {
        self.key
    }

    fn min_delay(&self) -> Duration {
        self.min_delay
    }
}
