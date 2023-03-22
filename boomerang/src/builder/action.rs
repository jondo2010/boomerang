//! Builder for actions. This is typically done using the builder methods on [`crate::builder::env`]
//! and [`crate::builder::reactor`].
//!
//! An action, like a port (see [`crate::builder::PortBuilder`]), can carry data, but unlike a port,
//! an action is visible only within the reactor that defines it.

use std::{fmt::Debug, marker::PhantomData};

use crate::runtime;
use slotmap::SecondaryMap;

use super::BuilderReactionKey;

slotmap::new_key_type! {pub struct BuilderActionKey;}

/// `TypedActionKey` is a typed wrapper around `ActionKey` that is used to associate a type with an
/// action. This is used to ensure that the type of the action matches the type of the port that it
/// is connected to.
#[derive(Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[repr(transparent)]
pub struct TypedActionKey<T: runtime::PortData = ()>(BuilderActionKey, PhantomData<T>);

impl<T: runtime::PortData> From<BuilderActionKey> for TypedActionKey<T> {
    fn from(key: BuilderActionKey) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: runtime::PortData> From<TypedActionKey<T>> for BuilderActionKey {
    fn from(key: TypedActionKey<T>) -> Self {
        key.0
    }
}

impl<T: runtime::PortData> runtime::InnerType for TypedActionKey<T> {
    type Inner = T;
}

#[derive(Debug)]
pub enum ActionType {
    Timer {
        period: runtime::Duration,
        offset: runtime::Duration,
    },
    Logical {
        min_delay: Option<runtime::Duration>,
    },
    Shutdown,
}

pub trait ActionBuilderFn: Fn(&str, runtime::ActionKey) -> runtime::InternalAction {}
impl<F> ActionBuilderFn for F where F: Fn(&str, runtime::ActionKey) -> runtime::InternalAction {}

impl Debug for Box<dyn ActionBuilderFn> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Box<dyn ActionBuilderFn>").finish()
    }
}

#[derive(Debug)]
pub struct ActionBuilder {
    /// Name of the Action
    name: String,
    /// Logical type of the action
    ty: ActionType,
    /// Out-going Reactions that this action triggers
    pub triggers: SecondaryMap<BuilderReactionKey, ()>,
    /// List of Reactions that may schedule this action
    pub schedulers: SecondaryMap<BuilderReactionKey, ()>,
    /// User builder function for the Action
    action_builder_fn: Box<dyn ActionBuilderFn>,
}

impl ActionBuilder {
    pub fn new(name: &str, ty: ActionType, action_builder_fn: Box<dyn ActionBuilderFn>) -> Self {
        Self {
            name: name.to_owned(),
            ty,
            triggers: SecondaryMap::new(),
            schedulers: SecondaryMap::new(),
            action_builder_fn,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_type(&self) -> &ActionType {
        &self.ty
    }

    /// Build the `ActionBuilder` into a [`runtime::InternalAction`]
    pub fn build_runtime(&self, action_key: runtime::ActionKey) -> runtime::InternalAction {
        (self.action_builder_fn)(&self.name, action_key)
    }
}
