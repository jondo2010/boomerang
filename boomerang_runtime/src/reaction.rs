use std::{fmt::Debug, sync::RwLock};

use crate::{
    key_set::KeySet,
    refs::{Refs, RefsMut},
    ActionCommon, ActionRef, AsyncActionRef, BaseAction, BasePort, BaseReactor, CommonContext,
    Context, Duration, InputRef, OutputRef, Reactor, ReactorData, SendContext,
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

/// Special type implementing [`ReactionFn`] for sending data to an another Enclave.
///
/// This is used to implement connections between Ports in different Enclaves.
/// The Reaction using this function should be 'triggered' by the sending side port only.
pub struct EnclaveSenderReactionFn<T: ReactorData + Clone> {
    /// A clone of the sender side context
    remote_context: SendContext,
    /// The remote action that we're sending data to.
    remote_action_ref: AsyncActionRef<T>,
    /// The optional delay to apply to the event.
    delay: Option<Duration>,
}

impl<T: ReactorData + Clone> From<EnclaveSenderReactionFn<T>> for BoxedReactionFn {
    fn from(value: EnclaveSenderReactionFn<T>) -> Self {
        Box::new(value)
    }
}

impl<T: ReactorData + Clone> EnclaveSenderReactionFn<T> {
    pub fn new(
        remote_context: SendContext,
        remote_action_ref: AsyncActionRef<T>,
        delay: Option<Duration>,
    ) -> Self {
        Self {
            remote_context,
            remote_action_ref,
            delay,
        }
    }
}

impl<'store, T: ReactorData + Clone> ReactionFn<'store> for EnclaveSenderReactionFn<T> {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _state: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        _ports_mut: RefsMut<'store, dyn BasePort>,
        _actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let port: InputRef<T> = ports.partition().expect("Expected a port");
        if let Some(value) = (*port).as_ref() {
            //TODO something nicer here
            if self.remote_action_ref.is_logical() {
                let current_tag = ctx.get_tag();

                let delay = self.remote_action_ref.min_delay();

                let tag = if delay.is_zero() {
                    current_tag
                } else {
                    current_tag.delay(delay)
                };

                self.remote_context
                    .schedule_external(crate::event::AsyncEvent::Logical {
                        tag,
                        key: self.remote_action_ref.key(),
                        value: Box::new(value.clone()),
                    });
            } else {
                self.remote_context.schedule_action_async(
                    &self.remote_action_ref,
                    value.clone(),
                    self.delay,
                );
            }
        } else {
            tracing::warn!("Port is empty, skipping event send");
        }
    }
}

/// Special type implementing [`ReactionFn`] for sending data across a non-trivial connection (enclave, physical or delayed).
pub struct ConnectionSenderReactionFn<T: ReactorData + Clone> {
    /// Marker for the type of data being sent.
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: ReactorData + Clone> Default for ConnectionSenderReactionFn<T> {
    fn default() -> Self {
        Self {
            _marker: Default::default(),
        }
    }
}

impl<'store, T: ReactorData + Clone> ReactionFn<'store> for ConnectionSenderReactionFn<T> {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _reactor: &'store mut dyn BaseReactor,
        ports: Refs<'store, dyn BasePort>,
        _ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let mut action: ActionRef<T> = actions.partition_mut().unwrap();
        let port: InputRef<T> = ports.partition().unwrap();
        if let Some(value) = (*port).as_ref() {
            ctx.schedule_action(&mut action, value.clone(), None);
        } else {
            tracing::warn!("Port is empty, skipping action send");
        }
    }
}

impl<T: ReactorData + Clone> From<ConnectionSenderReactionFn<T>> for BoxedReactionFn {
    fn from(value: ConnectionSenderReactionFn<T>) -> Self {
        Box::new(value)
    }
}

/// Special type implementing [`ReactionFn`] for receiving data across a non-trivial connection (enclave, physical or delayed).
///
/// This is used to implement connections between Ports in different Enclaves.
pub struct ConnectionReceiverReactionFn<T: ReactorData + Clone> {
    /// Marker for the type of data being sent.
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: ReactorData + Clone> Default for ConnectionReceiverReactionFn<T> {
    fn default() -> Self {
        Self {
            _marker: Default::default(),
        }
    }
}

impl<'store, T: ReactorData + Clone> ReactionFn<'store> for ConnectionReceiverReactionFn<T> {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _reactor: &'store mut dyn BaseReactor,
        _ports: Refs<'store, dyn BasePort>,
        ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let mut action: ActionRef<T> = actions.partition_mut().unwrap();
        let mut port: OutputRef<T> = ports_mut.partition_mut().unwrap();
        if let Some(value) = ctx.get_action_value(&mut action) {
            *port = Some(value.clone());
        } else {
            tracing::warn!("Action is empty, skipping port update");
        }
    }
}

impl<T: ReactorData + Clone> From<ConnectionReceiverReactionFn<T>> for BoxedReactionFn {
    fn from(value: ConnectionReceiverReactionFn<T>) -> Self {
        Box::new(value)
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

/// An empty reaction function that does nothing. Used by the [`reaction_closure`] macro.
pub fn empty_reaction(
    _ctx: &mut Context,
    _reactor: &mut dyn BaseReactor,
    _ref_ports: Refs<dyn BasePort>,
    _mut_ports: RefsMut<dyn BasePort>,
    _actions: RefsMut<dyn BaseAction>,
) {
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
        _reactor: &'store mut dyn BaseReactor,
        _ports: Refs<'store, dyn BasePort>,
        _ports_mut: RefsMut<'store, dyn BasePort>,
        actions: RefsMut<'store, dyn BaseAction>,
    ) {
        let mut timer: ActionRef = actions.partition_mut().expect("Expected a timer action");
        ctx.schedule_action(&mut timer, (), self.0);
    }
}

/// A macro to create a new reaction closure.
///
/// The macro takes a block of code and creates a new reaction closure from it.
///
/// # Example
///
/// ```rust
/// # use boomerang_runtime::reaction_closure;
/// let closure = reaction_closure!(ctx, _reactor, _ref_ports, _mut_ports, _actions => {
///    ctx.get_elapsed_logical_time();
/// });
/// ```
#[macro_export]
macro_rules! reaction_closure {
    // empty closure case
    () => {
        Box::new($crate::reaction::empty_reaction)
    };
    // closure with body
    ( $ctx:ident, $reactor:ident, $ref_ports:ident, $mut_ports:ident, $actions:ident => $body:block ) => {{
        Box::new(
            move |$ctx: &mut $crate::Context,
                  $reactor: &mut dyn $crate::BaseReactor,
                  $ref_ports: $crate::Refs<dyn $crate::BasePort>,
                  $mut_ports: $crate::RefsMut<dyn $crate::BasePort>,
                  $actions: $crate::RefsMut<dyn $crate::BaseAction>| { $body },
        )
    }};
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

    #[test]
    fn test_reaction_closure() {
        let _closure = reaction_closure!(ctx, _state, _ref_ports, _mut_ports, _actions => {
            ctx.get_elapsed_logical_time();
        });
    }
}
