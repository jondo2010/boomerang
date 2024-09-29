//! Builder for actions. This is typically done using the builder methods on [`crate::builder::env`]
//! and [`crate::builder::reactor`].
//!
//! An action, like a port (see [`crate::builder::PortBuilder`]), can carry data, but unlike a port,
//! an action is visible only within the reactor that defines it.

use std::{fmt::Debug, marker::PhantomData};

use super::{BuilderReactionKey, BuilderReactorKey};
use crate::runtime;

use slotmap::SecondaryMap;

slotmap::new_key_type! {pub struct BuilderActionKey;}

#[derive(Copy, Clone, Debug)]
pub struct Logical;

#[derive(Copy, Clone, Debug)]
pub struct Physical;

/// `TypedActionKey` is a typed wrapper around [`BuilderActionKey`] that is used to associate a type with an
/// action. This is used to ensure that the type of the action matches the type of the port that it
/// is connected to.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct TypedActionKey<T = (), Q = Logical>(BuilderActionKey, PhantomData<(T, Q)>)
where
    T: runtime::ActionData;

impl<T: runtime::ActionData, Q> From<BuilderActionKey> for TypedActionKey<T, Q> {
    fn from(key: BuilderActionKey) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: runtime::ActionData, Q> From<TypedActionKey<T, Q>> for BuilderActionKey {
    fn from(key: TypedActionKey<T, Q>) -> Self {
        key.0
    }
}

/// `PhysicalActionKey` is a type-erased physical Action.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct PhysicalActionKey(BuilderActionKey);

impl From<BuilderActionKey> for PhysicalActionKey {
    fn from(value: BuilderActionKey) -> Self {
        Self(value)
    }
}

impl From<PhysicalActionKey> for TypedActionKey<(), Physical> {
    fn from(value: PhysicalActionKey) -> Self {
        Self(value.0, PhantomData)
    }
}

impl<T: runtime::ActionData> From<TypedActionKey<T, Physical>> for PhysicalActionKey {
    fn from(value: TypedActionKey<T, Physical>) -> Self {
        Self(value.0)
    }
}

impl From<PhysicalActionKey> for BuilderActionKey {
    fn from(value: PhysicalActionKey) -> Self {
        value.0
    }
}

/// `TimerActionKey` is an wrapper around [`BuilderActionKey`] for timer Actions.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct TimerActionKey(BuilderActionKey);

impl From<TimerActionKey> for BuilderActionKey {
    fn from(value: TimerActionKey) -> Self {
        value.0
    }
}

impl From<TimerActionKey> for TypedActionKey<()> {
    fn from(value: TimerActionKey) -> Self {
        Self(value.into(), PhantomData)
    }
}

impl From<BuilderActionKey> for TimerActionKey {
    fn from(value: BuilderActionKey) -> Self {
        Self(value)
    }
}

/// TimerSpec is used to specify the period and offset of a timer action.
#[derive(Debug, PartialEq, Eq)]
pub struct TimerSpec {
    /// Interval between timer events
    pub period: Option<runtime::Duration>,
    /// (logical) time interval between when the program starts executing and the first timer event
    pub offset: Option<runtime::Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ActionType {
    Timer(TimerSpec),
    Logical {
        min_delay: Option<runtime::Duration>,
    },
    Physical {
        min_delay: Option<runtime::Duration>,
    },
    Startup,
    Shutdown,
}

pub trait ActionBuilderFn: Fn(&str, runtime::ActionKey) -> runtime::Action {}
impl<F> ActionBuilderFn for F where F: Fn(&str, runtime::ActionKey) -> runtime::Action {}

impl Debug for Box<dyn ActionBuilderFn> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Box<dyn ActionBuilderFn>").finish()
    }
}

#[derive(Debug)]
pub struct ActionBuilder {
    /// Name of the Action
    name: String,
    /// The key of the Reactor that owns this ActionBuilder
    reactor_key: BuilderReactorKey,
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
    pub fn new(
        name: &str,
        reactor_key: BuilderReactorKey,
        ty: ActionType,
        action_builder_fn: Box<dyn ActionBuilderFn>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            reactor_key,
            ty,
            triggers: SecondaryMap::new(),
            schedulers: SecondaryMap::new(),
            action_builder_fn,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    pub fn get_type(&self) -> &ActionType {
        &self.ty
    }

    /// Build the `ActionBuilder` into a [`runtime::Action`]
    pub fn build_runtime(&self, action_key: runtime::ActionKey) -> runtime::Action {
        (self.action_builder_fn)(&self.name, action_key)
    }
}
