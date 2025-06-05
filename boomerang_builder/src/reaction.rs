use std::fmt::Debug;

use super::{BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder};
use crate::{
    runtime, ActionTag, BuilderRuntimeParts, ParentReactorBuilder, PortTag, TimerActionKey,
    TypedActionKey, TypedPortKey,
};
use slotmap::SecondaryMap;

slotmap::new_key_type! {
    pub struct BuilderReactionKey;
}

impl petgraph::graph::GraphIndex for BuilderReactionKey {
    fn index(&self) -> usize {
        self.0.as_ffi() as usize
    }

    fn is_node_index() -> bool {
        true
    }
}

/// A boxed deferred Reaction builder function
pub type BoxedBuilderReactionFn = Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>;

pub struct ReactionBuilder {
    pub(super) name: Option<String>,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: BuilderReactorKey,
    /// The Reaction function
    pub(super) reaction_fn: BoxedBuilderReactionFn,
    /// Relations between this Reaction and Actions
    pub(super) action_relations: SecondaryMap<BuilderActionKey, TriggerMode>,
    /// Relations between this Reaction and Ports
    pub(super) port_relations: SecondaryMap<BuilderPortKey, TriggerMode>,
}

impl ParentReactorBuilder for ReactionBuilder {
    fn parent_reactor_key(&self) -> Option<BuilderReactorKey> {
        Some(self.reactor_key)
    }
}

impl Debug for ReactionBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactionBuilder")
            .field("name", &self.name)
            .field("reactor_key", &self.reactor_key)
            .field("reaction_fn", &"ReactionFn()")
            .field("action_relations", &self.action_relations)
            .field("port_relations", &self.port_relations)
            .finish()
    }
}

impl ReactionBuilder {
    /// Get the name of this Reaction
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone, Copy, Debug)]
/// Describes how an action is used by a reaction
pub enum TriggerMode {
    /// The action/port triggers the reaction, but is not provided as input
    TriggersOnly,
    /// The action/port triggers the reaction and is provided as input in the actions/ports arrays
    TriggersAndUses,

    /// The port triggers the reaction and is provided to the reaction as an input/output
    TriggersAndEffects,
    /// The port does not trigger the reaction, but is provided to the reaction as an input
    UsesOnly,
    /// The port does not trigger the reaction, but is provided to the reaction as an output
    EffectsOnly,
}

impl TriggerMode {
    pub fn is_triggers(&self) -> bool {
        matches!(
            self,
            TriggerMode::TriggersOnly
                | TriggerMode::TriggersAndUses
                | TriggerMode::TriggersAndEffects
        )
    }

    pub fn is_uses(&self) -> bool {
        matches!(self, TriggerMode::UsesOnly | TriggerMode::TriggersAndUses)
    }

    pub fn is_effects(&self) -> bool {
        matches!(
            self,
            TriggerMode::EffectsOnly | TriggerMode::TriggersAndEffects
        )
    }
}

pub trait PartialReactionBuilderField: runtime::ReactionRefsExtract {
    fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
        &self,
        builder: &mut PartialReactionBuilder<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    );
}

impl<T, Q, A> PartialReactionBuilderField for TypedPortKey<T, Q, A>
where
    T: runtime::ReactorData,
    Q: PortTag,
    TypedPortKey<T, Q, A>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
        &self,
        builder: &mut PartialReactionBuilder<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let port_key = BuilderPortKey::from(*self);
        builder.port_relations.insert(port_key, trigger_mode);
    }
}

impl<T, Q, A, const N: usize> PartialReactionBuilderField for [TypedPortKey<T, Q, A>; N]
where
    T: runtime::ReactorData,
    Q: PortTag,
    TypedPortKey<T, Q, A>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
        &self,
        builder: &mut PartialReactionBuilder<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        self.iter().for_each(|port| {
            port.extend_builder(builder, trigger_mode);
        })
    }
}

impl<T, Q> PartialReactionBuilderField for TypedActionKey<T, Q>
where
    T: runtime::ReactorData,
    Q: ActionTag,
    TypedActionKey<T, Q>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
        &self,
        builder: &mut PartialReactionBuilder<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let action_key = BuilderActionKey::from(*self);
        builder.action_relations.insert(action_key, trigger_mode);
    }
}

impl PartialReactionBuilderField for TimerActionKey {
    fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
        &self,
        builder: &mut PartialReactionBuilder<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let action_key = BuilderActionKey::from(*self);
        builder.action_relations.insert(action_key, trigger_mode);
    }
}

#[derive(Debug)]
pub struct PartialReactionBuilder<'a, S: runtime::ReactorData, Fields: Copy = (), ReactionFn = ()> {
    name: Option<String>,
    reaction_fn: ReactionFn,
    port_relations: slotmap::SecondaryMap<BuilderPortKey, TriggerMode>,
    action_relations: slotmap::SecondaryMap<BuilderActionKey, TriggerMode>,
    reactor_key: BuilderReactorKey,
    env: &'a mut EnvBuilder,
    phantom: std::marker::PhantomData<(S, Fields, ReactionFn)>,
}

impl<'a, S: runtime::ReactorData> PartialReactionBuilder<'a, S, (), ()> {
    pub fn new(
        name: Option<&str>,
        reactor_key: BuilderReactorKey,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            name: name.map(|s| s.to_string()),
            reaction_fn: (),
            port_relations: slotmap::SecondaryMap::new(),
            action_relations: slotmap::SecondaryMap::new(),
            reactor_key,
            env,
            phantom: std::marker::PhantomData,
        }
    }
}

