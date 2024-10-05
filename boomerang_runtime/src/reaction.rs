use std::{fmt::Debug, sync::RwLock, time::Duration};

use crate::{
    key_set::KeySet,
    refs::{Refs, RefsMut},
    Action, BasePort, Context, ReactorState,
};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

pub trait ReactionFn<'store> {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        state: &'store mut dyn ReactorState,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    );
}

pub type BoxedReactionFn = Box<dyn for<'store> ReactionFn<'store> + Send + Sync>;

pub type BoxedHandlerFn = Box<dyn Fn() + Send + Sync>;

/// Conversion trait for creating a Reaction struct from port and action references.
///
/// This trait is typically automatically implemented by the derive macro.
pub trait FromRefs {
    type Marker<'store>;

    fn from_refs<'store>(
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) -> Self::Marker<'store>;
}

/// The `Trigger` trait should be implemented by the user for each Reaction struct.
///
/// Type parameter `S` is the state type of the Reactor.
pub trait Trigger<S: ReactorState> {
    fn trigger(self, ctx: &mut Context, state: &mut S);
}

/// Wrapper struct for implementing the `ReactionFn` trait for a Reaction struct.
pub struct ReactionWrapper<Reaction, S>(std::marker::PhantomData<(Reaction, S)>);

impl<Reaction, S> Default for ReactionWrapper<Reaction, S> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<'store, Reaction, S> ReactionFn<'store> for ReactionWrapper<Reaction, S>
where
    Reaction: FromRefs,
    Reaction::Marker<'store>: 'store + Trigger<S>,
    S: ReactorState,
{
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        state: &'store mut dyn ReactorState,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) {
        let state: &mut S = state
            .downcast_mut()
            .expect("Unable to downcast reactor state");

        let reaction = Reaction::from_refs(ports, ports_mut, actions);
        reaction.trigger(ctx, state);
    }
}

impl<'store, F> ReactionFn<'store> for F
where
    F: FnMut(
            &'store mut Context,
            &'store mut dyn ReactorState,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, Action>,
        ) + Sync
        + Send,
{
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        state: &'store mut dyn ReactorState,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) {
        (self)(ctx, state, ports, ports_mut, actions);
    }
}

pub struct Deadline {
    pub(crate) deadline: Duration,
    #[allow(dead_code)]
    pub(crate) handler: RwLock<BoxedHandlerFn>,
}

impl Debug for Deadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deadline")
            .field("deadline", &self.deadline)
            .field("handler", &"HandlerFn()")
            .finish()
    }
}

pub struct Reaction {
    name: String,
    /// Reaction closure
    pub(crate) body: BoxedReactionFn,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    pub(crate) deadline: Option<Deadline>,
}

impl Debug for Reaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reaction")
            .field("name", &self.name)
            .field("body", &"ReactionFn()")
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl Reaction {
    pub fn new(name: &str, body: BoxedReactionFn, deadline: Option<Deadline>) -> Self {
        Self {
            name: name.to_owned(),
            body,
            deadline,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}
