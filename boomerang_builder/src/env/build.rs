use boomerang_runtime::{self as runtime, LevelReactionKey};
use itertools::Itertools;
use slotmap::{SecondaryMap, SlotMap};

use crate::{
    connection::PortBindings, ActionBuilder, ActionType, BasePortBuilder, BuilderActionKey,
    BuilderError, BuilderPortKey, BuilderReactionKey, BuilderReactorKey, ParentReactorBuilder,
};

use super::{collect_transitive_port_triggers, EnvBuilder};

/// Return type for building runtime parts
#[derive(Debug, Default)]
pub(crate) struct RuntimePortParts {
    /// All runtime Ports
    pub ports: tinymap::TinyMap<runtime::PortKey, Box<dyn runtime::BasePort>>,
    /// For each Port, a set of Reactions triggered by it
    pub port_triggers: tinymap::TinySecondaryMap<runtime::PortKey, Vec<BuilderReactionKey>>,
    /// A mapping from `BuilderPortKey`s to aliased [`runtime::PortKey`]s.
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

#[derive(Debug, Default)]
pub(super) struct RuntimeActionParts {
    actions: tinymap::TinyMap<runtime::ActionKey, Box<dyn runtime::BaseAction>>,
    action_triggers: tinymap::TinySecondaryMap<runtime::ActionKey, Vec<BuilderReactionKey>>,
    startup_reactions: Vec<BuilderReactionKey>,
    shutdown_reactions: Vec<BuilderReactionKey>,
    aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
}

#[derive(Debug, Default)]
pub(super) struct RuntimeReactionParts {
    reactions: tinymap::TinyMap<runtime::ReactionKey, runtime::Reaction>,
    use_ports: tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::PortKey>>,
    effect_ports:
        tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::PortKey>>,
    actions: tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::ActionKey>>,
    /// Aliases from BuilderReactionKey to runtime::ReactionKey
    reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    /// Aliases from BuilderReactionKey to it's owning BuilderReactorKey
    reaction_reactor_aliases: SecondaryMap<BuilderReactionKey, BuilderReactorKey>,
}

#[derive(Debug, Default)]
pub(super) struct RuntimeReactorParts {
    pub(super) runtime_reactors:
        tinymap::TinyMap<runtime::ReactorKey, Box<dyn runtime::BaseReactor>>,
    pub(super) reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub(super) reactor_bank_indices:
        tinymap::TinySecondaryMap<runtime::ReactorKey, Option<runtime::BankInfo>>,
}

/// Runtime parts of an enclave
#[derive(Debug, Default)]
pub struct EnclaveParts {
    /// The runtime Enclave
    pub enclave: runtime::Enclave,
    /// Aliases from builder keys to runtime keys
    pub aliases: BuilderAliases,
}

pub type EnclavePartsMap = SecondaryMap<BuilderReactorKey, EnclaveParts>;

/// Intermediate parts used to build the runtime structures
#[derive(Default)]
struct IntermediateParts {
    port_parts: RuntimePortParts,
    action_parts: RuntimeActionParts,
    reaction_parts: RuntimeReactionParts,
    reactor_parts: RuntimeReactorParts,
}

#[cfg(feature = "disable")]
impl IntermediateParts {
    /// Construct runtime port structures from the builders.
    fn build_runtime_ports(
        &mut self,
        port_builders: &SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
        partitioned_port_keys: impl Iterator<Item = BuilderPortKey>,
        port_bindings: &PortBindings,
    ) {
        let RuntimePortParts {
            ports: runtime_ports,
            port_triggers,
            port_aliases: alias_map,
        } = &mut self.port_parts;

        let port_groups = partitioned_port_keys
            .map(|port_key| (port_key, port_bindings.follow_port_inward(port_key)))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let downstream_reactions =
                collect_transitive_port_triggers(inward_port_key, port_builders, port_bindings)
                    .keys()
                    .collect_vec();

            let runtime_port_key = runtime_ports
                .insert_with_key(|key| port_builders[inward_port_key].build_runtime_port(key));

            port_triggers.insert(runtime_port_key, downstream_reactions);

            alias_map.extend(group.map(move |(port_key, _inward_key)| (port_key, runtime_port_key)))
        }
    }

