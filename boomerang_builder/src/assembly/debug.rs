//! Debug impls and output utility methods for the [`Assembly`].

use std::fmt::Debug;

use crate::{AssemblyFqn, AssemblyPortKey, AssemblyReactorKey};
use itertools::Itertools;
use petgraph::prelude::DiGraphMap;

use super::{
    build::{RuntimeAliases, RuntimeAssembly},
    Assembly,
};

use boomerang_runtime::fmt_utils as fmt;

impl Assembly {
    /// Returns a grouped list of (first_key, last_key, fqn) of reactors
    pub(crate) fn reactors_debug_grouped(
        &self,
    ) -> Vec<(AssemblyReactorKey, Option<AssemblyReactorKey>, AssemblyFqn)> {
        let reactors_chunked = self
            .reactor_specs
            .keys()
            .map(|reactor_key| (self.fqn_for(reactor_key, true).unwrap(), reactor_key))
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
    pub(crate) fn ports_debug_grouped(
        &self,
        ports: impl Iterator<Item = AssemblyPortKey>,
    ) -> Vec<(AssemblyPortKey, Option<AssemblyPortKey>, AssemblyFqn)> {
        let ports_chunked = ports
            .map(|port_key| (self.fqn_for(port_key, true).unwrap(), port_key))
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
    pub(crate) fn build_reactor_graph_grouped(&self) -> DiGraphMap<AssemblyReactorKey, ()> {
        let reactors_grouped = self.reactors_debug_grouped();

        let mut graph =
            DiGraphMap::from_edges(reactors_grouped.iter().filter_map(|(first_key, _, _)| {
                self.reactor_specs[*first_key]
                    .parent_reactor_key
                    .map(|parent_key| (parent_key, *first_key))
            }));

        // ensure all Reactors are represented
        reactors_grouped.iter().for_each(|(key, _, _)| {
            graph.add_node(*key);
        });

        graph
    }
}

impl Debug for Assembly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.application_topology() {
            Ok(topology) => topology.fmt(f),
            Err(error) => f
                .debug_struct("ApplicationTopologyProjectionError")
                .field("error", &error.to_string())
                .finish(),
        }
    }
}

impl Debug for RuntimeAliases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let enclave_aliases = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.enclave_aliases
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), v.to_string())),
                )
                .finish()
        });

        let reactor_aliases = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.reactor_aliases
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), format!("{v:?}"))),
                )
                .finish()
        });

        let reaction_aliases = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.reaction_aliases
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), format!("{v:?}"))),
                )
                .finish()
        });

        let action_aliases = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.action_aliases
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), format!("{v:?}"))),
                )
                .finish()
        });

        let port_aliases = fmt::from_fn(|f| {
            f.debug_map()
                .entries(
                    self.port_aliases
                        .iter()
                        .map(|(k, v)| (format!("{k:?}"), format!("{v:?}"))),
                )
                .finish()
        });

        f.debug_struct("RuntimeAliases")
            .field("enclave_aliases", &enclave_aliases)
            .field("reactor_aliases", &reactor_aliases)
            .field("reaction_aliases", &reaction_aliases)
            .field("action_aliases", &action_aliases)
            .field("port_aliases", &port_aliases)
            .finish()
    }
}

impl Debug for RuntimeAssembly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let enclaves = fmt::from_fn(|f| {
            f.debug_map()
                .entries(self.enclaves.iter().map(|(k, v)| (format!("{k:?}"), v)))
                .finish()
        });

        f.debug_struct("RuntimeAssembly")
            .field("enclave_map", &enclaves)
            .field("aliases_map", &self.aliases)
            .field("inter_partition_plan", &self.inter_partition_plan)
            .field("federation", &{
                #[cfg(feature = "federated")]
                {
                    &self.federation.as_ref().map(|federation| &federation.plan)
                }
                #[cfg(not(feature = "federated"))]
                {
                    &"<disabled>"
                }
            })
            .finish()
    }
}
