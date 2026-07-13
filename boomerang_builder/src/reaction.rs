use std::fmt::Debug;

use super::{
    Assembly, AssemblyActionKey, AssemblyModeKey, AssemblyPortKey, AssemblyReactorKey, BuilderError,
};
use crate::{
    runtime, ActionTag, BuilderRuntimeParts, ParentReactorSpec, PortBank, PortTag, TimerActionKey,
    TypedActionKey, TypedPortKey,
};
use slotmap::SecondaryMap;
use variadics_please::all_tuples;

slotmap::new_key_type! {
    pub struct AssemblyReactionKey;
}

impl petgraph::graph::GraphIndex for AssemblyReactionKey {
    fn index(&self) -> usize {
        self.0.as_ffi() as usize
    }

    fn is_node_index() -> bool {
        true
    }
}

/// A deferred factory for a runtime reaction function.
pub type DeferredReactionFactory =
    Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>;

pub struct ReactionSpec {
    pub(super) name: Option<String>,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: AssemblyReactorKey,
    /// The Reaction function
    pub(super) reaction_fn: DeferredReactionFactory,
    /// Modes in which this reaction is enabled
    pub(super) enabled_modes: Option<Vec<AssemblyModeKey>>,
    /// Enclosing mode scope, if this reaction was declared inside a mode.
    pub(super) scope_mode: Option<AssemblyModeKey>,
    /// Declared typed mode effects for this reaction
    pub(super) mode_effects: Vec<BuilderModeEffect>,
    /// Whether this reaction is triggered by mode reset entry.
    pub(super) reset_trigger: bool,
    /// Relations between this Reaction and Actions
    pub(super) action_relations: SecondaryMap<AssemblyActionKey, TriggerMode>,
    /// Actions in the order they were declared on the builder
    pub(super) action_order: Vec<AssemblyActionKey>,
    /// Relations between this Reaction and Ports
    pub(super) port_relations: SecondaryMap<AssemblyPortKey, TriggerMode>,
    /// Ports in the order they were declared on the builder
    pub(super) port_order: Vec<AssemblyPortKey>,
}

impl ReactionSpec {
    /// Create a new ReactionSpec
    pub fn new<S: Into<String>>(
        name: Option<S>,
        parent_key: AssemblyReactorKey,
        reaction_fn: Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>,
    ) -> Self {
        ReactionSpec {
            name: name.map(|s| s.into()),
            reactor_key: parent_key,
            reaction_fn,
            enabled_modes: None,
            scope_mode: None,
            mode_effects: Vec::new(),
            reset_trigger: false,
            action_relations: SecondaryMap::new(),
            action_order: Vec::new(),
            port_relations: SecondaryMap::new(),
            port_order: Vec::new(),
        }
    }
}

impl ParentReactorSpec for ReactionSpec {
    fn parent_reactor_key(&self) -> Option<AssemblyReactorKey> {
        Some(self.reactor_key)
    }
}

impl Debug for ReactionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactionSpec")
            .field("name", &self.name)
            .field("reactor_key", &self.reactor_key)
            .field("reaction_fn", &"ReactionFn()")
            .field("enabled_modes", &self.enabled_modes)
            .field("scope_mode", &self.scope_mode)
            .field("mode_effects", &self.mode_effects)
            .field("reset_trigger", &self.reset_trigger)
            .field("action_relations", &self.action_relations)
            .field("port_relations", &self.port_relations)
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuilderModeEffect {
    target: AssemblyModeKey,
    runtime_target: Option<runtime::ModeKey>,
    transition: runtime::TransitionKind,
}

impl BuilderModeEffect {
    pub(crate) fn new(target: AssemblyModeKey, transition: runtime::TransitionKind) -> Self {
        Self {
            target,
            runtime_target: None,
            transition,
        }
    }

    pub fn target(&self) -> AssemblyModeKey {
        self.target
    }

    pub fn transition(&self) -> runtime::TransitionKind {
        self.transition
    }

    pub fn with_transition(mut self, transition: runtime::TransitionKind) -> Self {
        self.transition = transition;
        self
    }
}

#[doc(hidden)]
pub trait ResolveModeEffects {
    fn resolve_mode_effects(&mut self, runtime_parts: &BuilderRuntimeParts);
}