    /// Construct runtime action structures from the builders.
    fn build_runtime_actions<I>(
        &mut self,
        action_builders: I,
        reaction_builders: &[(BuilderReactionKey, ReactionBuilder)],
    ) where
        I: IntoIterator<Item = (BuilderActionKey, ActionBuilder)>,
        I::IntoIter: ExactSizeIterator,
    {
        let action_builders = action_builders.into_iter();

        //let mut runtime_actions = tinymap::TinyMap::with_capacity(action_builders.len());
        //let mut action_triggers = tinymap::TinySecondaryMap::with_capacity(action_builders.len());
        //let mut startup_reactions = Vec::new();
        //let mut shutdown_reactions = Vec::new();
        //let mut action_alias = SecondaryMap::new();

        let RuntimeActionParts {
            actions: runtime_actions,
            action_triggers,
            startup_reactions,
            shutdown_reactions,
            aliases: action_alias,
        } = &mut self.action_parts;

        for (builder_action_key, action_builder) in action_builders {
            // Find all the reactions that are triggered by this action
            let triggered_reactions =
                reaction_builders
                    .iter()
                    .filter_map(|(reaction_key, reaction)| {
                        match reaction.action_relations.get(builder_action_key) {
                            Some(TriggerMode::TriggersOnly)
                            | Some(TriggerMode::TriggersAndEffects)
                            | Some(TriggerMode::TriggersAndUses) => Some(reaction_key),
                            _ => None,
                        }
                    });

            match action_builder.r#type() {
                ActionType::Startup => startup_reactions.extend(triggered_reactions),
                ActionType::Shutdown => shutdown_reactions.extend(triggered_reactions),
                ActionType::Standard { build_fn, .. } => {
                    let action_key = runtime_actions
                        .insert_with_key(|key| (build_fn)(action_builder.name(), key));
                    action_triggers.insert(action_key, triggered_reactions.copied().collect());
                    action_alias.insert(builder_action_key, action_key);
                }
                ActionType::Timer(timer_spec) => {
                    let action_key = runtime_actions.insert_with_key(|action_key| {
                        runtime::Action::<()>::new(action_builder.name(), action_key, None, true)
                            .boxed()
                    });

                    let reaction_key = self.reaction_parts.reactions.insert({
                        runtime::Reaction::new(
                            &format!("{}_reaction", action_builder.name()),
                            runtime::reaction::TimerFn(timer_spec.period),
                            None,
                        )
                    });

                    todo!("Finish me")
                }
            }
        }
    }

    /// Build the runtime reactions for a single partition of builders
    #[cfg(feature = "disable")]
    fn build_runtime_reactions<I>(
        &mut self,
        reaction_builders: I,
        //port_aliases: &SecondaryMap<BuilderPortKey, runtime::PortKey>,
        //action_aliases: &SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    ) where
        I: IntoIterator<Item = (BuilderReactionKey, ReactionBuilder)>,
        I::IntoIter: ExactSizeIterator,
    {
        let reaction_builders = reaction_builders.into_iter();

        //let mut runtime_reactions = tinymap::TinyMap::with_capacity(reaction_builders.len());
        //let mut reaction_use_ports = tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
        //let mut reaction_effect_ports = tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
        //let mut reaction_actions = tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
        //let mut reaction_aliases = SecondaryMap::new();
        //let mut reaction_reactor_aliases = SecondaryMap::new();

        let RuntimeReactionParts {
            reactions: runtime_reactions,
            use_ports: reaction_use_ports,
            effect_ports: reaction_effect_ports,
            actions: reaction_actions,
            reaction_aliases,
            reaction_reactor_aliases,
        } = &mut self.reaction_parts;

        let port_aliases = &self.port_parts.port_aliases;
        let action_aliases = &self.action_parts.aliases;

        for (builder_key, reaction_builder) in reaction_builders.into_iter() {
            reaction_reactor_aliases.insert(builder_key, reaction_builder.reactor_key);
            // Create the set of readable ports for this reaction sorted by order
            let use_port_set = reaction_builder
                .use_ports
                .iter()
                .sorted_by_key(|(_, &order)| order)
                .map(|(builder_port_key, _)| port_aliases[builder_port_key])
                .collect();

            // Create the set of writable ports for this reaction sorted by order
            let effect_port_set = reaction_builder
                .effect_ports
                .iter()
                .sorted_by_key(|(_, &order)| order)
                .map(|(builder_port_key, _)| port_aliases[builder_port_key])
                .collect();

            // Create the Vec of actions for this reaction sorted by order
            let actions_set = reaction_builder
                .use_effect_actions
                .iter()
                .sorted_by_key(|(_, &order)| order)
                .map(|(builder_action_key, _)| action_aliases[builder_action_key])
                .collect();

            let reaction_key = runtime_reactions.insert({
                runtime::Reaction::new(&reaction_builder.name, reaction_builder.reaction_fn, None)
            });
            reaction_use_ports.insert(reaction_key, use_port_set);
            reaction_effect_ports.insert(reaction_key, effect_port_set);
            reaction_actions.insert(reaction_key, actions_set);
            reaction_aliases.insert(builder_key, reaction_key);
        }
    }
}

