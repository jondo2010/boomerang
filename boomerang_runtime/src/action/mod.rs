//! Actions are Reactor elements that can be scheduled. When an Action triggers, all Reactions dependent on that Action are executed.
//!
//! Actions come in two flavours that specify their scheduling behavior:
//! - `LogicalAction`: Logical Actions are scheduled with a [`Tag`] equal to the current *logical time* + optional delay.
//! - `PhysicalAction`: Physical Actions are scheduled with a [`Tag`] equal to the current *physical time* + optional delay.
//!
//! Actions can be scheduled in two ways:
//! **Synchronous**: Actions are scheduled synchronously from within a Reaction using an `ActionRef`. This is the most common
//!     way to schedule Actions. A future `Tag` for the event is calculated for both Logical and Physical Actions, and the
//!     `ActionData` is pushed directly into the Action's store.
//! **Asynchronous**: Actions are scheduled asynchronously from oustide of the scheduler thread. This is useful when the
//!     Action needs to be scheduled from a different thread. An `AsyncEvent` is created and pushed onto the `async_tx`
//!     channel.
use std::{
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{event::AsyncEvent, Context, SendContext, Tag, Timestamp};

mod action_ref;
mod store;

pub use action_ref::*;
use downcast_rs::Downcast;
use store::{ActionStore, BaseActionStore};

#[cfg(not(feature = "serde"))]
pub trait ActionData: Debug + Send + Sync + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> ActionData for T where T: Debug + Send + Sync + 'static {}

#[cfg(feature = "serde")]
pub trait ActionData: Debug
    + Send
    + Sync
    //+ serde::Serialize
    //+ for<'de> serde::Deserialize<'de>
    + Downcast
    + 'static
{
}

#[cfg(feature = "serde")]
impl<T> ActionData for T where
    T: Debug
        + Send
        + Sync
        //+ serde::Serialize
        //+ for<'de> serde::Deserialize<'de>
        + Downcast
        + 'static
{
}

downcast_rs::impl_downcast!(ActionData);

tinymap::key_type! {
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub ActionKey
}

pub trait BaseAction: Debug + Send + Sync + Downcast {
    /// Get the name of this action
    fn name(&self) -> &str;

    /// Get the key for this action
    fn key(&self) -> ActionKey;

    /// Get the minimum delay for this action
    fn min_delay(&self) -> Option<Duration>;

    /// Return true if the action is logical
    fn is_logical(&self) -> bool;

    /// Push a new value onto the action store. If the underlying types are not the same, this will panic.
    fn push_value(&mut self, tag: Tag, value: Option<Box<dyn ActionData>>);
}

#[derive(Debug)]
pub struct Action2<T: ActionData = ()> {
    name: String,
    key: ActionKey,
    min_delay: Option<Duration>,
    store: ActionStore<T>,
    is_logical: bool,
}

impl<T: ActionData> BaseAction for Action2<T> {
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

    fn push_value(&mut self, tag: Tag, value: Option<Box<dyn ActionData>>) {
        self.store.push(tag, value.map(|v| *v.downcast().unwrap()));
    }
}

impl<T: ActionData> Action2<T> {
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

pub struct ActionRef2<'a, T: ActionData = ()>(&'a mut Action2<T>);

impl<'a, T: ActionData> ActionRef2<'a, T> {
    /// Return true if the action is present at the current tag
    pub fn is_present(&mut self, context: &Context) -> bool {
        self.0.store.get_current(context.tag).is_some()
    }

    /// Schedule a new value for this action
    pub fn schedule(&mut self, context: &mut Context, value: Option<T>, delay: Option<Duration>) {
        let action = &mut self.0;

        let tag_delay = action.min_delay.unwrap_or_default() + delay.unwrap_or_default();

        let new_tag = if action.is_logical {
            // Logical actions are scheduled at the current logical time + tag_delay
            context.tag.delay(tag_delay)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            Tag::absolute(context.start_time, Timestamp::now().offset(tag_delay))
        };

        // Push the new value into the store
        action.store.push(new_tag, value);

        // Schedule the action to trigger at the new tag
        context
            .trigger_res
            .scheduled_actions
            .push((action.key, new_tag));
    }
}

pub struct AsyncActionRef<T: ActionData = ()> {
    key: ActionKey,
    min_delay: Option<Duration>,
    is_logical: bool,
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<T: ActionData> AsyncActionRef<T> {
    /// Schedule a new value for this action
    pub fn schedule(&self, context: &SendContext, value: Option<T>, delay: Option<Duration>) {
        let tag_delay = self.min_delay.unwrap_or_default() + delay.unwrap_or_default();
        let value: Option<Box<dyn ActionData>> = value.map(|v| Box::new(v) as _);

        let event = if self.is_logical {
            // Logical actions are scheduled at the current logical time + tag_delay
            tracing::info!(tag_delay = ?tag_delay, key = ?self.key, "Scheduling Async LogicalAction");
            AsyncEvent::logical(self.key, tag_delay, value)
        } else {
            // Physical actions are scheduled at the current physical time + tag_delay
            let new_tag = Tag::absolute(context.start_time, Timestamp::now().offset(tag_delay));
            tracing::info!(new_tag = %new_tag, key = ?self.key, "Scheduling Async PhysicalAction");
            AsyncEvent::physical(self.key, new_tag, value)
        };

        context
            .async_tx
            .send(event)
            .expect("Failed to send async event");
    }
}

/// Typed Action state, storing potentially different values at different Tags.
#[derive(Debug)]
pub struct LogicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub store: Box<dyn BaseActionStore>,
}

#[derive(Debug)]
pub struct PhysicalAction {
    pub name: String,
    pub key: ActionKey,
    pub min_delay: Duration,
    pub store: Arc<Mutex<dyn BaseActionStore>>,
}

impl PhysicalAction {
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
        let store = ActionStore::<T>::new();
        Self::Logical(LogicalAction {
            name: name.into(),
            key,
            min_delay,
            store: Box::new(store),
        })
    }

    pub fn new_physical<T: ActionData>(name: &str, key: ActionKey, min_delay: Duration) -> Self {
        let store = ActionStore::<T>::new();
        let store: Arc<Mutex<dyn BaseActionStore>> = Arc::new(Mutex::new(store));
        Self::Physical(PhysicalAction {
            name: name.into(),
            key,
            min_delay,
            store,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action2() {
        let actions =
            tinymap::TinyMap::<ActionKey, Box<dyn BaseAction>>::from_iter([Action2::<i32>::new(
                "action0",
                ActionKey::from(0),
                None,
                true,
            )
            .boxed()]);
    }
}
