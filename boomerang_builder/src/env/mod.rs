//! Build-time environment lowering.
//!
//! The builder owns the linear construction pass that turns reactor declarations into
//! `boomerang_runtime` data, including static dependency maps and derived scheduler indexes. The
//! runtime receives those data structures ready to execute.

#[cfg(feature = "federated")]
use crate::connection::{FederatedDecoderAdapter, FederatedEncoderAdapter};
use crate::{
    connection::{BaseConnectionBuilder, ConnectionBuilder, PortBindings},
    port::Contained,
    ActionTag, Fqn, FqnSegment, ParentReactorBuilder, Physical, PortType, TimerActionKey,
    TimerSpec, TriggerMode,
};

use super::{
    action::ActionBuilder,
    port::BasePortBuilder,
    port::PortBuilder,
    reaction::{BuilderModeEffect, ReactionBuilder},
    runtime, ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderModeKey,
    BuilderPortKey, BuilderReactionKey, BuilderReactorKey, Input, Logical, ModeKind, Output,
    PortTag, ReactorBuilder, ReactorBuilderState, ReactorPlacement, TypedActionKey, TypedPortKey,
};
use itertools::Itertools;
use petgraph::{prelude::DiGraphMap, EdgeDirection};
use slotmap::{Key, SecondaryMap, SlotMap};
#[cfg(feature = "federated")]
use std::sync::Arc;
use std::{collections::HashMap, convert::TryInto};

mod build;
mod debug;
#[cfg(test)]
mod tests;

pub use build::{BuilderRuntimeParts, DeferedBuild, EnclaveDep, PartitionMap};

mod util {
    use petgraph::visit::{IntoNeighborsDirected, IntoNodeIdentifiers, Visitable};
    use std::hash::Hash;

    /// Find a minimal cycle in a graph using DFS
    pub fn find_minimal_cycle<G>(graph: G, start_node: G::NodeId) -> Vec<G::NodeId>
    where
        G: IntoNeighborsDirected + IntoNodeIdentifiers + Visitable,
        G::NodeId: Hash + Eq,
    {
        let mut dfs = petgraph::visit::Dfs::new(&graph, start_node);
        let mut stack = Vec::new();
        let mut visited = std::collections::HashSet::new();

        while let Some(nx) = dfs.next(&graph) {
            if visited.contains(&nx) {
                // We've found a cycle, backtrack to find the minimal cycle
                while let Some(&last) = stack.last() {
                    if last == nx {
                        break;
                    }
                    stack.pop();
                }
                return stack.to_vec();
            }
            visited.insert(nx);
            stack.push(nx);
        }

        // This shouldn't happen if there's definitely a cycle
        vec![start_node]
    }
}

#[cfg(feature = "replay")]
type ReplayFunctionBuilder = dyn FnOnce(&BuilderRuntimeParts) -> Box<dyn runtime::replay::ReplayFn>;

#[cfg(feature = "federated")]
type FederatedInboundEndpointBuilder = dyn FnOnce(
    &BuilderRuntimeParts,
    &mut runtime::FederatedInboundEndpointRegistry,
) -> Result<(), BuilderError>;

#[derive(Debug)]
pub struct ModeBuilder {
    pub name: String,
    pub reactor_key: BuilderReactorKey,
    pub kind: ModeKind,
}

