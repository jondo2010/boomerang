//! Debug impls and output utility methods for the [`EnvBuilder`].

use std::{collections::HashMap, fmt::Debug};

use itertools::Itertools;
use petgraph::prelude::DiGraphMap;

use crate::{BuilderFqn, BuilderPortKey, BuilderReactorKey};

use super::EnvBuilder;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Dependency(BuilderFqn, BuilderFqn);

impl Debug for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\" -> \"{}\"", self.0, self.1)
    }
}

impl EnvBuilder {
    /// Returns a grouped list of (first_key, last_key, fqn) of reactors
    pub fn reactors_debug_grouped(
        &self,
    ) -> Vec<(BuilderReactorKey, Option<BuilderReactorKey>, BuilderFqn)> {
        let reactors_chunked = self
            .reactor_builders
            .keys()
            .map(|reactor_key| (self.reactor_fqn(reactor_key, true).unwrap(), reactor_key))
            .sorted()
            .chunk_by(|(fqn, _)| fqn.clone());
        reactors_chunked
            .into_iter()
            .map(|(fqn, mut group)| {
                let (_, first_key) = group.next().unwrap();
                let last_key = group.last().map(|(_, key)| key);
                (first_key, last_key, fqn)
            })
            .collect()
    }

    /// Returns a grouped list of (first_key, last_key, fqn) of ports
    pub fn ports_debug_grouped(
        &self,
        ports: impl Iterator<Item = BuilderPortKey>,
    ) -> Vec<(BuilderPortKey, Option<BuilderPortKey>, BuilderFqn)> {
        let ports_chunked = ports
            .map(|port_key| (self.port_fqn(port_key, true).unwrap(), port_key))
            .sorted()
            .chunk_by(|(fqn, _)| fqn.clone());
        ports_chunked
            .into_iter()
            .map(|(fqn, mut group)| {
                let (_, first_key) = group.next().unwrap();
                let last_key = group.last().map(|(_, key)| key);
                (first_key, last_key, fqn)
            })
            .collect()
    }

    /// Build a DAG of Reactors, grouped by bank
    pub fn build_reactor_graph_grouped(&self) -> DiGraphMap<BuilderReactorKey, ()> {
        let reactors_grouped = self.reactors_debug_grouped();

        let mut graph =
            DiGraphMap::from_edges(reactors_grouped.iter().filter_map(|(first_key, _, _)| {
                self.reactor_builders[*first_key]
                    .parent_reactor_key
                    .map(|parent_key| (parent_key, *first_key))
            }));

        // ensure all Reactors are represented
        reactors_grouped.iter().for_each(|(key, _, _)| {
            graph.add_node(*key);
        });

        graph
    }

    fn reactors_debug_map(&self) -> HashMap<String, String> {
        let reactors_chunked = self.reactors_debug_grouped();
        reactors_chunked
            .into_iter()
            .map(|(first_key, last_key, fqn)| {
                if let Some(last_key) = last_key {
                    (format!("{first_key:?}..{last_key:?}"), fqn.to_string())
                } else {
                    (format!("{first_key:?}"), fqn.to_string())
                }
            })
            .collect()
    }

    fn ports_debug_map(&self) -> HashMap<String, String> {
        let ports = self.port_builders.keys();
        let ports_grouped = self.ports_debug_grouped(ports);
        ports_grouped
            .into_iter()
            .map(|(first_key, last_key, fqn)| {
                if let Some(last_key) = last_key {
                    (format!("{first_key:?}..{last_key:?}"), fqn.to_string())
                } else {
                    (format!("{first_key:?}"), fqn.to_string())
                }
            })
            .collect()
    }

    fn actions_debug_map(&self) -> HashMap<String, String> {
        let actions_chunked = self
            .action_builders
            .keys()
            .map(|action_key| (action_key, self.action_fqn(action_key, true).unwrap()))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .chunk_by(|(_, fqn)| fqn.clone());

        actions_chunked
            .into_iter()
            .map(|(fqn, mut group)| {
                let (first_key, _) = group.next().unwrap();
                let ty = self.action_builders[first_key].r#type();
                if let Some((last_key, _)) = group.last() {
                    (
                        format!("{first_key:?}..{last_key:?}"),
                        format!("{fqn} : <{ty:?}>"),
                    )
                } else {
                    (format!("{first_key:?}"), format!("{fqn} : <{ty:?}>"))
                }
            })
            .collect()
    }

    fn reactions_debug_map(&self) -> HashMap<String, String> {
        let reactions_chunked = self
            .reaction_builders
            .keys()
            .map(|reaction_key| (reaction_key, self.reaction_fqn(reaction_key, true).unwrap()))
            .sorted_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
            .chunk_by(|(_, fqn)| fqn.clone());

        let level_map = self.build_runtime_level_map().ok();

        reactions_chunked
            .into_iter()
            .map(|(fqn, mut group)| {
                let (first_key, _) = group.next().unwrap();
                let last_key = group.last().map(|(key, _)| key);
                let res_key = if let Some(last_key) = last_key {
                    format!("{first_key:?}..{last_key:?}")
                } else {
                    format!("{first_key:?}")
                };

                let res_level = if let Some(level_map) = &level_map {
                    if let Some(last_key) = last_key {
                        format!("{:?}..{:?}", level_map[first_key], level_map[last_key])
                    } else {
                        format!("{:?}", level_map[first_key])
                    }
                } else {
                    // There was a cycle in the reaction graph, so don't show the reaction levels.
                    let priority = self.reaction_builders[first_key].priority();
                    format!("{priority}")
                };

                (format!("{res_key}, {res_level}"), fqn.to_string())
            })
            .collect()
    }

    fn reaction_edges_debug_map(&self) -> Vec<Dependency> {
        let edges = self
            .reaction_dependency_edges()
            .map(|(a, b)| {
                let a_fqn = self.reaction_fqn(a, true).unwrap();
                let b_fqn = self.reaction_fqn(b, true).unwrap();
                Dependency(a_fqn, b_fqn)
            })
            .sorted()
            .chunk_by(|dep| dep.clone())
            .into_iter()
            .map(|(dep, _group)| dep)
            .collect_vec();
        edges
    }
}

impl Debug for EnvBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reactors = self.reactors_debug_map();
        let actions = self.actions_debug_map();
        let ports = self.ports_debug_map();
        let edges = self.reaction_edges_debug_map();
        let reactions = self.reactions_debug_map();

        //let runtime_port_parts = self.build_runtime_ports();
        //let port_aliases = runtime_port_parts
        //    .port_aliases
        //    .iter()
        //    .map(|(builder_port_key, port_key)| {
        //        (
        //            self.port_fqn(builder_port_key, false).unwrap(),
        //            format!("{:?}", runtime_port_parts.ports[*port_key]),
        //        )
        //    })
        //    .collect::<BTreeMap<_, _>>();

        f.debug_struct("EnvBuilder")
            .field("reactors", &reactors)
            .field("actions", &actions)
            .field("ports", &ports)
            //.field("runtime_port_aliases", &port_aliases)
            .field("reaction_dependency_edges", &edges)
            .field("reactions", &reactions)
            .finish()
    }
}