/// Aliasing maps from Builder keys to runtime keys
#[derive(Default)]
pub struct BuilderAliases {
    pub reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    pub action_aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

/// A map of partitions: each Reactor is mapped to one Enclave Reactor.
pub type PartitionMap = SecondaryMap<BuilderReactorKey, BuilderReactorKey>;

impl EnvBuilder {
    /// Process the connections and reduce them to a set of port bindings.
    pub(super) fn build_connections(
        &mut self,
        partition_map: &mut PartitionMap,
    ) -> Result<PortBindings, BuilderError> {
        let mut port_bindings = PortBindings::default();
        for connection in std::mem::take(&mut self.connection_builders).iter_mut() {
            connection.build(self, partition_map, &mut port_bindings)?;
        }
        Ok(port_bindings)
    }

    /// Build the enclave partitioning map.
    pub(super) fn build_partition_map(&self) -> PartitionMap {
        let graph = self.build_reactor_graph();

        let mut partitions = vec![];
        let mut node_stack = vec![self.reactor_builders.keys().next().unwrap()];

        petgraph::visit::depth_first_search(&graph, self.reactor_builders.keys(), |event| {
            match event {
                petgraph::visit::DfsEvent::Discover(key, _) => {
                    if self.reactor_builders[key].is_enclave {
                        node_stack.push(key);
                    }
                    partitions.push((key, *node_stack.last().unwrap()));
                }
                petgraph::visit::DfsEvent::Finish(key, _) => {
                    if self.reactor_builders[key].is_enclave {
                        node_stack.pop();
                    }
                }
                _ => {}
            }
        });

        // Not sure if sorting is necessary, with the depth first search the partitions should be sorted already
        //partitions.sort_by_key(|(_, partition_key)| *partition_key);

        /*
        partitions
            .into_iter()
            .chunk_by(|(_, partition_key)| *partition_key)
            .into_iter()
            .map(|(key, chunk)| (key, chunk.map(|(key, _)| key).collect()))
            .collect()
            */
        partitions.into_iter().collect()
    }

    fn build_runtime_reactors(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
    ) -> Result<(), BuilderError> {
        for (builder_reactor_key, reactor) in self.reactor_builders.drain() {
            let partition_key = partition_map[builder_reactor_key];
            let partition = &mut partitions[partition_key];
            let bank_info = reactor.bank_info.clone();
            let runtime_reactor_key = partition
                .enclave
                .insert_reactor(reactor.into_runtime(), bank_info);
            partition
                .aliases
                .reactor_aliases
                .insert(builder_reactor_key, runtime_reactor_key);
        }
        Ok(())
    }

    fn build_runtime_actions(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
    ) -> Result<(), BuilderError> {
        for (builder_action_key, action) in &self.action_builders {
            let partition_key = partition_map[action.parent_reactor_key().unwrap()];
            let partition = &mut partitions[partition_key];

            match action.r#type() {
                ActionType::Timer(_) | ActionType::Shutdown => {
                    let runtime_action_key = partition.enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });

                    partition
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, runtime_action_key);
                }

