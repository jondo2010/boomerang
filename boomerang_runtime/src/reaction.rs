use std::{fmt::Debug, sync::RwLock};

use crate::{
    key_set::KeySet, ActionCommon, ActionRef, AsyncActionRef, BaseReactor, CommonContext, Context,
    Duration, InputRef, OutputRef, ReactionRefs, ReactionRefsExtract, ReactorData, SendContext,
    Tag,
};

tinymap::key_type! { pub ReactionKey }

pub type ReactionSet = KeySet<ReactionKey>;

pub trait ReactionFn<'store>: Send + Sync {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        reactor: &'store mut dyn BaseReactor,
        refs: ReactionRefs<'store>,
    );
}

pub type BoxedReactionFn = Box<dyn for<'store> ReactionFn<'store> + Send + Sync>;

pub type BoxedHandlerFn = Box<dyn Fn() + Send + Sync>;

/// Conversion trait for creating a Reaction struct from port and action references.
///
/// This trait is typically implemented by macro-generated code.
pub trait FromRefs {
    type Marker<'store>;
    fn from_refs(refs: ReactionRefs<'_>) -> Self::Marker<'_>;
}

/// We implement [`ReactionFn`] for any `Fn` that straightforwardly matches the signature.
impl<F> ReactionFn<'_> for F
where
    F: for<'any> Fn(&mut Context, &mut dyn BaseReactor, ReactionRefs<'any>) + Send + Sync + 'static,
{
    fn trigger(&mut self, ctx: &mut Context, reactor: &mut dyn BaseReactor, refs: ReactionRefs) {
        (self)(ctx, reactor, refs)
    }
}

/// Anything that implements [`ReactionFn`] can be converted to a [`BoxedReactionFn`].
impl<T> From<T> for BoxedReactionFn
where
    T: for<'any> ReactionFn<'any> + 'static,
{
    fn from(value: T) -> Self {
        Box::new(value)
    }
}

/// Adapter struct for implementing the [`ReactionFn`] trait for a generic FnMut with fields that implement `ReactionRefsExtract`.
pub struct FnRefsAdapter<S, Fields, F>
where
    S: ReactorData,
    Fields: ReactionRefsExtract + Send + Sync,
    F: for<'any> Fn(&mut Context, &mut S, Fields::Ref<'any>) + Send + Sync + 'static,
{
    fields: Fields,
    f: F,
    _phantom: std::marker::PhantomData<fn() -> S>,
}

impl<S, Fields, F> FnRefsAdapter<S, Fields, F>
where
    S: ReactorData,
    Fields: ReactionRefsExtract + Send + Sync,
    F: for<'any> Fn(&mut Context, &mut S, Fields::Ref<'any>) + Send + Sync + 'static,
{
    pub fn new(fields: Fields, f: F) -> Self {
        Self {
            fields,
            f,
            _phantom: Default::default(),
        }
    }
}

impl<S, Fields, F> ReactionFn<'_> for FnRefsAdapter<S, Fields, F>
where
    S: ReactorData,
    Fields: ReactionRefsExtract + Send + Sync,
    F: for<'any> Fn(&mut Context, &mut S, Fields::Ref<'any>) + Send + Sync + 'static,
{
    fn trigger(
        &mut self,
        ctx: &mut Context,
        reactor: &mut dyn BaseReactor,
        mut refs: ReactionRefs,
    ) {
        let state = reactor.get_state_mut::<S>().expect("state");

        let fields = match self.fields.extract(&mut refs) {
            Ok(fields) => fields,
            Err(error) => {
                let fields_type = std::any::type_name::<Fields>();
                panic!("Failed to extract reaction references ({fields_type}): {error}")
            }
        };

        (self.f)(ctx, state, fields)
    }
}

/// Placement-neutral delivery time calculated by a cross-partition sender reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterPartitionEventTime {
    /// Deliver at this complete logical tag.
    Logical(Tag),
    /// Deliver as a physical event with an optional additional delay.
    Physical(Option<Duration>),
}

/// Backend for one cross-partition sender reaction.
///
/// The reaction owns port extraction and delivery-time calculation. Implementations own only the
/// placement-specific dispatch and error policy; they must not recalculate logical tags.
pub trait InterPartitionEventSink<T: ReactorData>: Send + Sync + 'static {
    fn send(&self, time: InterPartitionEventTime, target: &AsyncActionRef<T>, value: &T);
}

/// In-process cross-partition event sink backed by a scheduler send context.
pub struct InProcessInterPartitionEventSink {
    remote_context: SendContext,
}

impl InProcessInterPartitionEventSink {
    pub fn new(remote_context: SendContext) -> Self {
        Self { remote_context }
    }
}

