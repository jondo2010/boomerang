use boomerang_runtime::{self as runtime, LevelReactionKey};
use itertools::Itertools;
use slotmap::{SecondaryMap, SlotMap};

use crate::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey, BuilderReactorKey,
    ReactionBuilder, ReactorBuilder,
};

use super::EnvBuilder;

/// Return type for building runtime parts
#[derive(Debug)]
pub(crate) struct RuntimePortParts {
    /// All runtime Ports
    pub ports: tinymap::TinyMap<runtime::PortKey, Box<dyn runtime::BasePort>>,
    /// For each Port, a set of Reactions triggered by it
    pub port_triggers: tinymap::TinySecondaryMap<runtime::PortKey, Vec<BuilderReactionKey>>,
    /// A mapping from `BuilderPortKey`s to aliased [`runtime::PortKey`]s.
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

#[derive(Debug)]
struct RuntimeActionParts {
    actions: tinymap::TinyMap<runtime::ActionKey, runtime::Action>,
    action_triggers: tinymap::TinySecondaryMap<runtime::ActionKey, Vec<BuilderReactionKey>>,
    aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
}

#[derive(Debug)]
struct RuntimeReactionParts {
    reactions: tinymap::TinyMap<runtime::ReactionKey, runtime::Reaction>,
    use_ports: tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::PortKey>>,
    effect_ports:
        tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::PortKey>>,
    actions: tinymap::TinySecondaryMap<runtime::ReactionKey, tinymap::KeySet<runtime::ActionKey>>,
    /// Aliases from BuilderReactionKey to runtime::ReactionKey
    reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    reaction_reactor_aliases: SecondaryMap<BuilderReactionKey, BuilderReactorKey>,
}

#[derive(Debug)]
struct RuntimeReactorParts {
    runtime_reactors: tinymap::TinyMap<runtime::ReactorKey, Box<dyn runtime::BaseReactor>>,
    reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    reactor_bank_indices: tinymap::TinySecondaryMap<runtime::ReactorKey, Option<runtime::BankInfo>>,
}

fn build_runtime_reactions(
    reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    port_aliases: &SecondaryMap<BuilderPortKey, runtime::PortKey>,
    action_aliases: &SecondaryMap<BuilderActionKey, runtime::ActionKey>,
) -> RuntimeReactionParts {
    let mut runtime_reactions = tinymap::TinyMap::with_capacity(reaction_builders.len());
    let mut reaction_use_ports = tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
    let mut reaction_effect_ports =
        tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
    let mut reaction_actions = tinymap::TinySecondaryMap::with_capacity(reaction_builders.len());
    let mut reaction_aliases = SecondaryMap::new();
    let mut reaction_reactor_aliases = SecondaryMap::new();

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

    RuntimeReactionParts {
        reactions: runtime_reactions,
        use_ports: reaction_use_ports,
        effect_ports: reaction_effect_ports,
        actions: reaction_actions,
        reaction_aliases,
        reaction_reactor_aliases,
    }
}

fn build_runtime_reactors(
    reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
) -> RuntimeReactorParts {
    let mut runtime_reactors = tinymap::TinyMap::with_capacity(reactor_builders.len());
    let mut reactor_aliases = SecondaryMap::new();
    let mut reactor_bank_indices = tinymap::TinySecondaryMap::with_capacity(reactor_builders.len());

    for (builder_key, reactor_builder) in reactor_builders.into_iter() {
        let bank_info = reactor_builder.bank_info.clone();
        let reactor_key = runtime_reactors.insert(reactor_builder.into_runtime());
        reactor_aliases.insert(builder_key, reactor_key);
        reactor_bank_indices.insert(reactor_key, bank_info);
    }

    RuntimeReactorParts {
        runtime_reactors,
        reactor_aliases,
        reactor_bank_indices,
    }
}

/// Aliasing maps from Builder keys to runtime keys
pub struct BuilderAliases {
    pub reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    pub action_aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

impl EnvBuilder {
    /// Construct runtime port structures from the builders.
    pub(crate) fn build_runtime_ports(&self) -> RuntimePortParts {
        let mut runtime_ports = tinymap::TinyMap::new();
        let mut port_triggers = tinymap::TinySecondaryMap::new();
        let mut alias_map = SecondaryMap::new();

        let port_groups = self
            .port_builders
            .keys()
            .map(|port_key| (port_key, self.follow_port_inward_binding(port_key)))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let downstream_reactions = self
                .collect_transitive_port_triggers(inward_port_key)
                .keys()
                .collect_vec();

            let runtime_port_key = runtime_ports.insert_with_key(|key| {
                self.port_builders[inward_port_key].create_runtime_port(key)
            });

            port_triggers.insert(runtime_port_key, downstream_reactions);

            alias_map.extend(group.map(move |(port_key, _inward_key)| (port_key, runtime_port_key)))
        }