                ActionType::Standard {
                    is_logical: _,
                    min_delay: _,
                    build_fn,
                } => {
                    let runtime_action_key = partition
                        .enclave
                        .insert_action(|key| build_fn(action.name(), key));

                    partition
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, runtime_action_key);
                }
            }
        }

        Ok(())
    }

    fn build_runtime_ports(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
        port_bindings: &PortBindings,
    ) -> Result<(), BuilderError> {
        let port_groups = self
            .port_builders
            .keys()
            .map(|port_key| (port_key, port_bindings.follow_port_inward(port_key)))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let port = &self.port_builders[inward_port_key];
            let partition_key = partition_map[port.parent_reactor_key().unwrap()];
            let partition = &mut partitions[partition_key];

            let runtime_port_key = partition
                .enclave
                .insert_port(|key| port.build_runtime_port(key));

            partition
                .aliases
                .port_aliases
                .insert(inward_port_key, runtime_port_key);

            partition
                .aliases
                .port_aliases
                .extend(group.map(|(port_key, _inward_key)| (port_key, runtime_port_key)));
        }
        Ok(())
    }

    fn build_runtime_reactions(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
        reaction_levels: &SecondaryMap<BuilderReactionKey, runtime::Level>,
    ) -> Result<(), BuilderError> {
        for (builder_reaction_key, reaction) in self.reaction_builders.drain() {
            let reaction_body = (reaction.reaction_fn)(partitions);

            let partition_key = partition_map[reaction.reactor_key];
            let partition = &mut partitions[partition_key];
            let runtime_reactor_key = partition.aliases.reactor_aliases[reaction.reactor_key];

            let use_ports = reaction
                .port_relations
                .iter()
                .filter_map(|(port_key, tm)| tm.is_uses().then_some(port_key))
                .map(|builder_port_key| partition.aliases.port_aliases[builder_port_key]);

            let effect_ports = reaction
                .port_relations
                .iter()
                .filter_map(|(port_key, tm)| tm.is_effects().then_some(port_key))
                .map(|builder_port_key| partition.aliases.port_aliases[builder_port_key]);

            let actions = reaction
                .action_relations
                .iter()
                .filter_map(|(action_key, tm)| {
                    (tm.is_effects() || tm.is_uses()).then_some(action_key)
                })
                .map(|builder_action_key| partition.aliases.action_aliases[builder_action_key]);

            let runtime_reaction_key = partition.enclave.insert_reaction(
                runtime::Reaction::new(&reaction.name, reaction_body, None),
                runtime_reactor_key,
                use_ports,
                effect_ports,
                actions,
            );

            let level_reaction = (reaction_levels[builder_reaction_key], runtime_reaction_key);

            for (builder_port_key, tm) in &reaction.port_relations {
                let port_key = partition.aliases.port_aliases[builder_port_key];
                if tm.is_triggers() {
                    partition
                        .enclave
                        .insert_port_trigger(port_key, level_reaction);
                }
            }

            for (builder_action_key, tm) in &reaction.action_relations {
                let action_key = partition.aliases.action_aliases[builder_action_key];
                if tm.is_triggers() {
                    partition
                        .enclave
                        .insert_action_trigger(action_key, level_reaction);

                    match self.action_builders[builder_action_key].r#type() {
                        ActionType::Timer(timer_spec) => {
                            partition
                                .enclave
                                .insert_startup_reaction(level_reaction, timer_spec.offset);
                        }
                        ActionType::Shutdown => {
                            partition.enclave.insert_shutdown_reaction(level_reaction);
                        }
                        _ => {}
                    }
                }
            }

            partition
                .aliases
                .reaction_aliases
                .insert(builder_reaction_key, runtime_reaction_key);
        }
        Ok(())
    }

    /// Convert the [`EnvBuilder`] into a  [`tinymap::TinySecondaryMap`] of [`EnclaveParts`], one for each partition.
    pub fn into_runtime_parts(mut self) -> Result<Vec<EnclaveParts>, BuilderError> {
        let mut partition_map = self.build_partition_map();
        let port_bindings = self.build_connections(&mut partition_map)?;

        let mut partitions: SecondaryMap<BuilderReactorKey, EnclaveParts> = partition_map
            .values()
            .unique()
            .map(|reactor_key| (*reactor_key, EnclaveParts::default()))
            .collect();

        let reaction_levels = self.build_runtime_level_map(&port_bindings)?;

        self.build_runtime_reactors(&partition_map, &mut partitions)?;
        self.build_runtime_actions(&partition_map, &mut partitions)?;
        self.build_runtime_ports(&partition_map, &mut partitions, &port_bindings)?;
        self.build_runtime_reactions(&partition_map, &mut partitions, &reaction_levels)?;

        /*
        for partition in partitions.values_mut() {
            for (port_key, runtime_port_key) in &partition.aliases.port_aliases {
                let downstream_reactions =
                    collect_transitive_port_triggers(port_key, &self.port_builders, &port_bindings);
                for downstream_reaction in downstream_reactions {
                    let level_reaction = (
                        reaction_levels[downstream_reaction],
                        partition.aliases.reaction_aliases[downstream_reaction],
                    );
                    partition
                        .enclave
                        .insert_port_trigger(*runtime_port_key, level_reaction);
                }
            }
        }
        */

        Ok(partitions.into_iter().map(|(_, parts)| parts).collect())
    }
}
