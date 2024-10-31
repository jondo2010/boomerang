use super::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactorKey, EnvBuilder, FindElements,
    PortType, Reactor, ReactorBuilderState,
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
    fn build<'builder>(
        name: &str,
        reactor: &R,
        builder: &'builder mut ReactorBuilderState,
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

impl<'a, T: runtime::ReactorData> ReactionField for runtime::InputRef<'a, T> {
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

impl<'a, T: runtime::ReactorData, const N: usize> ReactionField for [runtime::InputRef<'a, T>; N] {
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

impl<'a, T: runtime::ReactorData> ReactionField for runtime::OutputRef<'a, T> {
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

impl<'a, T: runtime::ReactorData, const N: usize> ReactionField for [runtime::OutputRef<'a, T>; N] {
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

pub struct ReactionBuilder {
    pub(super) name: String,
    /// Unique ordering of this reaction within the reactor.
    pub(super) priority: usize,
    /// The owning Reactor for this Reaction
    pub(super) reactor_key: BuilderReactorKey,
    /// The Reaction function
    pub(super) reaction_fn: Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>,

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
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn priority(&self) -> usize {
        self.priority
    }
}

pub struct ReactionBuilderState<'a> {
    builder: ReactionBuilder,
    env: &'a mut EnvBuilder,
}

impl<'a> FindElements for ReactionBuilderState<'a> {
    /// Find the PortKey with a given name within the parent Reactor
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError> {
        self.env
            .find_port_by_name(port_name, self.builder.reactor_key)
    }

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError> {
        self.env
            .find_action_by_name(action_name, self.builder.reactor_key)
    }
}

#[derive(Clone, Copy, Debug)]
/// Describes how an action is used by a reaction
pub enum TriggerMode {
    /// The action/port triggers the reaction, but is not provided as input
    TriggersOnly,
    /// The action/port triggers the reaction and is provided as input in the actions/ports arrays
    TriggersAndUses,
    /// The action/port triggers the reaction and is provided to the reaction in the actions/mut
    /// ports arrays
    TriggersAndEffects,
    /// The action/port does not trigger the reaction, but is provided as input in the
    /// actions/ports arrays
    UsesOnly,
    /// The action/port does not trigger the reaction, but is provided to the reaction in the
    /// actions/mut ports arrays
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
        reaction_fn: Box<dyn FnOnce(&BuilderRuntimeParts) -> runtime::BoxedReactionFn>,
        env: &'a mut EnvBuilder,
    ) -> Self {
        Self {
            builder: ReactionBuilder {
                name: name.into(),
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
                "Cannot add action '{}' to ReactionBuilder '{}', it must belong to the same reactor as the reaction",
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
                        "Reaction {} cannot 'trigger on' or 'use' input port '{}', it must belong to the same reactor as the reaction",
                        self.builder.name(),
                        self.env.port_fqn(key, false).unwrap()
                    )));
                }
                // effects are valid for input ports on contained reactors
                if trigger_mode.is_effects()
                    && port_parent_reactor_key != Some(self.builder.reactor_key)
                {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {} cannot 'effect' input port '{}', it must belong to a contained reactor",
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
                        "Reaction {} cannot 'trigger on' or 'use' output port '{}', it must belong to a contained reactor",
                        self.builder.name(),
                        port_builder.name()
                    )));
                }
                // effects are valid for output ports on the same reactor
                if trigger_mode.is_effects() && port_reactor_key != self.builder.reactor_key {
                    return Err(BuilderError::ReactionBuilderError(format!(
                        "Reaction {} cannot 'effect' output port '{}', it must belong to the same reactor as the reaction",
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
                "Reaction '{}' has no triggers defined",
                &reaction_builder.name
            )));
        }

        let reactor = &mut env.reactor_builders[reaction_builder.reactor_key];
        let reactions = &mut env.reaction_builders;
        let actions = &mut env.action_builders;
        let ports = &mut env.port_builders;

        let reaction_key = reactions.insert_with_key(|key| {
            reactor.reactions.insert(key, ());
            reaction_builder
        });

        let reaction_builder = &reactions[reaction_key];

        /*
        TODO: Move this check somewhere else?
        for port_key in reaction_builder.effect_ports.keys() {
            let port = ports.get_mut(port_key).unwrap();

            if port.port_type() == &PortType::Output {
                assert_eq!(
                    reaction_builder.reactor_key,
                    port.get_reactor_key(),
                    "Antidependent output ports must belong to the same reactor as the reaction"
                );
            } else {
                assert_eq!(
                    reaction_builder.reactor_key,
                    env.reactor_builders[port.get_reactor_key()]
                        .parent_reactor_key
                        .unwrap(),
                    "Antidependent input ports must belong to a contained reactor"
                );
            }
        }
        */

        for (port_key, trigger_mode) in &reaction_builder.port_relations {
            let port = ports.get_mut(port_key).unwrap();

            // Note, these assertions are the same as the ones on the builder methods
            /*
            if port.port_type() == &PortType::Input {
                assert_eq!(
                    reaction_builder.reactor_key,
                    port.get_reactor_key(),
                    "Input port triggers must belong to the same reactor as the triggered reaction"
                );
            } else {
                assert_eq!(
                    reaction_builder.reactor_key,
                    env.reactor_builders[port.get_reactor_key()]
                        .parent_reactor_key
                        .unwrap(),
                    "Output port triggers must belong to a contained reactor"
                );
            }
            */

            if trigger_mode.is_triggers() {
                port.register_trigger(reaction_key);
            }
        }

        Ok(reaction_key)
    }
}