impl<T: ReactorData + Clone> InterPartitionEventSink<T> for InProcessInterPartitionEventSink {
    fn send(&self, time: InterPartitionEventTime, target: &AsyncActionRef<T>, value: &T) {
        match time {
            InterPartitionEventTime::Logical(tag) => {
                self.remote_context
                    .schedule_external(crate::event::AsyncEvent::Logical {
                        tag,
                        key: target.key(),
                        value: Box::new(value.clone()),
                    });
            }
            InterPartitionEventTime::Physical(delay) => {
                self.remote_context
                    .schedule_action_async(target, value.clone(), delay);
            }
        }
    }
}

/// Common reaction for sending a value across any runtime partition boundary.
///
/// A zero logical minimum delay preserves the current complete tag, while a positive delay
/// advances logical time and resets the microstep. The selected sink owns only dispatch and error
/// policy.
pub struct InterPartitionSenderReactionFn<T: ReactorData + Clone> {
    target_action_ref: AsyncActionRef<T>,
    sink: Box<dyn InterPartitionEventSink<T>>,
    physical_delay: Option<Duration>,
}

impl<T: ReactorData + Clone> InterPartitionSenderReactionFn<T> {
    pub fn new(
        target_action_ref: AsyncActionRef<T>,
        sink: Box<dyn InterPartitionEventSink<T>>,
        physical_delay: Option<Duration>,
    ) -> Self {
        Self {
            target_action_ref,
            sink,
            physical_delay,
        }
    }
}

impl<'store, T: ReactorData + Clone> ReactionFn<'store> for InterPartitionSenderReactionFn<T> {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _state: &'store mut dyn BaseReactor,
        refs: ReactionRefs<'store>,
    ) {
        let port: InputRef<T> = match refs.ports.partition() {
            Ok(port) => port,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure ports");
                return;
            }
        };
        let Some(value) = (*port).as_ref() else {
            tracing::warn!("Port is empty, skipping inter-partition event send");
            return;
        };

        let time = if self.target_action_ref.is_logical() {
            let current_tag = ctx.get_tag();
            let delay = self.target_action_ref.min_delay();
            InterPartitionEventTime::Logical(if delay.is_zero() {
                current_tag
            } else {
                current_tag.delay(delay)
            })
        } else {
            InterPartitionEventTime::Physical(self.physical_delay)
        };

        self.sink.send(time, &self.target_action_ref, value);
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
        refs: ReactionRefs<'store>,
    ) {
        let mut action: ActionRef<T> = match refs.actions.partition_mut() {
            Ok(action) => action,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure actions");
                return;
            }
        };

        let port: InputRef<T> = match refs.ports.partition() {
            Ok(port) => port,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure ports");
                return;
            }
        };
        if let Some(value) = (*port).as_ref() {
            ctx.schedule_action(&mut action, value.clone(), None);
        } else {
            tracing::warn!("Port is empty, skipping action send");
        }
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
        refs: ReactionRefs<'store>,
    ) {
        let mut action: ActionRef<T> = match refs.actions.partition_mut() {
            Ok(action) => action,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure actions");
                return;
            }
        };

        let mut port: OutputRef<T> = match refs.ports_mut.partition_mut() {
            Ok(port) => port,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure ports");
                return;
            }
        };
        if let Some(value) = ctx.get_action_value(&mut action) {
            *port = Some(value.clone());
        } else {
            tracing::warn!("Action is empty, skipping port update");
        }
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
pub fn empty_reaction(_ctx: &mut Context, _reactor: &mut dyn BaseReactor, _refs: ReactionRefs<'_>) {
}

/// Timer ReactionFn for timer actions
pub struct TimerFn(pub Option<Duration>);

impl<'store> ReactionFn<'store> for TimerFn {
    fn trigger(
        &mut self,
        ctx: &'store mut Context,
        _reactor: &'store mut dyn BaseReactor,
        refs: ReactionRefs<'store>,
    ) {
        let mut timer: ActionRef = match refs.actions.partition_mut() {
            Ok(timer) => timer,
            Err(error) => {
                tracing::error!(?error, "Failed to destructure timer action");
                return;
            }
        };
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
/// let closure = reaction_closure!(ctx, _reactor, _refs => {
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
    ( $ctx:ident, $reactor:ident, $refs:ident => $body:block ) => {{
        Box::new(
            move |$ctx: &mut $crate::Context,
                  $reactor: &mut dyn $crate::BaseReactor,
                  $refs: $crate::ReactionRefs<'_>| { $body },
        )
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the FnAdapter struct.
    #[test]
    fn test_fn_wrapper() {
        let test_fn = |_: &mut Context, _: &mut dyn BaseReactor, _: ReactionRefs<'_>| {};
        let _reaction = Reaction::new("dummy", test_fn, None);
    }

    #[test]
    fn test_reaction_closure() {
        let _closure = reaction_closure!(ctx, _state, _refs => {
            ctx.get_elapsed_logical_time();
        });
    }
}
