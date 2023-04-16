//! Impls and methods for transforming a `EnvBuilder` into an `Env`

use std::collections::{BTreeSet, HashMap};

use itertools::Itertools;
use petgraph::{prelude::DiGraphMap, EdgeDirection};
use slotmap::{SecondaryMap, SlotMap};

use crate::{
    builder::{
        BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey, BuilderReactorKey,
        ReactorBuilder,
    },
    runtime,
};

use super::EnvBuilder;

/// Return type for building runtime parts
#[derive(Debug)]
pub(crate) struct RuntimePortParts {
    /// All runtime Ports
    pub ports: tinymap::TinyMap<runtime::keys::PortKey, Box<dyn runtime::BasePort>>,
    /// For each Port, a set of Reactions triggered by it
    pub port_triggers: tinymap::TinySecondaryMap<runtime::keys::PortKey, Vec<BuilderReactionKey>>,
    /// A mapping from `BuilderPortKey`s to aliased [`runtime::PortKey`]s.
    pub aliases: SecondaryMap<BuilderPortKey, runtime::keys::PortKey>,
}

#[derive(Debug)]
pub(crate) struct RuntimeActionParts {
    pub actions: tinymap::TinyMap<runtime::keys::ActionKey, runtime::Action>,
    pub action_triggers:
        tinymap::TinySecondaryMap<runtime::keys::ActionKey, Vec<BuilderReactionKey>>,
    pub aliases: SecondaryMap<BuilderActionKey, runtime::keys::ActionKey>,
}

impl EnvBuilder {
    /// Follow the inward_binding's of Ports to the source
    fn follow_port_inward_binding(&self, port_key: BuilderPortKey) -> BuilderPortKey {
        let mut cur_key = port_key;
        while let Some(new_idx) = self
            .port_builders
            .get(cur_key)
            .and_then(|port| port.get_inward_binding())
        {
            cur_key = new_idx;
        }
        cur_key
    }

    /// Transitively collect all Reactions triggered by this Port being set
    fn collect_transitive_port_triggers(
        &self,
        port_key: BuilderPortKey,
    ) -> SecondaryMap<BuilderReactionKey, ()> {
        let mut all_triggers = SecondaryMap::new();
        let mut port_set = BTreeSet::<BuilderPortKey>::new();
        port_set.insert(port_key);
        while !port_set.is_empty() {
            let port_key = port_set.pop_first().unwrap();
            let port_builder = &self.port_builders[port_key];
            all_triggers.extend(port_builder.get_triggers().iter().map(|&key| (key, ())));
            port_set.extend(port_builder.get_outward_bindings());
        }
        all_triggers
    }

    /// Build an iterator of all Reaction dependency edges in the graph
    pub(crate) fn reaction_dependency_edges(
        &self,
    ) -> impl Iterator<Item = (BuilderReactionKey, BuilderReactionKey)> + '_ {
        let deps = self
            .reaction_builders
            .iter()
            .flat_map(move |(reaction_key, reaction)| {
                // Connect all reactions this reaction depends upon
                reaction
                    .input_ports
                    .keys()
                    .flat_map(move |port_key| {
                        let source_port_key = self.follow_port_inward_binding(port_key);
                        self.port_builders[source_port_key].get_antideps()
                    })
                    .map(move |dep_key| (reaction_key, dep_key))
            });