#[derive(Default)]
pub struct EnvBuilder {
    /// Builder for Actions
    pub(super) action_builders: SlotMap<BuilderActionKey, ActionBuilder>,
    /// Builders for Ports
    pub(super) port_builders: SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
    /// Builders for Reactions
    pub(super) reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    /// Builders for Modes
    pub(super) mode_builders: SlotMap<BuilderModeKey, ModeBuilder>,
    /// Builders for Reactors
    pub(super) reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
    /// Builders for Connections
    pub(super) connection_builders: Vec<Box<dyn BaseConnectionBuilder>>,
    #[cfg(feature = "federated")]
    /// Builders for inbound federated endpoint registry entries.
    pub(super) federated_inbound_endpoint_builders: Vec<Box<FederatedInboundEndpointBuilder>>,
    #[cfg(feature = "replay")]
    /// Builders for Replay functions
    pub(super) replay_builders: SecondaryMap<BuilderActionKey, Box<ReplayFunctionBuilder>>,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a new Reactor
    /// - name: Instance name of the reactor
    pub fn add_reactor<S: runtime::ReactorData>(
        &mut self,
        name: &str,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        state: S,
        placement: impl Into<ReactorPlacement>,
    ) -> ReactorBuilderState<'_, S> {
        tracing::debug!("Adding Reactor: {name}");
        ReactorBuilderState::new(name, parent, bank_info, state, placement, self)
    }

    /// Get a previously built reactor
    pub fn get_reactor_builder(
        &mut self,
        reactor_key: BuilderReactorKey,
    ) -> Result<ReactorBuilderState<'_>, BuilderError> {
        if !self.reactor_builders.contains_key(reactor_key) {
            return Err(BuilderError::ReactorKeyNotFound(reactor_key));
        }
        Ok(ReactorBuilderState::from_pre_existing(reactor_key, self))
    }

    /// Add an Input port to the Reactor
    pub fn add_input_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T, Input>, BuilderError> {
        self.internal_add_port::<T, Input>(name, reactor_key, None)
            .map(From::from)
    }

    /// Add an Output port to the Reactor
    pub fn add_output_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T, Output>, BuilderError> {
        self.internal_add_port::<T, Output>(name, reactor_key, None)
            .map(From::from)
    }

    pub fn internal_add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        bank_info: Option<runtime::BankInfo>,
    ) -> Result<BuilderPortKey, BuilderError> {
        // Ensure no duplicates on (name, reactor_key, bank_info)
        if self.port_builders.values().any(|port| {
            port.name() == name
                && port.get_reactor_key() == reactor_key
                && port.bank_info() == bank_info.as_ref()
        }) {
            return Err(BuilderError::DuplicatePortDefinition {
                reactor_name: self.reactor_builders[reactor_key].name().to_owned(),
                port_name: name.into(),
            });
        }

        let key = self.port_builders.insert_with_key(|port_key| {
            self.reactor_builders[reactor_key]
                .ports
                .insert(port_key, ());
            Box::new(PortBuilder::<T, Q>::new(name, reactor_key, bank_info))
        });

        Ok(key)
    }

    pub fn add_startup_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action_impl::<(), Logical>(
            name,
            reactor_key,
            None,
            ActionType::Timer(TimerSpec::STARTUP),
        )
    }

    pub fn add_shutdown_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action_impl::<(), Logical>(name, reactor_key, None, ActionType::Shutdown)
    }

    pub fn add_mode(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        kind: impl Into<ModeKind>,
    ) -> Result<BuilderModeKey, BuilderError> {
        let kind = kind.into();
        let reactor_builder = &mut self.reactor_builders[reactor_key];
        if reactor_builder.modes.keys().any(|mode_key| {
            self.mode_builders[mode_key].name == name
                && self.mode_builders[mode_key].reactor_key == reactor_key
        }) {
            return Err(BuilderError::DuplicateModeDefinition {
                reactor_name: reactor_builder.name().to_owned(),
                mode_name: name.to_owned(),
            });
        }

        if kind.is_initial() && reactor_builder.initial_mode.is_some() {
            return Err(BuilderError::MultipleInitialModes {
                reactor_name: reactor_builder.name().to_owned(),
            });
        }

        let key = self.mode_builders.insert(ModeBuilder {
            name: name.to_owned(),
            reactor_key,
            kind,
        });

        reactor_builder.modes.insert(key, ());
        if kind.is_initial() {
            reactor_builder.initial_mode = Some(key);
        }

        Ok(key)
    }

    pub fn mode_effect(
        &self,
        reactor_key: BuilderReactorKey,
        mode_key: BuilderModeKey,
        transition: runtime::TransitionKind,
    ) -> Result<BuilderModeEffect, BuilderError> {
        let mode = self.mode_builders.get(mode_key).ok_or_else(|| {
            BuilderError::ReactionBuilderError(format!("Unknown mode key {mode_key:?}"))
        })?;
        if mode.reactor_key != reactor_key {
            return Err(BuilderError::ReactionBuilderError(format!(
                "Mode '{}' does not belong to reactor '{}'",
                mode.name,
                self.reactor_builders[reactor_key].name()
            )));
        }
        Ok(BuilderModeEffect::new(mode_key, transition))
    }

    /// Add a Timer Action to the given Reactor
    pub fn add_timer_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        timer_spec: TimerSpec,
    ) -> Result<TimerActionKey, BuilderError> {
        self.add_timer_action_in_scope(name, reactor_key, None, timer_spec)
    }

    pub(crate) fn add_timer_action_in_scope(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        scope_mode: Option<BuilderModeKey>,
        timer_spec: TimerSpec,
    ) -> Result<TimerActionKey, BuilderError> {
        let action_key = self.add_action_impl::<(), Logical>(
            name,
            reactor_key,
            scope_mode,
            ActionType::Timer(timer_spec),
        )?;
        Ok(TimerActionKey::from(BuilderActionKey::from(action_key)))
    }

    /// Add a user Action to the given Reactor.
    pub fn add_action<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Q>, BuilderError> {
        self.add_action_in_scope::<T, Q>(name, min_delay, reactor_key, None)
    }

    pub(crate) fn add_action_in_scope<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
        scope_mode: Option<BuilderModeKey>,
    ) -> Result<TypedActionKey<T, Q>, BuilderError> {
        self.add_action_impl::<T, Q>(
            name,
            reactor_key,
            scope_mode,
            ActionType::Standard {
                is_logical: Q::IS_LOGICAL,
                min_delay,
                build_fn: Box::new(move |name, key| {
                    runtime::Action::<T>::new(name, key, min_delay, Q::IS_LOGICAL).boxed()
                }),
            },
        )
    }

    /// Internal implementation for adding an Action to a Reactor
    fn add_action_impl<T, Q: ActionTag>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        scope_mode: Option<BuilderModeKey>,
        r#type: ActionType,
    ) -> Result<TypedActionKey<T, Q>, BuilderError>
    where
        T: runtime::ReactorData,
    {
        let reactor_builder = &mut self.reactor_builders[reactor_key];

        if let Some(mode_key) = scope_mode {
            let mode = self.mode_builders.get(mode_key).ok_or_else(|| {
                BuilderError::ReactionBuilderError(format!(
                    "Unknown mode key {mode_key:?} for action '{name}'"
                ))
            })?;
            if mode.reactor_key != reactor_key {
                return Err(BuilderError::ReactionBuilderError(format!(
                    "Mode '{}' does not belong to action '{name}'",
                    mode.name
                )));
            }
        }

        // Ensure no duplicates
        if reactor_builder
            .actions
            .keys()
            .any(|action_key| self.action_builders[action_key].name() == name)
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: reactor_builder.name().to_owned(),
                action_name: name.into(),
            });
        }

        let key =
            self.action_builders
                .insert(ActionBuilder::new(name, reactor_key, scope_mode, r#type));

        reactor_builder.actions.insert(key, ());

        Ok(key.into())
    }

    /// Add a replay function for a given Action
    #[cfg(feature = "replay")]
    pub fn add_replayer<T, Q, F>(
        &mut self,
        action_key: TypedActionKey<T, Q>,
        replayer_builder_fn: F,
    ) -> Result<(), BuilderError>
    where
        T: boomerang_runtime::ReactorData + for<'de> serde::Deserialize<'de>,
        Q: ActionTag,
        F: FnOnce(&BuilderRuntimeParts) -> Box<dyn runtime::replay::ReplayFn> + 'static,
    {
        let action_key = action_key.into();
        if self.replay_builders.contains_key(action_key) {
            return Err(BuilderError::ReplayKeyAlreadyExists(action_key));
        }

        self.replay_builders
            .insert(action_key, Box::new(replayer_builder_fn));

        Ok(())
    }

    /// Get a previously built action
    pub fn get_action(&self, action_key: BuilderActionKey) -> Result<&ActionBuilder, BuilderError> {
        self.action_builders
            .get(action_key)
            .ok_or(BuilderError::ActionKeyNotFound(action_key))
    }

    /// Get a previously built port
    pub fn get_port(&self, port_key: BuilderPortKey) -> Result<&dyn BasePortBuilder, BuilderError> {
        self.port_builders
            .get(port_key)
            .map(|builder| builder.as_ref())
            .ok_or(BuilderError::PortKeyNotFound(port_key))
    }

    /// Find a port in a child Reactor given it's name and the parent ReactorKey.
    ///
    /// The port data type and tag must be provided as type parameters.
    pub fn find_port_by_name<T: runtime::ReactorData, Q: PortTag>(
        &self,
        port_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Option<TypedPortKey<T, Q, Contained>> {
        self.port_builders
            .iter()
            .find(|(_, port_builder)| {
                port_builder.name() == port_name
                    && port_builder.parent_reactor_key() == Some(reactor_key)
                    && port_builder.inner_type_id() == std::any::TypeId::of::<(T, Q)>()
            })
            .map(|(port_key, _)| port_key.into())
    }

    /// Find a Reaction matching a given name and ReactorKey
    pub fn find_reaction_by_name(
        &self,
        reaction_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderReactionKey, BuilderError> {
        self.reaction_builders
            .iter()
            .find(|(_, reaction_builder)| {
                reaction_builder
                    .name()
                    .map(|name| name == reaction_name)
                    .unwrap_or_default()
                    && reaction_builder.parent_reactor_key() == Some(reactor_key)
            })
            .map(|(reaction_key, _)| reaction_key)
            .ok_or_else(|| BuilderError::NamedReactionNotFound(reaction_name.to_string()))
    }

    /// Find an Action matching a given name and ReactorKey
    pub fn find_action_by_name(
        &self,
        action_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.reactor_builders[reactor_key]
            .actions
            .keys()
            .find(|action_key| self.action_builders[*action_key].name() == action_name)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_name.to_string()))
    }

    /// Find a Reactor in the EnvBuilder given its fully-qualified name
    pub fn find_reactor_by_fqn<T>(&self, reactor_fqn: T) -> Result<BuilderReactorKey, BuilderError>
    where
        T: TryInto<BuilderFqn>,
        T::Error: Into<BuilderError>,
    {
        let reactor_fqn: BuilderFqn = reactor_fqn.try_into().map_err(Into::into)?;
        let (_, segment) = reactor_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(reactor_fqn.to_string()))?;
        self.reactor_builders
            .iter()
            .find_map(|(reactor_key, reactor_builder)| {
                if reactor_builder.fqn_segment(false) == segment {
                    Some(reactor_key)
                } else {
                    None
                }
            })
            .ok_or_else(|| BuilderError::NamedReactorNotFound(reactor_fqn.to_string()))
    }

    /// Find a PhysicalAction globally in the EnvBuilder given its fully-qualified name
    pub fn find_physical_action_by_fqn<T>(
        &self,
        action_fqn: T,
    ) -> Result<BuilderActionKey, BuilderError>
    where
        T: TryInto<BuilderFqn>,
        T::Error: Into<BuilderError>,
    {
        let action_fqn: BuilderFqn = action_fqn.try_into().map_err(Into::into)?;

        let (reactor_fqn, segment) = action_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(action_fqn.to_string()))?;

        let reactor = self.find_reactor_by_fqn(reactor_fqn)?;

        self.reactor_builders[reactor]
            .actions
            .keys()
            .find(|action_key| self.action_builders[*action_key].fqn_segment(false) == segment)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_fqn.to_string()))
    }

    /// Find a possible common parent Reactor for two Reactor elements in the EnvBuilder (if it
    /// exists).
    pub fn common_reactor_key<E0, E1>(&self, e0: &E0, e1: &E1) -> Option<BuilderReactorKey>
    where
        E0: ParentReactorBuilder,
        E1: ParentReactorBuilder,
    {
        let mut e0_key = e0.parent_reactor_key();
        let mut e1_key = e1.parent_reactor_key();
        while e0_key != e1_key {
            match (e0_key, e1_key) {
                (Some(key0), Some(key1)) => {
                    e0_key = self.reactor_builders[key0].parent_reactor_key;
                    e1_key = self.reactor_builders[key1].parent_reactor_key;
                }
                _ => return None,
            }
        }
        e0_key
    }

    /// Connect two ports together
    ///
    /// ## Arguments
    ///
    /// * `source_key` - The key of the first port to connect
    /// * `target_key` - The key of the second port to connect
    /// * `after` - An optional delay to wait before triggering the downstream ports.
    /// * `physical` - Whether the connection is physical (or logical).
    ///     * Logical connections will trigger any downstream ports at the current logical time
    ///       (with any `after` delay).
    ///     * Physical connections will trigger the downstream ports at the current physical time
    ///       (with any `after` delay).
    pub fn add_port_connection<T, P1, P2>(
        &mut self,
        source_key: P1,
        target_key: P2,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
    {
        self.add_port_connection_in_scope::<T, P1, P2>(
            source_key, target_key, None, after, physical,
        )
    }

    pub(crate) fn add_port_connection_in_scope<T, P1, P2>(
        &mut self,
        source_key: P1,
        target_key: P2,
        scope_mode: Option<BuilderModeKey>,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
    {
        let source_key = source_key.into();
        let target_key = target_key.into();

        tracing::debug!(
            "Adding connection from {} to {} with after={after:?} and physical={physical}",
            source_key.fqn(self, false)?,
            target_key.fqn(self, false)?,
        );

        self.connection_builders.push(if physical {
            Box::new(ConnectionBuilder::<T, Physical> {
                source_key,
                target_key,
                after,
                scope_mode,
                #[cfg(feature = "federated")]
                federated_encoder: None,
                #[cfg(feature = "federated")]
                federated_decoder: None,
                _phantom: Default::default(),
            })
        } else {
            Box::new(ConnectionBuilder::<T, Logical> {
                source_key,
                target_key,
                after,
                scope_mode,
                #[cfg(feature = "federated")]
                federated_encoder: None,
                #[cfg(feature = "federated")]
                federated_decoder: None,
                _phantom: Default::default(),
            })
        });

        Ok(())
    }

    #[cfg(feature = "federated")]
    pub fn add_federated_port_connection<T, P1, P2, C>(
        &mut self,
        source_key: P1,
        target_key: P2,
        after: Option<runtime::Duration>,
        codec: C,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
        C: boomerang_federated::PayloadEncoder<T>
            + boomerang_federated::PayloadDecoder<T>
            + Send
            + Sync
            + 'static,
    {
        self.add_federated_port_connection_in_scope::<T, P1, P2, C>(
            source_key, target_key, None, after, codec,
        )
    }

    #[cfg(feature = "federated")]
    pub(crate) fn add_federated_port_connection_in_scope<T, P1, P2, C>(
        &mut self,
        source_key: P1,
        target_key: P2,
        scope_mode: Option<BuilderModeKey>,
        after: Option<runtime::Duration>,
        codec: C,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
        C: boomerang_federated::PayloadEncoder<T>
            + boomerang_federated::PayloadDecoder<T>
            + Send
            + Sync
            + 'static,
    {
        let source_key = source_key.into();
        let target_key = target_key.into();

        tracing::debug!(
            "Adding federated connection from {} to {} with after={after:?}",
            source_key.fqn(self, false)?,
            target_key.fqn(self, false)?,
        );

        let codec = Arc::new(codec);
        let encoder_codec = Arc::clone(&codec);
        let decoder_codec = Arc::clone(&codec);

        self.connection_builders
            .push(Box::new(ConnectionBuilder::<T, Logical> {
                source_key,
                target_key,
                after,
                scope_mode,
                federated_encoder: Some(Box::new(move || {
                    Box::new(FederatedEncoderAdapter {
                        codec: encoder_codec,
                    })
                })),
                federated_decoder: Some(Box::new(move || {
                    Box::new(FederatedDecoderAdapter {
                        codec: decoder_codec,
                    })
                })),
                _phantom: Default::default(),
            }));

        Ok(())
    }

    #[cfg(feature = "federated")]
    pub(super) fn add_federated_inbound_endpoint<T>(
        &mut self,
        endpoint: runtime::FederatedEndpointId,
        target_partition: BuilderReactorKey,
        target_action_key: BuilderActionKey,
        decoder: Box<dyn runtime::FederatedPayloadDecoder<T>>,
    ) where
        T: runtime::ReactorData,
    {
        self.federated_inbound_endpoint_builders
            .push(Box::new(move |builder_parts, registry| {
                let (enclave_key, runtime_action_key) = *builder_parts
                    .aliases
                    .action_aliases
                    .get(target_action_key)
                    .ok_or_else(|| {
                        BuilderError::InternalError(format!(
                            "missing runtime action alias for federated endpoint {endpoint}"
                        ))
                    })?;
                let expected_enclave_key = builder_parts.aliases.enclave_aliases[target_partition];
                if enclave_key != expected_enclave_key {
                    return Err(BuilderError::InternalError(format!(
                        "federated endpoint {endpoint} resolved to wrong target enclave"
                    )));
                }

                let enclave = &builder_parts.enclaves[enclave_key];
                let context = enclave.create_send_context(enclave_key);
                let action_ref = enclave.create_async_action_ref(runtime_action_key);
                registry
                    .register(endpoint, context, action_ref, decoder)
                    .map_err(|error| BuilderError::UnsupportedFederationTopology {
                        what: error.to_string(),
                    })
            }));
    }

    /// Get a fully-qualified name for a given key
    pub fn fqn_for(&self, key: impl Fqn, grouped: bool) -> Result<BuilderFqn, BuilderError> {
        key.fqn(self, grouped)
    }

    /// Validate the reactions in the environment
    pub fn validate_reactions(&self) -> Result<(), BuilderError> {
        for (_reaction_key, reaction) in &self.reaction_builders {
            // Validate the port dependencies of each Reaction
            for (port_key, trigger_mode) in &reaction.port_relations {
                let port = &self.port_builders[port_key];
                let port_reactor_key = port.get_reactor_key();
                let port_parent_reactor_key =
                    self.reactor_builders[port_reactor_key].parent_reactor_key;
                match port.port_type() {
                    PortType::Input => {
                        // triggers and uses are valid for input ports on the same reactor
                        if (trigger_mode.is_triggers() || trigger_mode.is_uses())
                            && port_reactor_key != reaction.reactor_key
                        {
                            return Err(BuilderError::ReactionBuilderError(format!(
                                "Reaction {:?} cannot 'trigger on' or 'use' input port '{}', it \
                                 must belong to the same reactor as the reaction",
                                reaction.name(),
                                self.fqn_for(port_key, false).unwrap()
                            )));
                        }
                        // effects are valid for input ports on contained reactors
                        if trigger_mode.is_effects()
                            && port_parent_reactor_key != Some(reaction.reactor_key)
                        {
                            return Err(BuilderError::ReactionBuilderError(format!(
                                "Reaction {:?} cannot 'effect' input port '{}', it must belong to \
                                 a contained reactor",
                                reaction.name(),
                                port.name()
                            )));
                        }
                    }
                    PortType::Output => {
                        // triggers and uses are valid for output ports on contained reactors
                        if (trigger_mode.is_triggers() || trigger_mode.is_uses())
                            && port_parent_reactor_key != Some(reaction.reactor_key)
                        {
                            return Err(BuilderError::ReactionBuilderError(format!(
                                "Reaction {:?} cannot 'trigger on' or 'use' output port '{}', it \
                                 must belong to a contained reactor",
                                reaction.name(),
                                port.name()
                            )));
                        }
                        // effects are valid for output ports on the same reactor
                        if trigger_mode.is_effects() && port_reactor_key != reaction.reactor_key {
                            return Err(BuilderError::ReactionBuilderError(format!(
                                "Reaction {:?} cannot 'effect' output port '{}', it must belong \
                                 to the same reactor as the reaction",
                                reaction.name(),
                                port.name()
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Build an iterator of all Reaction dependency edges in the graph
    pub fn reaction_dependency_edges<'a, 'b: 'a>(
        &'a self,
        port_bindings: &'b PortBindings,
    ) -> impl Iterator<Item = (BuilderReactionKey, BuilderReactionKey)> + 'a {
        let deps = self
            .reaction_builders
            .iter()
            .flat_map(move |(reaction_key, reaction)| {
                // Connect all reactions this reaction depends upon
                reaction
                    .port_relations
                    .iter()
                    .filter_map(|(port_key, trigger_mode)| {
                        trigger_mode.is_triggers().then_some(port_key)
                    })
                    .flat_map(|port_key| {
                        let source_port_key = port_bindings.follow_port_inward(port_key);

                        // all reactions that can set this port
                        self.reaction_builders
                            .iter()
                            .filter_map(move |(reaction_key, reaction)| {
                                match reaction.port_relations.get(source_port_key) {
                                    Some(TriggerMode::EffectsOnly)
                                    | Some(TriggerMode::TriggersAndEffects) => Some(reaction_key),
                                    _ => None,
                                }
                            })
                    })
                    .filter(move |&dep_key| {
                        !self.reactions_are_mutually_exclusive(reaction_key, dep_key)
                    })
                    .map(move |dep_key| (reaction_key, dep_key))
            });

        // For all Reactions within a Reactor, create a chain of dependencies by priority. This
        // ensures that Reactions within a Reactor always end up at unique levels.
        let internal = self.reactor_builders.values().flat_map(move |reactor| {
            let reactions = reactor
                .reactions
                .keys()
                .sorted_by_key(|&reaction_key| {
                    //self.reaction_builders[reaction_key].priority
                    reaction_key.data().as_ffi()
                })
                .rev()
                .collect_vec();

            let mut edges = Vec::new();
            for (idx, reaction_key) in reactions.iter().copied().enumerate() {
                for dep_key in reactions.iter().copied().skip(idx + 1) {
                    if !self.reactions_are_mutually_exclusive(reaction_key, dep_key) {
                        edges.push((reaction_key, dep_key));
                    }
                }
            }
            edges
        });
        deps.chain(internal)
    }

    fn reactions_are_mutually_exclusive(
        &self,
        a: BuilderReactionKey,
        b: BuilderReactionKey,
    ) -> bool {
        let a_modes = self.enclosing_modes_for_reaction(a);
        let b_modes = self.enclosing_modes_for_reaction(b);

        a_modes.iter().any(|&a_mode| {
            b_modes.iter().any(|&b_mode| {
                a_mode != b_mode
                    && self.mode_builders[a_mode].reactor_key
                        == self.mode_builders[b_mode].reactor_key
            })
        })
    }

    fn enclosing_modes_for_reaction(
        &self,
        reaction_key: BuilderReactionKey,
    ) -> Vec<BuilderModeKey> {
        let reaction = &self.reaction_builders[reaction_key];
        let mut modes = Vec::new();
        if let Some(mode) = reaction.scope_mode {
            modes.push(mode);
        }

        let mut reactor_key = Some(reaction.reactor_key);
        while let Some(key) = reactor_key {
            let reactor = &self.reactor_builders[key];
            if let Some(mode) = reactor.scope_mode {
                modes.push(mode);
            }
            reactor_key = reactor.parent_reactor_key;
        }

        modes
    }

    /// Build a DAG of Reactions
    pub fn build_reaction_graph(
        &self,
        port_bindings: &PortBindings,
    ) -> DiGraphMap<BuilderReactionKey, ()> {
        let mut graph = DiGraphMap::from_edges(
            self.reaction_dependency_edges(port_bindings)
                .map(|(a, b)| (b, a)),
        );
        // Ensure all ReactionIndicies are represented
        self.reaction_builders.keys().for_each(|key| {
            graph.add_node(key);
        });

        graph
    }

    /// Build a DAG of Reactors from the parent-child relationships
    pub fn build_reactor_graph(&self) -> DiGraphMap<BuilderReactorKey, ()> {
        let mut graph =
            DiGraphMap::from_edges(self.reactor_builders.iter().filter_map(|(key, reactor)| {
                reactor
                    .parent_reactor_key
                    .map(|parent_key| (parent_key, key))
            }));
        // ensure all Reactors are represented
        self.reactor_builders.keys().for_each(|key| {
            graph.add_node(key);
        });
        graph
    }

    /// Build a Mapping of `BuilderReactionKey` -> `Level` corresponding to the parallelizable
    /// schedule
    ///
    /// This implements the Coffman-Graham algorithm for job scheduling.
    /// See <https://en.m.wikipedia.org/wiki/Coffman%E2%80%93Graham_algorithm>
    pub fn build_runtime_level_map(
        &self,
        port_bindings: &PortBindings,
    ) -> Result<SecondaryMap<BuilderReactionKey, runtime::Level>, BuilderError> {
        use petgraph::{algo::tred, graph::DefaultIx, graph::NodeIndex};

        let mut graph = self
            .build_reaction_graph(port_bindings)
            .into_graph::<DefaultIx>();

        // Transitive reduction and closures
        let toposort = petgraph::algo::toposort(&graph, None).map_err(|cycle_error| {
            // A Cycle was found in the reaction graph.

            let res = petgraph::algo::astar(
                &graph,
                cycle_error.node_id(),
                |finish| finish == cycle_error.node_id(),
                |_| 1,
                |_| 0,
            );
            dbg!(res);

            // let fas = petgraph::algo::greedy_feedback_arc_set(&graph);
            // let cycle = petgraph::prelude::DiGraphMap::<BuilderReactionKey, ()>::from_edges(fas);
            let cycle = util::find_minimal_cycle(&graph, cycle_error.node_id())
                .into_iter()
                .map(|node_index| graph[node_index])
                .collect_vec();

            BuilderError::ReactionGraphCycle { what: cycle }
        })?;

        let (res, _) = tred::dag_to_toposorted_adjacency_list::<_, NodeIndex>(&graph, &toposort);
        let (_reduc, close) = tred::dag_transitive_reduction_closure(&res);

        // Replace the edges in graph with the transitive closure edges
        graph.clear_edges();
        graph.extend_with_edges(close.edge_indices().filter_map(|e| {
            close
                .edge_endpoints(e)
                .map(|(a, b)| (toposort[a.index()], toposort[b.index()]))
        }));

        let mut levels: HashMap<_, runtime::Level> = HashMap::new();
        for &idx in toposort.iter() {
            let max_neighbor = graph
                .neighbors_directed(idx, EdgeDirection::Incoming)
                .map(|neighbor_idx| *levels.entry(neighbor_idx).or_default())
                .max()
                .unwrap_or_default();

            levels.insert(idx, max_neighbor + 1);
        }

        // Collect and return a Map with ReactionKey indices instead of NodeIndex
        Ok(levels
            .iter()
            .map(|(&idx, &level)| (graph[idx], level - 1))
            .collect())
    }
}
