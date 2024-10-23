use std::{
    fmt::{Debug, Display},
    sync::RwLock,
    time::Duration,
};

use crate::{
    data::SerdeDataObj,
    key_set::KeySet,
    refs::{Refs, RefsMut},
    Action, ActionRef, BasePort, BaseReactor, Context, Reactor, ReactorData,
};

tinymap::key_type!(pub ReactionKey);

pub type ReactionSet = KeySet<ReactionKey>;

pub trait ReactionFn<'store>: SerdeDataObj + Send + Sync {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        reactor: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    );

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

impl Debug for BoxedReactionFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BoxedReactionFn<{}>", self.type_name())
    }
}

/// An empty reaction function that does nothing.
pub fn empty_reaction(
    _ctx: &mut Context,
    _reactor: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    _actions: RefsMut<Action>,
) {
}

pub type BoxedReactionFn = Box<dyn for<'store> ReactionFn<'store>>;

pub type BoxedHandlerFn = Box<dyn Fn()>;

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
/// Type parameter `R` is the state data type of the Reactor.
pub trait Trigger<R: ReactorData> {
    fn trigger(self, ctx: &mut Context, state: &mut R);
}

/// Adapter struct for implementing the `ReactionFn` trait for a Reaction struct.
///
/// The `ReactionAdapter` struct is used to convert a Reaction struct to a `Box<dyn ReactionFn>`. This is the mechanism
/// used by the derive-generated code to implement the Reaction trigger interface.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReactionAdapter<Reaction, T>(std::marker::PhantomData<fn() -> (Reaction, T)>);

#[cfg(feature = "serde")]
impl<Reaction, T: ReactorData> serde_flexitos::id::IdObj for ReactionAdapter<Reaction, T> {
    fn id(&self) -> serde_flexitos::id::Ident<'static> {
        serde_flexitos::id::Ident::I1("ReactionAdapter").extend(T::ID)
    }
}

impl<Reaction, T> Default for ReactionAdapter<Reaction, T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<Reaction, T> From<ReactionAdapter<Reaction, T>> for Box<dyn for<'store> ReactionFn<'store>>
where
    Reaction: FromRefs + 'static,
    for<'store> Reaction::Marker<'store>: 'store + Trigger<T>,
    T: ReactorData,
{
    fn from(adapter: ReactionAdapter<Reaction, T>) -> Self {
        Box::new(adapter)
    }
}

impl<'store, Reaction, T> ReactionFn<'store> for ReactionAdapter<Reaction, T>
where
    Reaction: FromRefs,
    Reaction::Marker<'store>: 'store + Trigger<T>,
    T: ReactorData,
{
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        reactor: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
    ) {
        let reactor: &mut Reactor<T> = reactor
            .downcast_mut()
            .expect("Unable to downcast reactor state");

        let reaction = Reaction::from_refs(ports, ports_mut, actions);
        reaction.trigger(ctx, &mut reactor.state);
    }
}

/// Wrapper struct for implementing the `ReactionFn` trait for a generic FnMut function.
///
/// An `FnWrapper` can be created from a closure or function pointer and then converted to a `Box<dyn ReactionFn>`.
pub struct FnWrapper<F>(F);

#[cfg(feature = "serde")]
impl<F> serde::Serialize for FnWrapper<F> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_unit_struct("FnWrapper")
    }
}

impl<F> From<F> for Box<dyn for<'store> ReactionFn<'store>>
where
    F: for<'store> Fn(
            &'store mut Context,
            &'store mut dyn BaseReactor,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, Action>,
        ) + Send
        + Sync
        + 'static,
{
    fn from(wrapper: F) -> Self {
        Box::new(FnWrapper(wrapper))
    }
}

#[cfg(feature = "serde")]
impl<'store, F> serde_flexitos::id::IdObj for FnWrapper<F>
where
    F: Fn(
            &'store mut Context,
            &'store mut dyn BaseReactor,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, Action>,
        ) + Send
        + Sync,
{
    fn id(&self) -> serde_flexitos::id::Ident<'static> {
        serde_flexitos::id::Ident::I2("FnWrapper", std::any::type_name::<F>())
    }
}

impl<'store, F> ReactionFn<'store> for FnWrapper<F>
where
    F: Fn(
            &'store mut Context,
            &'store mut dyn BaseReactor,
            Refs<'store, dyn BasePort>,
            RefsMut<'store, dyn BasePort>,
            RefsMut<'store, Action>,
        ) + Send
        + Sync,
{
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        state: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, Action>,
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

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct Reaction {
    name: String,
    /// Reaction closure
    pub(crate) body: BoxedReactionFn,
    /// Local deadline relative to the time stamp for invocation of the reaction.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) deadline: Option<Deadline>,
}

impl Display for Reaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.deadline.is_some() {
            todo!("support for deadline")
        }
        write!(
            f,
            "runtime::Reaction::new(\"{name}\", Box::new({ty}), None)",
            name = self.name,
            ty = self.reaction_type_name(),
        )
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

    pub fn reaction_type_name(&self) -> &'static str {
        self.body.type_name()
    }
}

/// Utility startup function for a timer action
pub fn timer_startup_fn(
    ctx: &mut Context,
    _state: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    actions: RefsMut<Action>,
) {
    let mut timer: ActionRef = actions.partition_mut().expect("Expected a timer action");
    ctx.schedule_action(&mut timer, None, None);
}

#[cfg(feature = "serde")]
crate::register_reaction_fn!(FnWrapper<timer_startup_fn>);

/// Utility reset function for a timer action
pub fn timer_reset_fn(
    ctx: &mut Context,
    _state: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    actions: RefsMut<Action>,
) {
    let mut timer: ActionRef = actions.partition_mut().expect("Expected a timer action");
    ctx.schedule_action(&mut timer, None, None);
}

#[cfg(feature = "serde")]
crate::register_reaction_fn!(FnWrapper<timer_reset_fn>);

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
                _actions: RefsMut<'store, Action>,
            ) -> Self::Marker<'store> {
            }
        }

        impl Trigger<()> for () {
            fn trigger(self, _ctx: &mut Context, _state: &mut ()) {}
        }

        let adapter = ReactionAdapter::<TestReaction, ()>::default();
        let _reaction = Reaction::new("dummy", adapter, None);
    }

    /// Test the FnWrapper struct.
    #[test]
    fn test_fn_wrapper() {
        let test_fn = |_: &mut Context,
                       _: &mut dyn BaseReactor,
                       _: Refs<'_, dyn BasePort>,
                       _: RefsMut<'_, dyn BasePort>,
                       _: RefsMut<'_, Action>| {};
        let _reaction = Reaction::new("dummy", test_fn, None);
    }
}