        // For all Reactions within a Reactor, create a chain of dependencies by priority. This
        // ensures that Reactions within a Reactor always end up at unique levels.
        let internal = self.reactor_builders.values().flat_map(move |reactor| {
            reactor
                .reactions
                .keys()
                .sorted_by_key(|&reaction_key| self.reaction_builders[reaction_key].priority)
                .tuple_windows()
        });
        deps.chain(internal)
    }

    /// Build a DAG of Reactions
    pub(crate) fn build_reaction_graph(&self) -> DiGraphMap<BuilderReactionKey, ()> {
        let mut graph =
            DiGraphMap::from_edges(self.reaction_dependency_edges().map(|(a, b)| (b, a)));
        // Ensure all ReactionIndicies are represented
        self.reaction_builders.keys().for_each(|key| {
            graph.add_node(key);
        });

        graph
    }

    /// Build a DAG of Reactors
    pub(crate) fn build_reactor_graph(&self) -> DiGraphMap<BuilderReactorKey, ()> {
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
    /// See https://en.m.wikipedia.org/wiki/Coffman%E2%80%93Graham_algorithm
    pub(crate) fn build_runtime_level_map(
        &self,
    ) -> Result<SecondaryMap<BuilderReactionKey, usize>, BuilderError> {
        use petgraph::{algo::tred, graph::DefaultIx, graph::NodeIndex};

        let mut graph = self.build_reaction_graph().into_graph::<DefaultIx>();

        // Transitive reduction and closures
        let toposort = petgraph::algo::toposort(&graph, None).map_err(|e| {
            // A Cycle was found in the reaction graph.
            // let fas = petgraph::algo::greedy_feedback_arc_set(&graph);
            // let cycle = petgraph::prelude::DiGraphMap::<BuilderReactionKey, ()>::from_edges(fas);

            BuilderError::ReactionGraphCycle {
                what: graph[e.node_id()],
            }
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

        let mut levels: HashMap<_, usize> = HashMap::new();
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
            .group_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let downstream_reactions = self
                .collect_transitive_port_triggers(inward_port_key)
                .keys()
                .collect_vec();

            let runtime_port_key =
                runtime_ports.insert(self.port_builders[inward_port_key].create_runtime_port());

            port_triggers.insert(runtime_port_key, downstream_reactions);

            alias_map.extend(group.map(move |(port_key, _inward_key)| (port_key, runtime_port_key)))
        }

        RuntimePortParts {
            ports: runtime_ports,
            port_triggers,
            aliases: alias_map,
        }
    }

    /// Construct runtime action structures from the builders.
    fn build_runtime_actions(&self) -> SecondaryMap<BuilderReactorKey, RuntimeActionParts> {
        let mut action_parts = SecondaryMap::new();
        for (builder_key, reactor_builder) in self.reactor_builders.iter() {
            let mut runtime_actions = tinymap::TinyMap::new();
            let mut action_triggers = tinymap::TinySecondaryMap::new();
            let mut action_alias = SecondaryMap::new();

            for (builder_action_key, action_builder) in reactor_builder.actions.iter() {
                let runtime_action_key = runtime_actions
                    .insert_with_key(|action_key| action_builder.build_runtime(action_key));
                let triggers = action_builder.triggers.keys().collect();
                action_triggers.insert(runtime_action_key, triggers);
                action_alias.insert(builder_action_key, runtime_action_key);
            }

            action_parts.insert(
                builder_key,
                RuntimeActionParts {
                    actions: runtime_actions,
                    action_triggers,
                    aliases: action_alias,
                },
            );
        }
        action_parts
    }
}

/// Build a new `SlotMap` of `runtime::Reactor` from `ReactorBuilder`s.
fn build_runtime_reactors(
    reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
    reaction_levels: &SecondaryMap<BuilderReactionKey, usize>,
    reaction_aliases: &SecondaryMap<BuilderReactionKey, runtime::keys::ReactionKey>,
    mut action_parts: SecondaryMap<BuilderReactorKey, RuntimeActionParts>,
) -> (
    tinymap::TinyMap<runtime::keys::ReactorKey, runtime::Reactor>,
    SecondaryMap<BuilderReactorKey, runtime::keys::ReactorKey>,
) {
    let mut runtime_reactors = tinymap::TinyMap::with_capacity(reactor_builders.len());
    let mut reactor_alias = SecondaryMap::new();

    for (builder_key, reactor_builder) in reactor_builders.into_iter() {
        let RuntimeActionParts {
            actions: runtime_actions,
            action_triggers,
            ..
        } = action_parts.remove(builder_key).unwrap();

        // Build the runtime_action_triggers from the action_triggers part, mapping
        // BuilderReactionKey -> runtime::ReactionKey
        let runtime_action_triggers = action_triggers
            .into_iter()
            .map(|(action_key, triggers)| {
                let downstream = triggers
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

        let reactor_key = runtime_reactors
            .insert(reactor_builder.build_runtime(runtime_actions, runtime_action_triggers));
        reactor_alias.insert(builder_key, reactor_key);
    }
    (runtime_reactors, reactor_alias)
}

impl TryInto<runtime::Env> for EnvBuilder {
    type Error = BuilderError;
    fn try_into(self) -> Result<runtime::Env, Self::Error> {
        let reaction_levels = self.build_runtime_level_map()?;
        let RuntimePortParts {
            ports: mut runtime_ports,
            port_triggers,
            aliases: port_aliases,
        } = self.build_runtime_ports();
        let action_parts = self.build_runtime_actions();

        let mut runtime_reactions = tinymap::TinyMap::with_capacity(self.reaction_builders.len());
        let mut reactions_aliases = SecondaryMap::new();
        let mut reaction_reactor_aliases = SecondaryMap::new();
        for (builder_key, reaction_builder) in self.reaction_builders.into_iter() {
            reaction_reactor_aliases.insert(builder_key, reaction_builder.reactor_key);
            let action_aliases = &action_parts[reaction_builder.reactor_key].aliases;
            let reaction_key = runtime_reactions.insert(reaction_builder.build_reaction(
                runtime::keys::ReactorKey::default(),
                &port_aliases,
                action_aliases,
            ));
            reactions_aliases.insert(builder_key, reaction_key);
        }

        // Update the the Ports with triggered downstream Reactions.
        for (port_key, triggers) in port_triggers.into_iter() {
            let downstream = triggers
                .into_iter()
                .map(|builder_reaction_key| {
                    (
                        reaction_levels[builder_reaction_key],
                        reactions_aliases[builder_reaction_key],
                    )
                })
                .collect();
            runtime_ports[port_key].set_downstream(downstream);
        }

        let (runtime_reactors, reactor_aliases) = build_runtime_reactors(
            self.reactor_builders,
            &reaction_levels,
            &reactions_aliases,
            action_parts,
        );

        // Update the Reactions with the Reactor keys
        for (builder_reaction_key, builder_reactor_key) in reaction_reactor_aliases {
            let runtime_reactor_key = reactor_aliases[builder_reactor_key];
            let runtime_reaction_key = reactions_aliases[builder_reaction_key];
            runtime_reactions[runtime_reaction_key].set_reactor_key(runtime_reactor_key);
        }

        Ok(runtime::Env {
            reactors: runtime_reactors,
            ports: runtime_ports,
            reactions: runtime_reactions,
        })
    }
}
