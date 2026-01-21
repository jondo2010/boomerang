//! Actions are Reactor elements that can be scheduled. When an Action triggers, all Reactions dependent on that Action are executed.
//!
//! Actions come in two flavours that specify their scheduling behavior:
//! - `LogicalAction`: Logical Actions are scheduled with a [`Tag`] equal to the current *logical time* + optional delay.
//! - `PhysicalAction`: Physical Actions are scheduled with a [`Tag`] equal to the current *physical time* + optional delay.
//!
//! Actions can be scheduled in two ways:
//! **Synchronous**: Actions are scheduled synchronously from within a Reaction using an `ActionRef`. This is the most common
//!     way to schedule Actions. A future `Tag` for the event is calculated for both Logical and Physical Actions, and the
//!     `ReactorData` is pushed directly into the Action's store.
//! **Asynchronous**: Actions are scheduled asynchronously from oustide of the scheduler thread. This is useful when the
//!     Action needs to be scheduled from a different thread. An `AsyncEvent` is created and pushed onto the `async_tx`
//!     channel.

use std::fmt::{Debug, Display};

use crate::{Duration, ReactorData, Tag};

mod action_ref;
pub mod store;

pub use action_ref::*;
use downcast_rs::Downcast;
use store::ActionStore;

tinymap::key_type! { pub ActionKey }

pub trait BaseAction: Debug + Downcast + Send + Sync {
    /// Get the name of this action
    fn name(&self) -> &str;

    /// Get the key for this action
    fn key(&self) -> ActionKey;

    /// Get the minimum delay for this action
    fn min_delay(&self) -> Option<Duration>;

    /// Return true if the action is logical
    fn is_logical(&self) -> bool;

    /// Get the concrete type name carried by this action
    fn type_name(&self) -> &'static str;

    /// Push a new value onto the action store. If the underlying types are not the same, this will panic.
    fn push_value(&mut self, tag: Tag, value: Box<dyn ReactorData>);
}

downcast_rs::impl_downcast!(BaseAction);

impl Display for dyn BaseAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Action({})", self.name())
    }
}

pub struct Action<T: ReactorData = ()> {
    name: String,
    key: ActionKey,
    min_delay: Option<Duration>,
    store: ActionStore<T>,
    is_logical: bool,
}

impl<T: ReactorData> Debug for Action<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Action")
            .field("name", &self.name)
            .field("key", &self.key)
            .field("min_delay", &self.min_delay)
            .field("store", &self.store)
            .field("is_logical", &self.is_logical)
            .finish()
    }
}

impl<T: ReactorData> BaseAction for Action<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> ActionKey {
        self.key
    }

    fn min_delay(&self) -> Option<Duration> {
        self.min_delay
    }

    fn is_logical(&self) -> bool {
        self.is_logical
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn push_value(&mut self, tag: Tag, value: Box<dyn ReactorData>) {
        if let Ok(v) = value.downcast() {
            self.store.push(tag, *v);
        } else {
            panic!("Type mismatch");
        }
    }
}

impl<T: ReactorData> Action<T> {
    pub fn new(name: &str, key: ActionKey, min_delay: Option<Duration>, is_logical: bool) -> Self {
        Self {
            name: name.into(),
            key,
            min_delay,
            store: ActionStore::new(),
            is_logical,
        }
    }

    pub fn boxed(self) -> Box<dyn BaseAction> {
        Box::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action2() {
        let _actions =
            tinymap::TinyMap::<ActionKey, Box<dyn BaseAction>>::from_iter([Action::<i32>::new(
                "action0",
                ActionKey::from(0),
                None,
                true,
            )
            .boxed()]);
    }
}
