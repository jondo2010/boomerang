use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::{Debug, Display},
};

use crate::{Duration, InnerType, PortData, Tag};
use downcast_rs::{impl_downcast, DowncastSync};

tinymap::key_type! { pub ActionKey }

pub struct ActionMut<'a, T: PortData = ()> {
    pub(crate) key: ActionKey,
    pub(crate) min_delay: &'a Duration,
    pub(crate) values: &'a mut ActionValues<T>,
}

pub struct Action<'a, T: PortData = ()> {
    pub(crate) key: ActionKey,
    pub(crate) min_delay: &'a Duration,
    pub(crate) values: &'a ActionValues<T>,
}

impl<'a, T: PortData> InnerType for ActionMut<'a, T> {
    type Inner = T;
}

impl<'a, T: PortData> InnerType for Action<'a, T> {
    type Inner = T;
}

impl<'a, T: PortData> From<&'a mut InternalAction> for ActionMut<'a, T> {
    fn from(action: &'a mut InternalAction) -> Self {
        action
            .as_valued_mut()
            .expect("Expected ValuedAction")
            .into()
    }
}

impl<'a, T: PortData> From<&'a InternalAction> for Action<'a, T> {
    fn from(action: &'a InternalAction) -> Self {
        action.as_valued().expect("Expected ValuedAction").into()
    }
}

impl<'a, T: PortData> From<&'a mut ValuedAction> for ActionMut<'a, T> {
    fn from(valued_action: &'a mut ValuedAction) -> Self {
        let values = valued_action
            .values
            .downcast_mut::<ActionValues<T>>()
            .expect("Type mismatch on ActionValues!");
        Self {
            key: valued_action.key,
            min_delay: &valued_action.min_delay,
            values,
        }
    }
}

impl<'a, T: PortData> From<&'a ValuedAction> for Action<'a, T> {
    fn from(valued_action: &'a ValuedAction) -> Self {
        let values = valued_action
            .values
            .downcast_ref::<ActionValues<T>>()
            .expect("Type mismatch on ActionValues!");
        Self {
            key: valued_action.key,
            min_delay: &valued_action.min_delay,
            values,
        }
    }
}

pub trait BaseActionValues: Debug + Send + Sync + DowncastSync {
    /// Remove any value at the given Tag
    fn remove(&mut self, tag: Tag);
}
impl_downcast!(sync BaseActionValues);

#[derive(Debug)]
pub(crate) struct ActionValues<T: PortData>(HashMap<Tag, T>);
impl<T: PortData> BaseActionValues for ActionValues<T> {
    fn remove(&mut self, tag: Tag) {
        self.0.remove(&tag);
    }
}
impl<T: PortData> ActionValues<T> {
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
pub struct ValuedAction {
    pub name: String,
    pub key: ActionKey,
    pub logical: bool,
    pub min_delay: Duration,
    pub values: Box<dyn BaseActionValues>,
}

impl ValuedAction {
    pub fn new<T: PortData>(
        name: &str,
        key: ActionKey,
        logical: bool,
        min_delay: Duration,
    ) -> Self {
        Self {
            name: name.into(),
            key,
            logical,
            min_delay,
            values: Box::new(ActionValues::<T>(HashMap::new())),
        }
    }
}

#[derive(Debug)]
pub enum InternalAction {
    Timer {
        name: String,
        key: ActionKey,
        offset: Duration,
        period: Duration,
    },
    /// ShutdownAction is a logical action that fires when the scheduler shuts down.
    Shutdown {
        name: String,
        key: ActionKey,
    },
    Valued(ValuedAction),
}

impl InternalAction {
    pub fn get_name(&self) -> &str {
        match self {
            InternalAction::Timer { name, .. } => name.as_ref(),
            InternalAction::Shutdown { name, .. } => name.as_ref(),
            InternalAction::Valued(ValuedAction { name, .. }) => name.as_ref(),
        }
    }

    pub fn get_key(&self) -> ActionKey {
        match self {
            InternalAction::Timer { key, .. } => *key,
            InternalAction::Shutdown { key, .. } => *key,
            InternalAction::Valued(ValuedAction { key, .. }) => *key,
        }
    }

    pub fn get_is_logical(&self) -> bool {
        match self {
            InternalAction::Timer { .. } => true,
            InternalAction::Shutdown { .. } => true,
            InternalAction::Valued(ValuedAction { logical, .. }) => *logical,
        }
    }

    pub fn as_valued(&self) -> Option<&ValuedAction> {
        if let Self::Valued(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_valued_mut(&mut self) -> Option<&mut ValuedAction> {
        if let Self::Valued(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl Display for InternalAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InternalAction::Timer {
                name,
                offset,
                period,
                ..
            } => f.write_fmt(format_args!(
                "Action::Timer<{name}, {offset:#?}, {period:#?}>"
            )),
            InternalAction::Shutdown { name, .. } => {
                f.write_fmt(format_args!("Action::Shutdown<{name}>"))
            }
            InternalAction::Valued(ValuedAction { name, .. }) => {
                f.write_fmt(format_args!("Action::BaseAction<{name}>"))
            }
        }
    }
}

impl AsRef<InternalAction> for InternalAction {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<InternalAction> for InternalAction {
    fn as_mut(&mut self) -> &mut InternalAction {
        self
    }
}
