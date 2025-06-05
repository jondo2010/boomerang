//! Support traits and impls for the `boomerang_derive` crate.

use slotmap::SecondaryMap;

use crate::{
    runtime, ActionTag, BoxedBuilderReactionFn, BuilderActionKey, BuilderError, BuilderPortKey,
    BuilderReactionKey, BuilderReactorKey, BuilderRuntimeParts, DeferedBuild, EnvBuilder,
    PhysicalActionKey, PortTag, PortType, ReactionBuilder, ReactorBuilderState, TimerActionKey,
    TimerSpec, TriggerMode, TypedActionKey, TypedPortKey,
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

impl<S: runtime::ReactorData> ReactorBuilderState<'_, S> {
    /// Add a new reaction to this reactor.
    pub fn add_derive_reaction<F>(
        &mut self,
        name: &str,
        reaction_builder_fn: F,
    ) -> DeriveReactionBuilder
    where
        F: FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        let reactor_key = self.key();
        self.env()
            .add_derive_reaction(name, reactor_key, reaction_builder_fn)
    }

    /// Add a new child reactor to this reactor.
    pub fn add_child_derive_reactor<R: Reactor>(
        &mut self,
        name: &str,
        state: R::State,
        is_enclave: bool,
    ) -> Result<R, BuilderError> {
        R::build(name, state, Some(self.key()), None, is_enclave, self.env())
    }

    /// Add multiple child reactors to this reactor.
    pub fn add_child_derive_reactors<R, const N: usize>(
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

impl EnvBuilder {
    /// Add a Reaction to a given Reactor
    pub fn add_derive_reaction<F>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        reaction_builder_fn: F,
    ) -> DeriveReactionBuilder
    where
        F: FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        DeriveReactionBuilder::new(name, reactor_key, Box::new(reaction_builder_fn), self)
    }
}

pub struct DeriveReactionBuilder<'a> {
    builder: ReactionBuilder,
    env: &'a mut EnvBuilder,
}

