//! Builder for actions. This is typically done using the builder methods on [`crate::builder::env`]
//! and [`crate::builder::reactor`].
//!
//! An action, like a port (see [`crate::builder::PortBuilder`]), can carry data, but unlike a port,
//! an action is visible only within the reactor that defines it.

use std::{fmt::Debug, marker::PhantomData};

use super::BuilderReactorKey;
use crate::{runtime, ParentReactorBuilder};

slotmap::new_key_type! {pub struct BuilderActionKey;}

#[derive(Copy, Clone, Debug)]
pub struct Logical;

#[derive(Copy, Clone, Debug)]
pub struct Physical;

pub trait ActionTag: Copy + Clone + Debug + 'static {
    const IS_LOGICAL: bool;
}

impl ActionTag for Logical {
    const IS_LOGICAL: bool = true;
}

impl ActionTag for Physical {
    const IS_LOGICAL: bool = false;
}

/// `TypedActionKey` is a typed wrapper around [`BuilderActionKey`] that is used to associate a type
/// with an action. This is used to ensure that the type of the action matches the type of the port
/// that it is connected to.
#[derive(Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct TypedActionKey<T = (), Q = Logical>(BuilderActionKey, PhantomData<(T, Q)>)
where
    T: runtime::ReactorData,
    Q: ActionTag;

impl<T: runtime::ReactorData, Q: ActionTag> Copy for TypedActionKey<T, Q> {}

impl<T: runtime::ReactorData, Q: ActionTag> Clone for TypedActionKey<T, Q> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> From<BuilderActionKey> for TypedActionKey<T, Q> {
    fn from(key: BuilderActionKey) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> From<TypedActionKey<T, Q>> for BuilderActionKey {
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

impl<T: runtime::ReactorData> From<TypedActionKey<T, Physical>> for PhysicalActionKey {
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
///
/// If the period is `None`, then the timer event occurs only once. If neither an offset nor a period is specified, then one timer event occurs at program start.
#[derive(Debug, PartialEq, Eq)]
pub struct TimerSpec {
    /// Interval between timer events
    pub period: Option<runtime::Duration>,
    /// (logical) time interval between when the program starts executing and the first timer event
    pub offset: Option<runtime::Duration>,
}

impl TimerSpec {
    pub const STARTUP: Self = Self {
        period: None,
        offset: None,
    };
}

#[derive(Debug)]
pub enum ActionType {
    Timer(TimerSpec),
    Standard {
        /// Whether the action is logical or physical
        is_logical: bool,
        /// Minimum delay between
        min_delay: Option<runtime::Duration>,
        /// Builder function that creates the runtime action
        build_fn: Box<dyn ActionBuilderFn>,
    },
    Shutdown,
}

pub trait ActionBuilderFn: Fn(&str, runtime::ActionKey) -> Box<dyn runtime::BaseAction> {}
impl<F> ActionBuilderFn for F where F: Fn(&str, runtime::ActionKey) -> Box<dyn runtime::BaseAction> {}

impl Debug for dyn ActionBuilderFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("dyn ActionBuilderFn").finish()
    }
}

#[derive(Debug)]
pub struct ActionBuilder {
    /// Name of the Action
    name: String,
    /// The key of the Reactor that owns this ActionBuilder
    reactor_key: BuilderReactorKey,
    /// Logical type of the action
    r#type: ActionType,
}

impl ParentReactorBuilder for ActionBuilder {
    fn parent_reactor_key(&self) -> Option<BuilderReactorKey> {
        Some(self.reactor_key)
    }
}

impl ActionBuilder {
    pub fn new(name: &str, reactor_key: BuilderReactorKey, r#type: ActionType) -> Self {
        Self {
            name: name.to_owned(),
            reactor_key,
            r#type,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn reactor_key(&self) -> BuilderReactorKey {
        self.reactor_key
    }

    pub fn r#type(&self) -> &ActionType {
        &self.r#type
    }
}