impl ResolveModeEffects for () {
    fn resolve_mode_effects(&mut self, _runtime_parts: &BuilderRuntimeParts) {}
}

impl ResolveModeEffects for BuilderModeEffect {
    fn resolve_mode_effects(&mut self, runtime_parts: &BuilderRuntimeParts) {
        self.runtime_target = Some(runtime_parts.aliases.mode_aliases[self.target].1);
    }
}

impl<T: ResolveModeEffects, const N: usize> ResolveModeEffects for [T; N] {
    fn resolve_mode_effects(&mut self, runtime_parts: &BuilderRuntimeParts) {
        for item in self {
            item.resolve_mode_effects(runtime_parts);
        }
    }
}

impl runtime::ReactionRefsExtract for BuilderModeEffect {
    type Ref<'store>
        = runtime::ModeEffectRef
    where
        Self: 'store;

    fn extract<'store>(
        &self,
        _refs: &mut runtime::ReactionRefs<'store>,
    ) -> Result<Self::Ref<'store>, runtime::ReactionRefsError> {
        let target = self
            .runtime_target
            .ok_or_else(|| runtime::ReactionRefsError::missing("mode effect"))?;
        Ok(runtime::ModeEffectRef::new_key(target, self.transition))
    }
}

impl<T: runtime::ReactorData, Q: ActionTag> ResolveModeEffects for TypedActionKey<T, Q> {
    fn resolve_mode_effects(&mut self, _runtime_parts: &BuilderRuntimeParts) {}
}

impl ResolveModeEffects for TimerActionKey {
    fn resolve_mode_effects(&mut self, _runtime_parts: &BuilderRuntimeParts) {}
}

impl<T: runtime::ReactorData, Q: PortTag, A> ResolveModeEffects for TypedPortKey<T, Q, A> {
    fn resolve_mode_effects(&mut self, _runtime_parts: &BuilderRuntimeParts) {}
}

impl<T: runtime::ReactorData, Q: PortTag, A> ResolveModeEffects for PortBank<T, Q, A> {
    fn resolve_mode_effects(&mut self, _runtime_parts: &BuilderRuntimeParts) {}
}

macro_rules! impl_resolve_mode_effects {
    ($($T:ident),*) => {
        impl<$($T,)*> ResolveModeEffects for ($($T,)*)
        where
            $($T: ResolveModeEffects,)*
        {
            #[allow(non_snake_case)]
            fn resolve_mode_effects(&mut self, runtime_parts: &BuilderRuntimeParts) {
                let ($($T,)*) = self;
                $($T.resolve_mode_effects(runtime_parts);)*
            }
        }
    };
}

all_tuples!(impl_resolve_mode_effects, 1, 10, T);

impl ReactionSpec {
    /// Get the name of this Reaction
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn record_port_relation(&mut self, key: AssemblyPortKey, trigger_mode: TriggerMode) {
        if !self.port_relations.contains_key(key) {
            self.port_order.push(key);
        }
        self.port_relations.insert(key, trigger_mode);
    }

    pub fn record_action_relation(&mut self, key: AssemblyActionKey, trigger_mode: TriggerMode) {
        if !self.action_relations.contains_key(key) {
            self.action_order.push(key);
        }
        self.action_relations.insert(key, trigger_mode);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

pub trait ReactionDeclarationField: runtime::ReactionRefsExtract {
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    );
}

impl<T, Q, A> ReactionDeclarationField for TypedPortKey<T, Q, A>
where
    T: runtime::ReactorData,
    Q: PortTag,
    TypedPortKey<T, Q, A>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let port_key = AssemblyPortKey::from(*self);
        builder.record_port_relation(port_key, trigger_mode);
    }
}

impl<T, Q, A, const N: usize> ReactionDeclarationField for [TypedPortKey<T, Q, A>; N]
where
    T: runtime::ReactorData,
    Q: PortTag,
    TypedPortKey<T, Q, A>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        self.iter().for_each(|port| {
            port.extend_builder(builder, trigger_mode);
        })
    }
}

impl<T, Q, A> ReactionDeclarationField for PortBank<T, Q, A>
where
    T: runtime::ReactorData,
    Q: PortTag,
    PortBank<T, Q, A>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        self.iter().for_each(|port| {
            let port_key = AssemblyPortKey::from(port);
            builder.record_port_relation(port_key, trigger_mode);
        });
    }
}