macro_rules! impl_with_field {
    ($($Fn:ident),*) => {
        impl<'a, S, $($Fn,)*> PartialReactionBuilder<'a, S, ($($Fn,)*)>
        where
            S: runtime::ReactorData,
            $($Fn: runtime::ReactionRefsExtract,)*
        {
            /// Trigger this reaction on the startup of the reactor
            pub fn with_startup_trigger(self) -> PartialReactionBuilder<'a, S, ($($Fn,)* TypedActionKey,)> {
                let startup = self
                    .env
                    .get_reactor_builder(self.reactor_key)
                    .unwrap()
                    .get_startup_action();
                self.with_trigger(startup)
            }

            /// Trigger this reaction on the shutdown of the reactor
            pub fn with_shutdown_trigger(self) -> PartialReactionBuilder<'a, S, ($($Fn,)* TypedActionKey,)> {
                let shutdown = self
                    .env
                    .get_reactor_builder(self.reactor_key)
                    .unwrap()
                    .get_shutdown_action();
                self.with_trigger(shutdown)
            }

            /// Triggers can be input ports, output ports of contained reactors, timers, actions.
            /// There must be at least one trigger for each reaction.
            pub fn with_trigger<F>(mut self, field: F) -> PartialReactionBuilder<'a, S, ($($Fn,)* F,)>
            where
                F: PartialReactionBuilderField
            {
                field.extend_builder(&mut self, TriggerMode::TriggersAndUses);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    ..
                } = self;
                PartialReactionBuilder {
                    name,
                    reaction_fn: (),
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    phantom: std::marker::PhantomData,
                }
            }

            /// Use specifies input ports (or output ports of contained reactors) that do not trigger execution of
            /// the reaction but may be read by the reaction.
            pub fn with_use<F>(mut self, field: F) -> PartialReactionBuilder<'a, S, ($($Fn,)* F,)>
            where
                F: PartialReactionBuilderField
            {
                field.extend_builder(&mut self, TriggerMode::UsesOnly);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    ..
                } = self;
                PartialReactionBuilder {
                    name,
                    reaction_fn: (),
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    phantom: std::marker::PhantomData,
                }
            }

            /// Specify an effect field, which can be an output port, input port of contained reactors, or actions.
            pub fn with_effect<F>(mut self, field: F) -> PartialReactionBuilder<'a, S, ($($Fn,)* F,)>
            where
                F: PartialReactionBuilderField
            {
                field.extend_builder(&mut self, TriggerMode::EffectsOnly);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    ..
                } = self;
                PartialReactionBuilder {
                    name,
                    reaction_fn: (),
                    port_relations,
                    action_relations,
                    reactor_key,
                    env,
                    phantom: std::marker::PhantomData,
                }
            }
        }
    };
}

// Generate implementations for tuples of size 1 to 8
impl_with_field!();
impl_with_field!(F0);
impl_with_field!(F0, F1);
impl_with_field!(F0, F1, F2);
impl_with_field!(F0, F1, F2, F3);
impl_with_field!(F0, F1, F2, F3, F4);
impl_with_field!(F0, F1, F2, F3, F4, F5);
impl_with_field!(F0, F1, F2, F3, F4, F5, F6);
impl_with_field!(F0, F1, F2, F3, F4, F5, F6, F7);

impl<'a, S, Fields> PartialReactionBuilder<'a, S, Fields>
where
    S: runtime::ReactorData,
    Fields: runtime::ReactionRefsExtract,
{
    pub fn with_reaction_fn<F>(
        self,
        f: F,
    ) -> PartialReactionBuilder<'a, S, Fields, BoxedBuilderReactionFn>
    where
        F: for<'store> Fn(&mut runtime::Context, &mut S, Fields::Ref<'store>)
            + Send
            + Sync
            + 'static,
    {
        let Self {
            name,
            port_relations,
            action_relations,
            reactor_key,
            env,
            ..
        } = self;
        let reaction_fn: BoxedBuilderReactionFn =
            Box::new(|_: &BuilderRuntimeParts| -> runtime::BoxedReactionFn {
                Box::new(runtime::reaction::FnRefsAdapter::new(f))
            });
        PartialReactionBuilder {
            name,
            reaction_fn,
            port_relations,
            action_relations,
            reactor_key,
            env,
            phantom: std::marker::PhantomData,
        }
    }

    pub fn with_defered_reaction_fn<F>(
        self,
        f: F,
    ) -> PartialReactionBuilder<'a, S, Fields, BoxedBuilderReactionFn>
    where
        F: FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        let Self {
            name,
            port_relations,
            action_relations,
            reactor_key,
            env,
            ..
        } = self;
        PartialReactionBuilder {
            name,
            reaction_fn: Box::new(f),
            port_relations,
            action_relations,
            reactor_key,
            env,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<S, Fields> PartialReactionBuilder<'_, S, Fields, BoxedBuilderReactionFn>
where
    S: runtime::ReactorData,
    Fields: runtime::ReactionRefsExtract,
{
    /// Finish building the Reaction and add it to the Environment
    pub fn finish(self) -> Result<BuilderReactionKey, BuilderError> {
        let Self {
            name,
            port_relations,
            action_relations,
            reaction_fn,
            reactor_key,
            env,
            ..
        } = self;

        // Ensure there is at least one trigger declared
        if !action_relations.values().any(|&mode| mode.is_triggers())
            && !port_relations.values().any(|&mode| mode.is_triggers())
        {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Reaction '{name:?}' has no triggers defined"
            )));
        }

        let reactor = &mut env.reactor_builders[reactor_key];
        let reactions = &mut env.reaction_builders;

        let reaction_builder = super::ReactionBuilder {
            name,
            reactor_key,
            reaction_fn,
            action_relations,
            port_relations,
        };

        let reaction_key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());
            reaction_builder
        });

        Ok(reaction_key)
    }
}
