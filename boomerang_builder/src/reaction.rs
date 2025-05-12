use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, PortType,
    Reactor, ReactorBuilderState,
};
use crate::{runtime, BuilderRuntimeParts, ParentReactorBuilder};
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
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError>;
}

impl<T: runtime::ReactorData> ReactionField for runtime::ActionRef<'_, T> {
    //type Key = TypedActionKey<T>;
    type Key = BuilderActionKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, order, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::AsyncActionRef<T> {
    type Key = BuilderActionKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_action_relation(key, order, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::InputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, order, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::InputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relations(key, order, trigger_mode)
    }
}

impl<T: runtime::ReactorData> ReactionField for runtime::OutputRef<'_, T> {
    type Key = BuilderPortKey;

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relation(key, order, trigger_mode)
    }
}

impl<T: runtime::ReactorData, const N: usize> ReactionField for [runtime::OutputRef<'_, T>; N] {
    type Key = [BuilderPortKey; N];

    fn build(
        builder: &mut ReactionBuilderState,
        key: Self::Key,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        builder.add_port_relations(key, order, trigger_mode)
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
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        match key {
            PortOrActionTriggerKey::Port(port_key) => {
                builder.add_port_relation(port_key, order, trigger_mode)
            }
            PortOrActionTriggerKey::Action(action_key) => {
                builder.add_action_relation(action_key, order, trigger_mode)
            }
        }
    }
}

/// A boxed deferred Reaction builder function
pub type BoxedBuilderReactionFn = Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>;

pub struct ReactionBuilder {
    pub(super) name: Option<String>,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
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

impl std::fmt::Debug for ReactionBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactionBuilder")
            .field("name", &self.name)
            .field("priority", &self.priority)
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

    pub fn priority(&self) -> usize {
        self.priority
    }
}

pub struct ReactionBuilderState<'a> {
    builder: ReactionBuilder,
    env: &'a mut EnvBuilder,
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

impl<'a> ReactionBuilderState<'a> {
    pub fn new(
        name: &str,
        priority: usize,
        reactor_key: BuilderReactorKey,
        reaction_fn: BoxedBuilderReactionFn,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: Some(name.into()),
                priority,
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
        order: usize,
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
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_action_relation(action_key.into(), order, trigger_mode)?;
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
        order: usize,
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
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<(), BuilderError> {
        for key in keys {
            self.add_port_relation(key, order, trigger_mode)?;
        }
        Ok(())
    }

    /// Indicate how this Reaction interacts with the given Port
    ///
    /// There must be at least one trigger for each reaction.
    pub fn with_port(
        mut self,
        port_key: impl Into<BuilderPortKey>,
        order: usize,
        trigger_mode: TriggerMode,
    ) -> Result<Self, BuilderError> {
        self.add_port_relation(port_key.into(), order, trigger_mode)?;
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

pub mod builder2 {
    use super::{BoxedBuilderReactionFn, BuilderReactionKey, TriggerMode};
    use crate::{
        runtime, ActionTag, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey,
        BuilderRuntimeParts, EnvBuilder, PortTag, TimerActionKey, TypedActionKey, TypedPortKey,
    };

    pub trait ReactionBuilderField: runtime::ReactionRefsExtract {
        fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
            &self,
            builder: &mut ReactionBuilder<S, Fields, ReactionFn>,
            trigger_mode: TriggerMode,
        );
    }

    impl<T, Q, A> ReactionBuilderField for TypedPortKey<T, Q, A>
    where
        T: runtime::ReactorData,
        Q: PortTag,
        TypedPortKey<T, Q, A>: runtime::ReactionRefsExtract,
    {
        fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
            &self,
            builder: &mut ReactionBuilder<S, Fields, ReactionFn>,
            trigger_mode: TriggerMode,
        ) {
            let port_key = BuilderPortKey::from(*self);
            builder.port_relations.insert(port_key, trigger_mode);
        }
    }

    impl<T, Q> ReactionBuilderField for TypedActionKey<T, Q>
    where
        T: runtime::ReactorData,
        Q: ActionTag,
        TypedActionKey<T, Q>: runtime::ReactionRefsExtract,
    {
        fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
            &self,
            builder: &mut ReactionBuilder<S, Fields, ReactionFn>,
            trigger_mode: TriggerMode,
        ) {
            let action_key = BuilderActionKey::from(*self);
            builder.action_relations.insert(action_key, trigger_mode);
        }
    }