impl<T, Q> ReactionDeclarationField for TypedActionKey<T, Q>
where
    T: runtime::ReactorData,
    Q: ActionTag,
    TypedActionKey<T, Q>: runtime::ReactionRefsExtract,
{
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let action_key = AssemblyActionKey::from(*self);
        builder.record_action_relation(action_key, trigger_mode);
    }
}

impl ReactionDeclarationField for TimerActionKey {
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        trigger_mode: TriggerMode,
    ) {
        let action_key = AssemblyActionKey::from(*self);
        builder.record_action_relation(action_key, trigger_mode);
    }
}

impl ReactionDeclarationField for BuilderModeEffect {
    fn extend_builder<S: runtime::ReactorData, Fields, ReactionFn>(
        &self,
        builder: &mut ReactionDeclaration<S, Fields, ReactionFn>,
        _trigger_mode: TriggerMode,
    ) {
        builder.record_mode_effect(*self);
    }
}

#[derive(Debug)]
pub struct ReactionDeclaration<'a, S: runtime::ReactorData, Fields = (), ReactionFn = ()> {
    name: Option<String>,
    reaction_fn: ReactionFn,
    enabled_modes: Option<Vec<AssemblyModeKey>>,
    scope_mode: Option<AssemblyModeKey>,
    mode_effects: Vec<BuilderModeEffect>,
    reset_trigger: bool,
    port_relations: slotmap::SecondaryMap<AssemblyPortKey, TriggerMode>,
    port_order: Vec<AssemblyPortKey>,
    action_relations: slotmap::SecondaryMap<AssemblyActionKey, TriggerMode>,
    action_order: Vec<AssemblyActionKey>,
    reactor_key: AssemblyReactorKey,
    assembly: &'a mut Assembly,
    fields: Fields,
    phantom: std::marker::PhantomData<(S, Fields, ReactionFn)>,
}

