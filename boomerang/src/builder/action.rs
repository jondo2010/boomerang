//! Builder for actions. This is typically done using the builder methods on [`crate::builder::env`]
//! and [`crate::builder::reactor`].
//!
//! An action, like a port (see [`crate::builder::PortBuilder`]), can carry data, but unlike a port,
//! an action is visible only within the reactor that defines it.

use std::{fmt::Debug, marker::PhantomData, time::Duration};

use crate::runtime;
use slotmap::SecondaryMap;

use super::BuilderReactionKey;

slotmap::new_key_type! {pub struct BuilderActionKey;}

#[derive(Copy, Clone, Debug)]
pub struct Logical;

#[derive(Copy, Clone, Debug)]
pub struct Physical;

/// `TypedActionKey` is a typed wrapper around `ActionKey` that is used to associate a type with an
/// action. This is used to ensure that the type of the action matches the type of the port that it
/// is connected to.
#[derive(Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[repr(transparent)]
pub struct TypedActionKey<T = (), Q = Logical>(BuilderActionKey, PhantomData<(T, Q)>)
where
    T: runtime::PortData;

impl<T: runtime::PortData, Q> From<BuilderActionKey> for TypedActionKey<T, Q> {
    fn from(key: BuilderActionKey) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: runtime::PortData, Q> From<TypedActionKey<T, Q>> for BuilderActionKey {
    fn from(key: TypedActionKey<T, Q>) -> Self {
        key.0
    }
}

impl<T: runtime::PortData, Q> runtime::InnerType for TypedActionKey<T, Q> {
    type Inner = T;
}

#[derive(Debug)]
pub enum ActionType {
    Timer { period: Duration, offset: Duration },
    Logical { min_delay: Option<Duration> },
    Physical { min_delay: Option<Duration> },
    Startup,
    Shutdown,
}

pub trait ActionBuilderFn: Fn(&str, runtime::keys::ActionKey) -> runtime::Action {}
impl<F> ActionBuilderFn for F where F: Fn(&str, runtime::keys::ActionKey) -> runtime::Action {}

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
    pub fn build_runtime(&self, action_key: runtime::keys::ActionKey) -> runtime::Action {
        (self.action_builder_fn)(&self.name, action_key)
    }
}