        RuntimePortParts {
            ports: runtime_ports,
            port_triggers,
            port_aliases: alias_map,
        }
    }

    /// Construct runtime action structures from the builders.
    fn build_runtime_actions(&self) -> RuntimeActionParts {
        let mut runtime_actions = tinymap::TinyMap::new();
        let mut action_triggers = tinymap::TinySecondaryMap::new();
        let mut action_alias = SecondaryMap::new();

        for (builder_action_key, action_builder) in self.action_builders.iter() {
            let runtime_action_key = runtime_actions
                .insert_with_key(|action_key| action_builder.build_runtime(action_key));
            let triggers = action_builder.triggers.keys().collect();
            action_triggers.insert(runtime_action_key, triggers);
            action_alias.insert(builder_action_key, runtime_action_key);
        }

        RuntimeActionParts {
            actions: runtime_actions,
            action_triggers,
            aliases: action_alias,
        }
    }

    /// Convert the `EnvBuilder` into a [`runtime::Env`], [`runtime::ReactionGraph`] and
    /// [`BuilderAliases`]
    pub fn into_runtime_parts(
        self,
    ) -> Result<(runtime::Env, runtime::ReactionGraph, BuilderAliases), BuilderError> {
        let reaction_levels = self.build_runtime_level_map()?;

        let RuntimePortParts {
            ports: runtime_ports,
            port_triggers,
            port_aliases,
        } = self.build_runtime_ports();

        let RuntimeActionParts {
            actions: runtime_actions,
            action_triggers,
            aliases: action_aliases,
        } = self.build_runtime_actions();

        let RuntimeReactionParts {
            reactions: runtime_reactions,
            use_ports: reaction_use_ports,
            effect_ports: reaction_effect_ports,
            actions: reaction_actions,
            reaction_aliases,
            reaction_reactor_aliases,
        } = build_runtime_reactions(self.reaction_builders, &port_aliases, &action_aliases);

        let RuntimeReactorParts {
            runtime_reactors,
            reactor_aliases,
            reactor_bank_indices,
        } = build_runtime_reactors(self.reactor_builders);

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

        let startup_reactions = runtime_actions
            .iter()
            .filter_map(|(action_key, action)| {
                if let runtime::Action::Startup = action {
                    Some(runtime_action_triggers[action_key].iter().copied())
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        let shutdown_reactions = runtime_actions
            .iter()
            .filter_map(|(action_key, action)| {
                if let runtime::Action::Shutdown = action {
                    Some(runtime_action_triggers[action_key].iter().copied())
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        let reaction_set_limits = runtime::ReactionSetLimits {
            max_level: reaction_levels.values().copied().max().unwrap_or_default(),
            num_keys: runtime_reactions.len(),
        };

        // Sanity checks:
        assert_eq!(runtime_port_triggers.len(), runtime_ports.len());
        assert_eq!(runtime_action_triggers.len(), runtime_actions.len());
        assert_eq!(reaction_use_ports.len(), runtime_reactions.len());
        assert_eq!(reaction_effect_ports.len(), runtime_reactions.len());
        assert_eq!(reaction_actions.len(), runtime_reactions.len());
        assert_eq!(reaction_reactors.len(), runtime_reactions.len());

        Ok((
            runtime::Env {
                reactors: runtime_reactors,
                actions: runtime_actions,
                ports: runtime_ports,
                reactions: runtime_reactions,
            },
            runtime::ReactionGraph {
                port_triggers: runtime_port_triggers,
                action_triggers: runtime_action_triggers,
                startup_reactions,
                shutdown_reactions,
                reaction_set_limits,
                reaction_use_ports,
                reaction_effect_ports,
                reaction_actions,
                reaction_reactors,
                reactor_bank_infos: reactor_bank_indices,
            },
            BuilderAliases {
                reactor_aliases,
                reaction_aliases,
                action_aliases,
                port_aliases,
            },
        ))
    }
}
