//! Support traits and impls for the `boomerang_derive` crate.

use crate::{
    runtime, ActionTag, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey,
    BuilderRuntimeParts, DeferedBuild, EnvBuilder, PhysicalActionKey, PortTag,
    ReactionBuilderState, ReactorBuilderState, TimerActionKey, TimerSpec, TriggerMode,
    TypedActionKey, TypedPortKey,
};

pub trait Reactor: Sized {
    type State: runtime::ReactorData;

    fn build(
        name: &str,
        state: Self::State,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        is_enclave: bool,
        env: &mut EnvBuilder,
    ) -> Result<Self, BuilderError>;

    fn iter(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self)
    }
}

impl<'a, S: runtime::ReactorData> ReactorBuilderState<'a, S> {
    /// Add a new reaction to this reactor.
    pub fn add_derive_reaction<F>(
        &mut self,
        name: &str,
        reaction_builder_fn: F,
    ) -> ReactionBuilderState
    where
        F: FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        let reactor_key = self.key();
        self.env()
            .add_reaction(name, reactor_key, reaction_builder_fn)
    }

    /// Add a new child reactor to this reactor.
    pub fn add_child_reactor<R: Reactor>(
        &mut self,
        name: &str,
        state: R::State,
        is_enclave: bool,
    ) -> Result<R, BuilderError> {
        R::build(name, state, Some(self.key()), None, is_enclave, self.env())
    }

    /// Add multiple child reactors to this reactor.
    pub fn add_child_reactors<R, const N: usize>(
        &mut self,
        name: &str,
        state: R::State,
        is_enclave: bool,
    ) -> Result<[R; N], BuilderError>
    where
        R: Reactor,
        R::State: Clone,
    {
        let reactors = (0..N)
            .map(|i| {
                R::build(
                    name,
                    state.clone(),
                    Some(self.key()),
                    Some(runtime::BankInfo { idx: i, total: N }),
                    is_enclave,
                    self.env(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        reactors
            .try_into()
            .map_err(|_| BuilderError::InternalError("Error converting Vec to array".to_owned()))
    }
}

/// This builder trait is implemented for fields in the Reactor struct.
pub trait ReactorField: Sized {
    type Inner;

    /// Build a `ReactionBuilderState` for this Reaction
    fn build<S: runtime::ReactorData>(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError>;
}

impl<R: Reactor> ReactorField for R {
    type Inner = (R::State, bool);

    fn build<S: runtime::ReactorData>(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        let (state, is_enclave) = inner;
        parent.add_child_reactor(name, state, is_enclave)
    }
}

/// NOTE: `R::State: Clone` is required because state is cloned for each child reactor.
impl<R, const N: usize> ReactorField for [R; N]
where
    R: Reactor,
    R::State: Clone,
{
    type Inner = (R::State, bool);

    fn build<S: runtime::ReactorData>(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        let (state, is_enclave) = inner;
        parent.add_child_reactors(name, state, is_enclave)
    }
}

impl<T: runtime::ReactorData, Q: PortTag> ReactorField for TypedPortKey<T, Q> {
    type Inner = ();

    fn build<S: runtime::ReactorData>(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        parent.add_port::<T, Q>(name, None)
    }
}

impl<T: runtime::ReactorData, Q: PortTag, const N: usize> ReactorField for [TypedPortKey<T, Q>; N] {
    type Inner = ();

    fn build<S: runtime::ReactorData>(
        name: &str,
        _inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        parent.add_ports::<T, Q, N>(name)
    }
}

impl ReactorField for TimerActionKey {
    type Inner = TimerSpec;

    fn build<S: runtime::ReactorData>(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        parent.add_timer(name, inner)
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> ReactorField for TypedActionKey<T, Q> {
    type Inner = Option<runtime::Duration>;

    fn build<S: runtime::ReactorData>(
        name: &str,
        min_delay: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        parent.add_action(name, min_delay)
    }
}

impl ReactorField for PhysicalActionKey {
    type Inner = Option<runtime::Duration>;

    fn build<S: runtime::ReactorData>(
        name: &str,
        inner: Self::Inner,
        parent: &'_ mut ReactorBuilderState<S>,
    ) -> Result<Self, BuilderError> {
        parent
            .add_physical_action::<()>(name, inner)
            .map(Into::into)
    }
}

/// The Reaction trait should be automatically derived for each Reaction struct.
pub trait Reaction<R: Reactor> {
    /// Build a `ReactionBuilderState` for this Reaction
    fn build<'builder, S: runtime::ReactorData>(
        name: &str,
        reactor: &R,
        builder: &'builder mut ReactorBuilderState<S>,
    ) -> Result<ReactionBuilderState<'builder>, BuilderError>;
}

pub trait ReactionField {
    type Key;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError>;
}

impl<T: runtime::ReactorData> ReactionField for runtime::ActionRef<'_, T> {
    //type Key = TypedActionKey<T>;
    type Key = BuilderActionKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::AsyncActionRef<T> {
    type Key = BuilderActionKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::InputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::InputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relations(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::OutputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::OutputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relations(key, trigger_mode)
    }
}

pub struct PortOrActionTrigger;
pub enum PortOrActionTriggerKey {
    Port(BuilderPortKey),
    Action(BuilderActionKey),
}
impl From<BuilderPortKey> for PortOrActionTriggerKey {
    fn from(key: BuilderPortKey) -> Self {
        Self::Port(key)
    }
}
impl From<BuilderActionKey> for PortOrActionTriggerKey {
    fn from(key: BuilderActionKey) -> Self {
        Self::Action(key)
    }
}

impl ReactionField for PortOrActionTrigger {
    type Key = PortOrActionTriggerKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        match key {
            PortOrActionTriggerKey::Port(port_key) => {
                builder.add_port_relation(port_key, trigger_mode)
            }
            PortOrActionTriggerKey::Action(action_key) => {
                builder.add_action_relation(action_key, trigger_mode)
            }
        }
    }
}

/// Adapter struct for implementing the `ReactionFn` trait for a Reaction struct.
///
/// The `ReactionAdapter` struct is used to convert a Reaction struct to a `Box<dyn ReactionFn>`. This is the mechanism
/// used by the derive-generated code to implement the Reaction trigger interface.
pub struct ReactionAdapter<R, S>(std::marker::PhantomData<fn() -> (R, S)>);

impl<Reaction, State> Default for ReactionAdapter<Reaction, State> {
    fn default() -> Self {
        Self(Default::default())
    }
}

/// The `Trigger` trait should be implemented by the user for each Reaction struct.
///
/// Type parameter `S` is the state type of the Reactor.
pub trait Trigger<S: runtime::ReactorData> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut S);
}

impl<'store, Reaction, S> runtime::ReactionFn<'store> for ReactionAdapter<Reaction, S>
where
    Reaction: runtime::FromRefs,
    Reaction::Marker<'store>: 'store + Trigger<S>,
    S: runtime::ReactorData,
{
    #[inline(always)]
    fn trigger(
        &mut self,
        ctx: &'store mut runtime::Context,
        reactor: &'store mut dyn runtime::BaseReactor,
        refs: runtime::ReactionRefs<'store>,
    ) {
        let reactor: &mut runtime::Reactor<S> = reactor
            .downcast_mut()
            .expect("Unable to downcast reactor state");

        let reaction = Reaction::from_refs(refs);
        reaction.trigger(ctx, &mut reactor.state);
    }
}

impl<Reaction, State> DeferedBuild for ReactionAdapter<Reaction, State>
where
    Reaction: runtime::FromRefs + 'static,
    for<'store> Reaction::Marker<'store>: 'store + Trigger<State>,
    State: runtime::ReactorData,
{
    type Output = runtime::BoxedReactionFn;
    fn defer(self) -> impl FnOnce(&BuilderRuntimeParts) -> Self::Output + 'static {
        move |_| runtime::BoxedReactionFn::from(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the ReactionAdapter struct.
    #[test]
    fn test_reaction_adapter() {
        struct TestReaction;

        impl runtime::FromRefs for TestReaction {
            type Marker<'store> = ();

            fn from_refs(_: runtime::ReactionRefs<'_>) -> Self::Marker<'_> {}
        }

        #[allow(non_local_definitions)]
        impl Trigger<()> for () {
            fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {}
        }

        let adapter = ReactionAdapter::<TestReaction, ()>::default();
        let _reaction = runtime::Reaction::new("dummy", adapter, None);
    }
}
