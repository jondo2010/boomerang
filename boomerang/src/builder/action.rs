use std::{fmt::Debug, marker::PhantomData};

use crate::runtime;
use slotmap::SecondaryMap;

use super::{args::{ActionArgs, TimerArgs}, BuilderError, ReactorBuilderState, ReactorPart};

#[derive(Clone, Copy, Debug)]
pub struct ActionPart<T: runtime::PortData = ()>(runtime::ActionKey, PhantomData<T>);

impl<T: runtime::PortData> From<ActionPart<T>> for runtime::ActionKey {
    fn from(part: ActionPart<T>) -> Self {
        part.0
    }
}

impl<T: runtime::PortData> ReactorPart for ActionPart<T> {
    type Args = ActionArgs;
    fn build_part<S: runtime::ReactorState>(
        builder: &mut ReactorBuilderState<S>,
        name: &str,
        args: Self::Args,
    ) -> Result<Self, BuilderError> {
        builder.add_logical_action::<T>(name, args.min_delay)
    }
}

impl<T: runtime::PortData> runtime::InnerType for ActionPart<T> {
    type Inner = T;
}

impl<T: runtime::PortData> ActionPart<T> {
    pub fn new(action_key: runtime::ActionKey) -> Self {
        Self(action_key, PhantomData)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TimerPart(pub(crate) runtime::ActionKey);

impl From<TimerPart> for runtime::ActionKey {
    fn from(part: TimerPart) -> Self {
        part.0
    }
}

impl ReactorPart for TimerPart {
    type Args = TimerArgs;
    fn build_part<S: runtime::ReactorState>(
        builder: &mut ReactorBuilderState<S>,
        name: &str,
        args: Self::Args,
    ) -> Result<Self, BuilderError> {
        builder.add_timer(name, args.period.unwrap_or_default(), args.offset.unwrap_or_default())
    }
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
    /// The key of this action in the EnvBuilder
    action_key: runtime::ActionKey,
    /// The parent Reactor that owns this Action
    reactor_key: runtime::ReactorKey,
    /// Out-going Reactions that this action triggers
    pub triggers: SecondaryMap<runtime::ReactionKey, ()>,
    /// List of Reactions that may schedule this action
    pub schedulers: SecondaryMap<runtime::ReactionKey, ()>,
    /// User builder function for the Action
    action_builder_fn: Box<dyn ActionBuilderFn>,
}

impl ActionBuilder {
    pub fn new(
        name: &str,
        ty: ActionType,
        action_key: runtime::ActionKey,
        reactor_key: runtime::ReactorKey,
        action_builder_fn: Box<dyn ActionBuilderFn>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            ty,
            action_key,
            reactor_key,
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

    pub fn get_action_key(&self) -> runtime::ActionKey {
        self.action_key
    }

    pub fn get_reactor_key(&self) -> runtime::ReactorKey {
        self.reactor_key
    }

    /// Build the ActionBuilder into a runtime Action
    pub fn into_action(&self) -> runtime::InternalAction {
        (self.action_builder_fn)(&self.name, self.action_key)
    }
}

#[cfg(test)]
mod tests {
    use crate::builder::{EnvBuilder, ActionPart, args::{ReactorPartArgs, ActionArgs}, ReactorPart};

    #[test]
    fn test_build_part() {
        struct T {
            a: ActionPart<u32>,
        }

        let mut env = EnvBuilder::new();
        let mut builder = env.add_reactor("test", None, ());
        let args = ActionArgs {
            physical: false,
            min_delay: None,
            mit: None,
            policy: None,
        };

        let t = T {
            a: ReactorPart::build_part(&mut builder, "a", args).unwrap(),
        };
    }
}
