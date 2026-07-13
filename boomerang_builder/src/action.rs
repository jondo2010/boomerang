//! Action specifications recorded through [`crate::Assembly`] and [`crate::ReactorContext`].
//!
//! An action, like a port (see [`crate::PortSpec`]), can carry data, but unlike a port,
//! an action is visible only within the reactor that defines it.

use std::{fmt::Debug, marker::PhantomData};

use super::{AssemblyModeKey, AssemblyReactorKey};
use crate::{runtime, ParentReactorSpec};

slotmap::new_key_type! {pub struct AssemblyActionKey;}

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

/// `TypedActionKey` is a typed wrapper around [`AssemblyActionKey`] that is used to associate a type
/// with an action. This is used to ensure that the type of the action matches the type of the port
/// that it is connected to.
#[derive(Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct TypedActionKey<T = (), Q = Logical>(AssemblyActionKey, PhantomData<(T, Q)>)
where
    T: runtime::ReactorData,
    Q: ActionTag;

impl<T: runtime::ReactorData, Q: ActionTag> Copy for TypedActionKey<T, Q> {}

impl<T: runtime::ReactorData, Q: ActionTag> Clone for TypedActionKey<T, Q> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> From<AssemblyActionKey> for TypedActionKey<T, Q> {
    fn from(key: AssemblyActionKey) -> Self {
        Self(key, PhantomData)
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> From<TypedActionKey<T, Q>> for AssemblyActionKey {
    fn from(key: TypedActionKey<T, Q>) -> Self {
        key.0
    }
}

/// `PhysicalActionKey` is a type-erased physical Action.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct PhysicalActionKey(AssemblyActionKey);

impl From<AssemblyActionKey> for PhysicalActionKey {
    fn from(value: AssemblyActionKey) -> Self {
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

impl<T: runtime::ReactorData, Q: ActionTag> runtime::ReactionRefsExtract for TypedActionKey<T, Q> {
    type Ref<'store>
        = runtime::ActionRef<'store, T>
    where
        Self: 'store;
    fn extract<'store>(
        &self,
        refs: &mut runtime::ReactionRefs<'store>,
    ) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let action = refs
            .actions
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("action"))?;

        runtime::ActionRef::try_from(runtime::DynActionRefMut(action))
    }
}

impl From<PhysicalActionKey> for AssemblyActionKey {
    fn from(value: PhysicalActionKey) -> Self {
        value.0
    }
}

/// `TimerActionKey` is an wrapper around [`AssemblyActionKey`] for timer Actions.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct TimerActionKey(AssemblyActionKey);

impl From<TimerActionKey> for AssemblyActionKey {
    fn from(value: TimerActionKey) -> Self {
        value.0
    }
}

impl From<TimerActionKey> for TypedActionKey<()> {
    fn from(value: TimerActionKey) -> Self {
        Self(value.into(), PhantomData)
    }
}

impl From<AssemblyActionKey> for TimerActionKey {
    fn from(value: AssemblyActionKey) -> Self {
        Self(value)
    }
}

impl runtime::ReactionRefsExtract for TimerActionKey {
    type Ref<'store>
        = runtime::ActionRef<'store>
    where
        Self: 'store;
    fn extract<'store>(
        &self,
        refs: &mut runtime::ReactionRefs<'store>,
    ) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let action = refs
            .actions
            .next()
            .ok_or_else(|| runtime::ReactionRefsError::missing("timer action"))?;

        runtime::ActionRef::try_from(runtime::DynActionRefMut(action))
    }
}

/// TimerSpec is used to specify the period and offset of a timer action.
///
/// If the period is `None`, then the timer event occurs only once. If neither an offset nor a period is specified, then one timer event occurs at program start.
#[derive(Debug, PartialEq, Eq, Default)]
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
    pub fn with_period(self, period: runtime::Duration) -> Self {
        Self {
            period: Some(period),
            ..self
        }
    }
    pub fn with_offset(self, offset: runtime::Duration) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }
}

#[derive(Debug)]
pub enum ActionType {
    Timer(TimerSpec),
    Standard {
        /// Whether the action is logical or physical
        is_logical: bool,
        /// Minimum delay between
        min_delay: Option<runtime::Duration>,
        /// Factory function that creates the runtime action.
        build_fn: Box<dyn ActionFactoryFn>,
    },
    Shutdown,
}

pub trait ActionFactoryFn: Fn(&str, runtime::ActionKey) -> Box<dyn runtime::BaseAction> {}
impl<F> ActionFactoryFn for F where F: Fn(&str, runtime::ActionKey) -> Box<dyn runtime::BaseAction> {}

impl Debug for dyn ActionFactoryFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("dyn ActionFactoryFn").finish()
    }
}

#[derive(Debug)]
pub struct ActionSpec {
    /// Name of the Action
    name: String,
    /// The key of the Reactor that owns this ActionSpec
    reactor_key: AssemblyReactorKey,
    /// Enclosing mode scope, if this action was declared inside a mode.
    scope_mode: Option<AssemblyModeKey>,
    /// Logical type of the action
    r#type: ActionType,
}

impl ParentReactorSpec for ActionSpec {
    fn parent_reactor_key(&self) -> Option<AssemblyReactorKey> {
        Some(self.reactor_key)
    }
}

impl ActionSpec {
    pub fn new(
        name: &str,
        reactor_key: AssemblyReactorKey,
        scope_mode: Option<AssemblyModeKey>,
        r#type: ActionType,
    ) -> Self {
        Self {
            name: name.to_owned(),
            reactor_key,
            scope_mode,
            r#type,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn reactor_key(&self) -> AssemblyReactorKey {
        self.reactor_key
    }

    pub fn scope_mode(&self) -> Option<AssemblyModeKey> {
        self.scope_mode
    }

    pub fn r#type(&self) -> &ActionType {
        &self.r#type
    }
}