    impl ReactionBuilderField for TimerActionKey {
        fn extend_builder<S: runtime::ReactorData, Fields: Copy, ReactionFn>(
            &self,
            builder: &mut ReactionBuilder<S, Fields, ReactionFn>,
            trigger_mode: TriggerMode,
        ) {
            let action_key = BuilderActionKey::from(*self);
            builder.action_relations.insert(action_key, trigger_mode);
        }
    }

    #[derive(Debug)]
    pub struct ReactionBuilder<'a, S: runtime::ReactorData, Fields: Copy = (), ReactionFn = ()> {
        name: Option<String>,
        reaction_fn: ReactionFn,
        port_relations: slotmap::SecondaryMap<BuilderPortKey, TriggerMode>,
        action_relations: slotmap::SecondaryMap<BuilderActionKey, TriggerMode>,
        reactor_key: BuilderReactorKey,
        env: &'a mut EnvBuilder,
        phantom: std::marker::PhantomData<(S, Fields, ReactionFn)>,
    }

    impl<'a, S: runtime::ReactorData> ReactionBuilder<'a, S, (), ()> {
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
            impl<'a, S, $($Fn,)*> ReactionBuilder<'a, S, ($($Fn,)*)>
            where
                S: runtime::ReactorData,
                $($Fn: runtime::ReactionRefsExtract,)*
            {
                /// Trigger this reaction on the startup of the reactor
                pub fn with_startup_trigger(self) -> ReactionBuilder<'a, S, ($($Fn,)* TypedActionKey,)> {
                    let startup = self
                        .env
                        .get_reactor_builder(self.reactor_key)
                        .unwrap()
                        .get_startup_action();
                    self.with_trigger(startup)
                }

                /// Trigger this reaction on the shutdown of the reactor
                pub fn with_shutdown_trigger(self) -> ReactionBuilder<'a, S, ($($Fn,)* TypedActionKey,)> {
                    let shutdown = self
                        .env
                        .get_reactor_builder(self.reactor_key)
                        .unwrap()
                        .get_shutdown_action();
                    self.with_trigger(shutdown)
                }

                /// Triggers can be input ports, output ports of contained reactors, timers, actions.
                /// There must be at least one trigger for each reaction.
                pub fn with_trigger<F>(mut self, field: F) -> ReactionBuilder<'a, S, ($($Fn,)* F,)>
                where
                    F: ReactionBuilderField
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
                    ReactionBuilder {
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
                pub fn with_use<F>(mut self, field: F) -> ReactionBuilder<'a, S, ($($Fn,)* F,)>
                where
                    F: ReactionBuilderField
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
                    ReactionBuilder {
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
                pub fn with_effect<F>(mut self, field: F) -> ReactionBuilder<'a, S, ($($Fn,)* F,)>
                where
                    F: ReactionBuilderField
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
                    ReactionBuilder {
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

    impl<'a, S, Fields> ReactionBuilder<'a, S, Fields>
    where
        S: runtime::ReactorData,
        Fields: runtime::ReactionRefsExtract,
    {
        pub fn with_reaction_fn<F>(
            self,
            f: F,
        ) -> ReactionBuilder<'a, S, Fields, BoxedBuilderReactionFn>
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
            let reaction_fn: BoxedBuilderReactionFn = Box::new(|_: &BuilderRuntimeParts| {
                Box::new(
                    move |ctx: &mut runtime::Context,
                          reactor: &mut dyn runtime::BaseReactor,
                          mut refs: runtime::ReactionRefs| {
                        let state = reactor.get_state_mut::<S>().expect("state");
                        let fields = Fields::extract(&mut refs);
                        f(ctx, state, fields)
                    },
                )
            });
            ReactionBuilder {
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
        ) -> ReactionBuilder<'a, S, Fields, BoxedBuilderReactionFn>
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
            ReactionBuilder {
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

    impl<S, Fields> ReactionBuilder<'_, S, Fields, BoxedBuilderReactionFn>
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
                priority: reactor.reactions.len(),
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
}