impl<'a, S: runtime::ReactorData> ReactionDeclaration<'a, S, (), ()> {
    pub fn new(
        name: Option<&str>,
        reactor_key: AssemblyReactorKey,
        assembly: &'a mut Assembly,
    ) -> Self {
        Self {
            name: name.map(|s| s.to_string()),
            reaction_fn: (),
            enabled_modes: None,
            scope_mode: None,
            mode_effects: Vec::new(),
            reset_trigger: false,
            port_relations: slotmap::SecondaryMap::new(),
            port_order: Vec::new(),
            action_relations: slotmap::SecondaryMap::new(),
            action_order: Vec::new(),
            reactor_key,
            assembly,
            fields: (),
            phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, S: runtime::ReactorData, Fields, ReactionFn>
    ReactionDeclaration<'a, S, Fields, ReactionFn>
{
    fn record_port_relation(&mut self, key: AssemblyPortKey, trigger_mode: TriggerMode) {
        if !self.port_relations.contains_key(key) {
            self.port_order.push(key);
        }
        self.port_relations.insert(key, trigger_mode);
    }

    fn record_action_relation(&mut self, key: AssemblyActionKey, trigger_mode: TriggerMode) {
        if !self.action_relations.contains_key(key) {
            self.action_order.push(key);
        }
        self.action_relations.insert(key, trigger_mode);
    }

    fn record_mode_effect(&mut self, effect: BuilderModeEffect) {
        self.mode_effects.push(effect);
    }

    /// Trigger this reaction when its enclosing mode is entered by reset.
    pub fn with_reset_trigger(mut self) -> Self {
        self.reset_trigger = true;
        self
    }

    /// Record the static mode scope that owns this reaction.
    pub fn in_mode_scope(mut self, mode: AssemblyModeKey) -> Self {
        self.scope_mode = Some(mode);
        if self.enabled_modes.is_none() {
            self.enabled_modes = Some(vec![mode]);
        }
        self
    }
}

macro_rules! impl_with_field {
    ($($Fn:ident),*) => {
        impl<'a, S, $($Fn,)*> ReactionDeclaration<'a, S, ($($Fn,)*)>
        where
            S: runtime::ReactorData,
            $($Fn: runtime::ReactionRefsExtract,)*
        {
            /// Trigger this reaction on the startup of the reactor
            pub fn with_startup_trigger(self) -> ReactionDeclaration<'a, S, ($($Fn,)* TypedActionKey,)> {
                let startup = self
                    .assembly
                    .get_reactor_builder(self.reactor_key)
                    .unwrap()
                    .get_startup_action();
                self.with_trigger(startup)
            }

            /// Trigger this reaction on the shutdown of the reactor
            pub fn with_shutdown_trigger(self) -> ReactionDeclaration<'a, S, ($($Fn,)* TypedActionKey,)> {
                let shutdown = self
                    .assembly
                    .get_reactor_builder(self.reactor_key)
                    .unwrap()
                    .get_shutdown_action();
                self.with_trigger(shutdown)
            }

            /// Triggers can be input ports, output ports of contained reactors, timers, actions.
            /// There must be at least one trigger for each reaction.
            pub fn with_trigger<F>(mut self, field: F) -> ReactionDeclaration<'a, S, ($($Fn,)* F,)>
            where
                F: ReactionDeclarationField
            {
                field.extend_builder(&mut self, TriggerMode::TriggersAndUses);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields,
                    ..
                } = self;
                ReactionDeclaration {
                    name,
                    reaction_fn: (),
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields: fields.append(field),
                    phantom: std::marker::PhantomData,
                }
            }

            /// Use specifies input ports (or output ports of contained reactors) that do not trigger execution of
            /// the reaction but may be read by the reaction.
            pub fn with_use<F>(mut self, field: F) -> ReactionDeclaration<'a, S, ($($Fn,)* F,)>
            where
                F: ReactionDeclarationField
            {
                field.extend_builder(&mut self, TriggerMode::UsesOnly);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields,
                    ..
                } = self;
                ReactionDeclaration {
                    name,
                    reaction_fn: (),
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields: fields.append(field),
                    phantom: std::marker::PhantomData,
                }
            }

            /// Specify an effect field, which can be an output port, input port of contained reactors, or actions.
            pub fn with_effect<F>(mut self, field: F) -> ReactionDeclaration<'a, S, ($($Fn,)* F,)>
            where
                F: ReactionDeclarationField
            {
                field.extend_builder(&mut self, TriggerMode::EffectsOnly);
                #[allow(non_snake_case)]
                let Self {
                    name,
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields,
                    ..
                } = self;
                ReactionDeclaration {
                    name,
                    reaction_fn: (),
                    enabled_modes,
                    scope_mode,
                    mode_effects,
                    reset_trigger,
                    port_relations,
                    port_order,
                    action_relations,
                    action_order,
                    reactor_key,
                    assembly,
                    fields: fields.append(field),
                    phantom: std::marker::PhantomData,
                }
            }
        }
    };
}

trait TupleAppend<T> {
    type Output;
    fn append(self, value: T) -> Self::Output;
}

macro_rules! impl_tuple_append {
    ($($T:ident),*) => {
        impl<$($T,)* X> TupleAppend<X> for ($($T,)*)
        {
            type Output = ($($T,)* X,);
            #[allow(non_snake_case)]
            fn append(self, value: X) -> Self::Output {
                let ($($T,)*) = self;
                ($($T,)* value,)
            }
        }
    };
}

all_tuples!(impl_tuple_append, 0, 10, T);

// Generate implementations for tuples of size 0 to 10
all_tuples!(impl_with_field, 0, 10, F);

