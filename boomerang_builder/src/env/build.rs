use boomerang_runtime::{self as runtime, LevelReactionKey};
use itertools::Itertools;
use slotmap::{SecondaryMap, SlotMap};

use crate::{
    connection::PortBindings, ActionBuilder, ActionType, BasePortBuilder, BuilderActionKey,
    BuilderError, BuilderPortKey, BuilderReactionKey, BuilderReactorKey, ParentReactorBuilder,
    ReactionBuilder, ReactorBuilder, TriggerMode,
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
#[derive(Debug)]
pub struct EnclaveParts {
    pub env: runtime::Env,
    pub graph: runtime::ReactionGraph,
    /// Aliases from Builder keys to runtime keys
    pub aliases: BuilderAliases,
}

/// Partition the port builders into the given partitions
pub(super) fn partition_port_builders(
    port_builders: &SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
    partitions: &PartitionMap,
) -> SecondaryMap<BuilderReactorKey, Vec<BuilderPortKey>> {
    partitions
        .iter()
        .map(|(key, partition)| {
            let partitioned_ports = partition
                .iter()
                .flat_map(|reactor_key| {
                    port_builders.iter().filter_map(|(port_key, port_builder)| {
                        if port_builder.parent_reactor_key() == Some(*reactor_key) {
                            Some(port_key)
                        } else {
                            None
                        }
                    })
                })
                .collect::<Vec<_>>();

            (key, partitioned_ports)
        })
        .collect()
}

/// Partition the action builders into the given partitions
fn partition_action_builders(
    mut action_builders: SlotMap<BuilderActionKey, ActionBuilder>,
    partitions: &PartitionMap,
) -> SecondaryMap<BuilderReactorKey, Vec<(BuilderActionKey, ActionBuilder)>> {
    partitions
        .iter()
        .map(|(key, partition)| {
            let partitioned_action_keys = partition
                .iter()
                .flat_map(|reactor_key| {
                    action_builders.iter().filter_map(|(action_key, action)| {
                        if action.parent_reactor_key() == Some(*reactor_key) {
                            Some(action_key)
                        } else {
                            None
                        }
                    })
                })
                .collect::<Vec<_>>();

            let partitioned_actions = partitioned_action_keys
                .into_iter()
                .map(|action_key| (action_key, action_builders.remove(action_key).unwrap()))
                .collect::<Vec<_>>();

            (key, partitioned_actions)
        })
        .collect()
}

/// Partition the reaction builders into the given partitions
pub(super) fn partition_reaction_builders(
    mut reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    partitions: &PartitionMap,
) -> SecondaryMap<BuilderReactorKey, Vec<(BuilderReactionKey, ReactionBuilder)>> {
    partitions
        .iter()
        .map(|(key, partition)| {
            // Create the set of reactions for this partition
            let partitioned_reaction_keys = partition
                .iter()
                .flat_map(|reactor_key| {
                    reaction_builders
                        .iter()
                        .filter_map(|(reaction_key, reaction)| {
                            if reaction.reactor_key == *reactor_key {
                                Some(reaction_key)
                            } else {
                                None
                            }
                        })
                })
                .collect::<Vec<_>>();

            let partitioned_reactions = partitioned_reaction_keys
                .into_iter()
                .map(|reaction_key| {
                    (
                        reaction_key,
                        reaction_builders.remove(reaction_key).unwrap(),
                    )
                })
                .collect::<Vec<_>>();

            (key, partitioned_reactions)
        })
        .collect()
}

/// Partition the reactor builders into the given partitions
pub(super) fn partition_reactor_builders(
    mut reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
    partitions: &PartitionMap,
) -> SecondaryMap<BuilderReactorKey, Vec<(BuilderReactorKey, ReactorBuilder)>> {
    partitions
        .iter()
        .map(|(key, partition)| {
            let partitioned_reactors = partition
                .iter()
                .map(|&reactor_key| {
                    let builder = reactor_builders.remove(reactor_key).unwrap();
                    (reactor_key, builder)
                })
                .collect::<Vec<_>>();

            (key, partitioned_reactors)
        })
        .collect()
}

pub(super) fn build_runtime_reactors<I>(reactor_builders: I) -> RuntimeReactorParts
where
    I: IntoIterator<Item = (BuilderReactorKey, ReactorBuilder)>,
    I::IntoIter: ExactSizeIterator,
{
    let reactor_builders = reactor_builders.into_iter();

    let mut runtime_reactors = tinymap::TinyMap::with_capacity(reactor_builders.len());
    let mut reactor_aliases = SecondaryMap::new();
    let mut reactor_bank_indices = tinymap::TinySecondaryMap::with_capacity(reactor_builders.len());

    for (builder_key, builder) in reactor_builders {
        let bank_info = builder.bank_info.clone();
        let reactor_key = runtime_reactors.insert(builder.into_runtime());
        reactor_aliases.insert(builder_key, reactor_key);
        reactor_bank_indices.insert(reactor_key, bank_info);
    }

    RuntimeReactorParts {
        runtime_reactors,
        reactor_aliases,
        reactor_bank_indices,
    }
}

/// Intermediate parts used to build the runtime structures
#[derive(Default)]
struct IntermediateParts {
    port_parts: RuntimePortParts,
    action_parts: RuntimeActionParts,
    reaction_parts: RuntimeReactionParts,
    reactor_parts: RuntimeReactorParts,
}

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
pub struct BuilderAliases {
    pub reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    pub action_aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

type PartitionMap = SecondaryMap<BuilderReactorKey, Vec<BuilderReactorKey>>;

impl EnvBuilder {
    /// Process the connections and reduce them to a set of port bindings.
    pub(super) fn reduce_connections(&mut self) -> Result<PortBindings, BuilderError> {
        let mut port_bindings = PortBindings::default();
        for connection in std::mem::take(&mut self.connection_builders).iter_mut() {
            connection.build(self, &mut port_bindings)?;
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
        partitions.sort_by_key(|(_, partition_key)| *partition_key);

        partitions
            .into_iter()
            .chunk_by(|(_, partition_key)| *partition_key)
            .into_iter()
            .map(|(key, chunk)| (key, chunk.map(|(key, _)| key).collect()))
            .collect()
    }

    /// Convert the [`EnvBuilder`] into a  Vec of [`EnclaveParts`], one for each partition.
    pub fn into_runtime_parts(mut self) -> Result<Vec<EnclaveParts>, BuilderError> {
        let partition_map = self.build_partition_map();

        let port_bindings = self.reduce_connections()?;

        let reaction_levels = self.build_runtime_level_map(&port_bindings)?;

        let mut partitioned_port_keys =
            partition_port_builders(&self.port_builders, &partition_map);

        let mut partitioned_actions =
            partition_action_builders(self.action_builders, &partition_map);

        let mut partitioned_reactions =
            partition_reaction_builders(self.reaction_builders, &partition_map);

        let mut partitioned_reactors =
            partition_reactor_builders(self.reactor_builders, &partition_map);

        todo!();

        #[cfg(feature = "disable")]
        partition_map
            .iter()
            .map(|(key, _partition)| {
                let mut inter_parts = IntermediateParts::default();

                let ports_partition = partitioned_port_keys.remove(key).unwrap();
                inter_parts.build_runtime_ports(
                    &self.port_builders,
                    ports_partition.into_iter(),
                    &port_bindings,
                );

                let actions_partition = partitioned_actions.remove(key).unwrap();
                let reactions_partition = partitioned_reactions.remove(key).unwrap();

                inter_parts.build_runtime_actions(actions_partition, &reactions_partition);
                inter_parts.build_runtime_reactions(reactions_partition);

                let reactor_partition = partitioned_reactors.remove(key).unwrap();
                let RuntimeReactorParts {
                    runtime_reactors,
                    reactor_aliases,
                    reactor_bank_indices,
                } = build_runtime_reactors(reactor_partition);

                // Mapping of Reaction to its owning Reactor
                let reaction_reactors: tinymap::TinySecondaryMap<
                    runtime::ReactionKey,
                    runtime::ReactorKey,
                > = reaction_reactor_aliases
                    .into_iter()
                    .map(|(builder_reaction_key, builder_reactor_key)| {
                        (
                            reaction_aliases[builder_reaction_key],
                            reactor_aliases[builder_reactor_key],
                        )
                    })
                    .collect();

                let runtime_port_triggers: tinymap::TinySecondaryMap<
                    runtime::PortKey,
                    Vec<LevelReactionKey>,
                > = port_triggers
                    .into_iter()
                    .map(|(port_key, triggers)| {
                        let downstream = triggers
                            .into_iter()
                            .map(|builder_reaction_key| {
                                (
                                    reaction_levels[builder_reaction_key],
                                    reaction_aliases[builder_reaction_key],
                                )
                            })
                            .collect();
                        (port_key, downstream)
                    })
                    .collect();

                let runtime_action_triggers: tinymap::TinySecondaryMap<
                    runtime::ActionKey,
                    Vec<LevelReactionKey>,
                > = action_triggers
                    .into_iter()
                    .map(|(action_key, trigger)| {
                        let downstream = trigger
                            .into_iter()
                            .map(|builder_reaction_key| {
                                (
                                    reaction_levels[builder_reaction_key],
                                    reaction_aliases[builder_reaction_key],
                                )
                            })
                            .collect();
                        (action_key, downstream)
                    })
                    .collect();

                // Add levels and pull runtime keys from the aliases
                let startup_reactions = startup_reactions
                    .iter()
                    .map(|builder_reaction_key| {
                        let level = reaction_levels[*builder_reaction_key];
                        let reaction_key = reaction_aliases[*builder_reaction_key];
                        (level, reaction_key)
                    })
                    .collect();

                let shutdown_reactions = shutdown_reactions
                    .iter()
                    .map(|builder_reaction_key| {
                        let level = reaction_levels[*builder_reaction_key];
                        let reaction_key = reaction_aliases[*builder_reaction_key];
                        (level, reaction_key)
                    })
                    .collect();

                // Sanity checks:
                assert_eq!(runtime_port_triggers.len(), runtime_ports.len());
                assert_eq!(runtime_action_triggers.len(), runtime_actions.len());
                assert_eq!(reaction_use_ports.len(), runtime_reactions.len());
                assert_eq!(reaction_effect_ports.len(), runtime_reactions.len());
                assert_eq!(reaction_actions.len(), runtime_reactions.len());
                assert_eq!(reaction_reactors.len(), runtime_reactions.len());

                Ok(EnclaveParts {
                    env: runtime::Env {
                        reactors: runtime_reactors,
                        actions: runtime_actions,
                        ports: runtime_ports,
                        reactions: runtime_reactions,
                    },
                    graph: runtime::ReactionGraph {
                        port_triggers: runtime_port_triggers,
                        action_triggers: runtime_action_triggers,
                        startup_reactions,
                        shutdown_reactions,
                        reaction_use_ports,
                        reaction_effect_ports,
                        reaction_actions,
                        reaction_reactors,
                        reactor_bank_infos: reactor_bank_indices,
                    },
                    aliases: BuilderAliases {
                        reactor_aliases,
                        reaction_aliases,
                        action_aliases,
                        port_aliases,
                    },
                })
            })
            .collect()
    }
}
