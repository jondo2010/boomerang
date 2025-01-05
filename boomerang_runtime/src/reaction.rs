use std::{fmt::Debug, sync::RwLock};

use crate::{
    key_set::KeySet,
    refs::{Refs, RefsMut},
    ActionRef, BaseAction, BasePort, BaseReactor, Context, Duration, Reactor, ReactorData,
};

tinymap::key_type! { pub ReactionKey }

pub type ReactionSet = KeySet<ReactionKey>;

pub trait ReactionFn<'store>: Send + Sync {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        reactor: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
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
        actions: RefsMut<'store, dyn BaseAction>,
    ) -> Self::Marker<'store>;
}

/// The `Trigger` trait should be implemented by the user for each Reaction struct.
///
/// Type parameter `S` is the state type of the Reactor.
pub trait Trigger<S: ReactorData> {
    fn trigger(self, ctx: &mut Context, state: &mut S);
}

/// Adapter struct for implementing the `ReactionFn` trait for a Reaction struct.
///
/// The `ReactionAdapter` struct is used to convert a Reaction struct to a `Box<dyn ReactionFn>`. This is the mechanism
/// used by the derive-generated code to implement the Reaction trigger interface.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReactionAdapter<Reaction, State>(std::marker::PhantomData<fn() -> (Reaction, State)>);

impl<Reaction, State> Default for ReactionAdapter<Reaction, State> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<Reaction, State> From<ReactionAdapter<Reaction, State>> for BoxedReactionFn
where
    Reaction: FromRefs + 'static,
    for<'store> Reaction::Marker<'store>: 'store + Trigger<State>,
    State: ReactorData,
{
    fn from(adapter: ReactionAdapter<Reaction, State>) -> Self {
        Box::new(adapter)
    }
}

impl<'store, Reaction, S> ReactionFn<'store> for ReactionAdapter<Reaction, S>
where
    Reaction: FromRefs,
    Reaction::Marker<'store>: 'store + Trigger<S>,
    S: ReactorData,
{
    #[inline(always)]
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        reactor: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let reactor: &mut Reactor<S> = reactor
            .downcast_mut()
            .expect("Unable to downcast reactor state");

        let reaction = Reaction::from_refs(ports, ports_mut, actions);
        reaction.trigger(ctx, &mut reactor.state);
    }
}

/// Wrapper struct for implementing the `ReactionFn` trait for a generic FnMut function.
///
/// An `FnAdapter` can be created from a closure or function pointer and then converted to a `Box<dyn ReactionFn>`.
pub struct FnAdapter<F>(F);

impl<F> From<F> for BoxedReactionFn
where
    F: for<'store> Fn(
            &'store mut Context,
            &'store mut dyn BaseReactor,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, dyn BaseAction>,
        ) + Send
        + Sync
        + 'static,
{
    fn from(f: F) -> Self {
        Box::new(FnAdapter(f))
    }
}

impl<'store, F> ReactionFn<'store> for FnAdapter<F>
where
    F: Fn(
            &'store mut Context,
            &'store mut dyn BaseReactor,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, dyn BaseAction>,
        ) + Sync
        + Send,
{
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        state: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        (self.0)(ctx, state, ports, ports_mut, actions);
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
    pub fn new(name: &str, body: impl Into<BoxedReactionFn>, deadline: Option<Deadline>) -> Self {
        Self {
            name: name.to_owned(),
            body: body.into(),
            deadline,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

/// An empty reaction function that does nothing.
pub fn empty_reaction(
    _ctx: &mut Context,
    _reactor: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    _actions: RefsMut<dyn BaseAction>,
) {
}

/// Utility startup function for a timer action
pub fn timer_startup_fn(
    ctx: &mut Context,
    _reactor: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    actions: RefsMut<dyn BaseAction>,
) {
    let mut timer: ActionRef = actions.partition_mut().expect("Expected a timer action");
    timer.schedule(ctx, (), None);
}

/// Timer ReactionFn for timer actions
pub struct TimerFn(pub Option<Duration>);

impl From<TimerFn> for BoxedReactionFn {
    fn from(value: TimerFn) -> Self {
        Box::new(value)
    }
}

impl<'store> ReactionFn<'store> for TimerFn {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _state: &'store mut dyn BaseReactor,
        _ports: Refs<'store, dyn BasePort>,
        _ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let mut timer: ActionRef = actions.partition_mut().expect("Expected a timer action");

        if timer.is_present(ctx) {
            timer.schedule(ctx, (), self.0);
        } else {
            timer.schedule(ctx, (), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the ReactionAdapter struct.
    #[test]
    fn test_reaction_adapter() {
        struct TestReaction;

        impl FromRefs for TestReaction {
            type Marker<'store> = ();

            fn from_refs<'store>(
                _ports: Refs<'store, dyn BasePort>,
                _ports_mut: RefsMut<'store, dyn BasePort>,
                _actions: RefsMut<'store, dyn BaseAction>,
            ) -> Self::Marker<'store> {
            }
        }

        impl Trigger<()> for () {
            fn trigger(self, _ctx: &mut Context, _state: &mut ()) {}
        }

        let adapter = ReactionAdapter::<TestReaction, ()>::default();
        let _reaction = Reaction::new("dummy", adapter, None);
    }

    /// Test the FnAdapter struct.
    #[test]
    fn test_fn_wrapper() {
        let test_fn = |_: &mut Context,
                       _: &mut dyn BaseReactor,
                       _: Refs<'_, dyn BasePort>,
                       _: RefsMut<'_, dyn BasePort>,
                       _: RefsMut<'_, dyn BaseAction>| {};
        let _reaction = Reaction::new("dummy", test_fn, None);
    }
}