impl<'a, S, Fields> ReactionDeclaration<'a, S, Fields>
where
    S: runtime::ReactorData,
    Fields: runtime::ReactionRefsExtract + ResolveModeEffects + Clone + Send + Sync,
{
    pub fn with_reaction_fn<F>(
        self,
        f: F,
    ) -> ReactionDeclaration<'a, S, Fields, DeferredReactionFactory>
    where
        F: for<'store> Fn(&mut runtime::Context, &mut S, Fields::Ref<'store>)
            + Send
            + Sync
            + 'static,
    {
        let Self {
            name,
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            port_relations,
            port_order,
            action_relations,
            action_order,
            reactor_key,
            assembly,
            fields,
            ..
        } = self;
        let fields_for_reaction = fields.clone();
        let reaction_fn: DeferredReactionFactory = Box::new(
            move |runtime_parts: &BuilderRuntimeParts| -> runtime::BoxedReactionFn {
                let mut fields_for_reaction = fields_for_reaction.clone();
                fields_for_reaction.resolve_mode_effects(runtime_parts);
                Box::new(runtime::reaction::FnRefsAdapter::new(
                    fields_for_reaction,
                    f,
                ))
            },
        );
        ReactionDeclaration {
            name,
            reaction_fn,
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            port_relations,
            port_order,
            action_relations,
            action_order,
            reactor_key,
            assembly,
            fields,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<'a, S, Fields> ReactionDeclaration<'a, S, Fields>
where
    S: runtime::ReactorData,
    Fields: runtime::ReactionRefsExtract,
{
    pub fn with_deferred_reaction_factory<F>(
        self,
        f: F,
    ) -> ReactionDeclaration<'a, S, Fields, DeferredReactionFactory>
    where
        F: FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn + 'static,
    {
        let Self {
            name,
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            port_relations,
            port_order,
            action_relations,
            action_order,
            reactor_key,
            assembly,
            fields,
            ..
        } = self;
        ReactionDeclaration {
            name,
            reaction_fn: Box::new(f),
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            port_relations,
            port_order,
            action_relations,
            action_order,
            reactor_key,
            assembly,
            fields,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<S, Fields> ReactionDeclaration<'_, S, Fields, DeferredReactionFactory>
where
    S: runtime::ReactorData,
    Fields: runtime::ReactionRefsExtract,
{
    /// Finish building the Reaction and add it to the Environment
    pub fn finish(self) -> Result<AssemblyReactionKey, BuilderError> {
        let Self {
            name,
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            port_relations,
            port_order,
            action_relations,
            action_order,
            reaction_fn,
            reactor_key,
            assembly,
            ..
        } = self;

        // Ensure there is at least one trigger declared
        if !action_relations.values().any(|&mode| mode.is_triggers())
            && !port_relations.values().any(|&mode| mode.is_triggers())
            && !reset_trigger
        {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Reaction '{name:?}' has no triggers defined"
            )));
        }

        if reset_trigger && scope_mode.is_none() {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Reaction '{name:?}' uses reset trigger outside a mode scope"
            )));
        }

        if let Some(ref modes) = enabled_modes {
            for mode_key in modes {
                let mode = assembly.mode_specs.get(*mode_key).ok_or_else(|| {
                    BuilderError::ReactionBuilderError(format!(
                        "Unknown mode key {mode_key:?} for reaction '{name:?}'"
                    ))
                })?;
                if mode.reactor_key != reactor_key {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Mode '{}' does not belong to reaction '{name:?}'",
                        mode.name
                    )));
                }
            }
        }

        if let Some(scope_mode) = scope_mode {
            let mode = assembly.mode_specs.get(scope_mode).ok_or_else(|| {
                BuilderError::ReactionBuilderError(format!(
                    "Unknown mode key {scope_mode:?} for reaction '{name:?}'"
                ))
            })?;
            if mode.reactor_key != reactor_key {
                return Err(BuilderError::ReactionBuilderError(format!(
                    "Mode scope '{}' does not belong to reaction '{name:?}'",
                    mode.name
                )));
            }
        }

        for effect in &mode_effects {
            let mode = assembly.mode_specs.get(effect.target()).ok_or_else(|| {
                BuilderError::ReactionBuilderError(format!(
                    "Unknown mode key {:?} for reaction '{name:?}'",
                    effect.target()
                ))
            })?;
            if mode.reactor_key != reactor_key {
                return Err(BuilderError::ReactionBuilderError(format!(
                    "Mode effect '{}' does not belong to reaction '{name:?}'",
                    mode.name
                )));
            }
        }

        let reactor = &mut assembly.reactor_specs[reactor_key];
        let reactions = &mut assembly.reaction_specs;

        let reaction_builder = ReactionSpec {
            name,
            reactor_key,
            reaction_fn,
            enabled_modes,
            scope_mode,
            mode_effects,
            reset_trigger,
            action_relations,
            action_order,
            port_relations,
            port_order,
        };

        let reaction_key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());
            reaction_builder
        });

        Ok(reaction_key)
    }
}