impl<'a> DeriveReactionBuilder<'a> {
    pub fn new(
        name: &str,
        reactor_key: BuilderReactorKey,
        reaction_fn: BoxedBuilderReactionFn,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: Some(name.into()),
                reactor_key,
                reaction_fn,
                action_relations: SecondaryMap::new(),
                port_relations: SecondaryMap::new(),
            },
            env,
        }
    }

    /// Declare a relation between this Reaction and the given Action
    pub fn add_action_relation(
        &mut self,
        key: BuilderActionKey,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        let action = &self.env.action_builders[key];
        if action.reactor_key() != self.builder.reactor_key {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Cannot add action '{}' to ReactionBuilder '{:?}', it must belong to the same reactor as the reaction",
                action.name(), &self.builder.name
            )));
        }
        self.builder.action_relations.insert(key, trigger_mode);
        Ok(())
    }

    /// Indicate how this Reaction interacts with the given Action
    ///
    /// There must be at least one trigger for each reaction.
    pub fn with_action(
        mut self,
        action_key: impl Into<BuilderActionKey>,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_action_relation(action_key.into(), trigger_mode)?;
        Ok(self)
    }

    /// Delcare a relation between this Reaction and the given Port
    ///
    /// Constraints on valid ports for each `trigger_mode`:
    ///  - For triggers: valid ports are input ports in this reactor, (or output ports of contained reactors).
    ///  - For uses: valid ports are input ports in this reactor, (or output ports of contained reactors).
    ///  - For effects: valid ports are output ports in this reactor, (or input ports of contained reactors).
    pub fn add_port_relation(
        &mut self,
        key: BuilderPortKey,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        let port_builder = &self.env.port_builders[key];
        let port_reactor_key = port_builder.get_reactor_key();
        let port_parent_reactor_key =
            self.env.reactor_builders[port_reactor_key].parent_reactor_key;

        // Validity checks:
        match port_builder.port_type() {
            PortType::Input => {
                // triggers and uses are valid for input ports on the same reactor
                if (trigger_mode.is_triggers() || trigger_mode.is_uses())
                    && port_reactor_key != self.builder.reactor_key
                {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {:?} cannot 'trigger on' or 'use' input port '{}', it must belong to the same reactor as the reaction",
                        self.builder.name(),
                        self.env.fqn_for(key, false).unwrap()
                    )));
                }
                // effects are valid for input ports on contained reactors
                if trigger_mode.is_effects()
                    && port_parent_reactor_key != Some(self.builder.reactor_key)
                {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {:?} cannot 'effect' input port '{}', it must belong to a contained reactor",
                        self.builder.name(),
                        port_builder.name()
                    )));
                }
            }
            PortType::Output => {
                // triggers and uses are valid for output ports on contained reactors
                if (trigger_mode.is_triggers() || trigger_mode.is_uses())
                    && port_parent_reactor_key != Some(self.builder.reactor_key)
                {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {:?} cannot 'trigger on' or 'use' output port '{}', it must belong to a contained reactor",
                        self.builder.name(),
                        port_builder.name()
                    )));
                }
                // effects are valid for output ports on the same reactor
                if trigger_mode.is_effects() && port_reactor_key != self.builder.reactor_key {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {:?} cannot 'effect' output port '{}', it must belong to the same reactor as the reaction",
                        self.builder.name(),
                        port_builder.name()
                    )));
                }
            }
        }
        self.builder.port_relations.insert(key, trigger_mode);
        Ok(())
    }

    /// Declare relations between this Reaction and the given Ports
    ///
    /// See [`Self::add_port_relation`] for constraints on valid ports for each `trigger_mode`.
    pub fn add_port_relations(
        &mut self,
        keys: impl IntoIterator<Item = BuilderPortKey>,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        for key in keys {
            self.add_port_relation(key, trigger_mode)?;
        }
        Ok(())
    }

    /// Indicate how this Reaction interacts with the given Port
    ///
    /// There must be at least one trigger for each reaction.
    pub fn with_port(
        mut self,
        port_key: impl Into<BuilderPortKey>,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_port_relation(port_key.into(), trigger_mode)?;
        Ok(self)
    }

    pub fn finish(self) -> Result<BuilderReactionKey, BuilderError> {
        let Self {
            builder: reaction_builder,
            env,
        } = self;

        // Ensure there is at least one trigger declared
        if !reaction_builder
            .action_relations
            .values()
            .any(|&mode| mode.is_triggers())
            && !reaction_builder
                .port_relations
                .values()
                .any(|&mode| mode.is_triggers())
        {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Reaction '{:?}' has no triggers defined",
                &reaction_builder.name
            )));
        }

        let reactor = &mut env.reactor_builders[reaction_builder.reactor_key];
        let reactions = &mut env.reaction_builders;

        let reaction_key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());
            reaction_builder
        });

        Ok(reaction_key)
    }
}

/// This builder trait is implemented for fields in the Reactor struct.
pub trait ReactorField: Sized {
    type Inner;

    /// Build a `ReactorBuilderState` for this Reaction
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
        parent.add_child_derive_reactor(name, state, is_enclave)
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
        parent.add_child_derive_reactors(name, state, is_enclave)
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
    /// Build a `ReactorBuilderState` for this Reaction
    fn build<'builder, S: runtime::ReactorData>(
        name: &str,
        reactor: &R,
        builder: &'builder mut ReactorBuilderState<S>,
    ) -> Result<DeriveReactionBuilder<'builder>, BuilderError>;
}

pub trait ReactionField {
    type Key;

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError>;
}

impl<T: runtime::ReactorData> ReactionField for runtime::ActionRef<'_, T> {
    //type Key = TypedActionKey<T>;
    type Key = BuilderActionKey;

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::AsyncActionRef<T> {
    type Key = BuilderActionKey;

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::InputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::InputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relations(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::OutputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut DeriveReactionBuilder,
        key: Self::Key,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::OutputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut DeriveReactionBuilder,
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
        builder: &mut DeriveReactionBuilder,
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
