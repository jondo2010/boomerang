use std::{
    fmt::Display,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::ReactorData;

mod action_ref;
pub mod store;

pub use action_ref::*;
use store::{ActionStore, BaseActionStore};

tinymap::key_type! { pub ActionKey }

/// Typed Action state, storing potentially different values at different Tags.
#[derive(Debug)]
pub struct LogicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub store: Box<dyn BaseActionStore>,
}

impl LogicalAction {
    pub fn new<T: ReactorData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        let store = ActionStore::<T>::default();
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
    pub fn new<T: ReactorData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        let store = ActionStore::<T>::default();
        let store = Arc::new(Mutex::new(store)) as Arc<Mutex<dyn BaseActionStore>>;
        Self {
            name: name.into(),
            key,
            min_delay,
            store,
        }
    }
}

impl<'a> From<&'a mut Action> for &'a mut PhysicalAction {
    fn from(value: &'a mut Action) -> Self {
        value.as_physical().expect("Action is not physical")
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
    pub fn new_logical<T: ReactorData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        Self::Logical(LogicalAction::new::<T>(name, key, min_delay))
    }

    pub fn new_physical<T: ReactorData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
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
