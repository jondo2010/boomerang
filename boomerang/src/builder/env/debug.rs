use std::{collections::BTreeMap, fmt::Debug};

use itertools::Itertools;

use super::EnvBuilder;

struct Dependency(String, String);

impl Debug for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\" -> \"{}\"", self.0, self.1)
    }
}

impl Debug for EnvBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reactors = self
            .reactor_builders
            .keys()
            .map(|reactor_key| {
                let fqn = self.reactor_fqn(reactor_key).unwrap().to_string();
                (format!("{reactor_key:?}"), fqn.to_string())
            })
            .collect::<BTreeMap<_, _>>();

        let actions = self
            .action_builders
            .iter()
            .map(move |(action_key, action)| {
                let fqn = self.action_fqn(action_key).unwrap();
                (
                    format!("{action_key:?}"),
                    format!("{fqn} : <{ty:?}>", ty = action.get_type()),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let ports = self
            .port_builders
            .keys()
            .map(|port_key| {
                let fqn = self.port_fqn(port_key).unwrap();
                (format!("{port_key:?}"), fqn.to_string())
            })
            .collect::<BTreeMap<_, _>>();

        let edges = self
            .reaction_dependency_edges()
            .map(|(a, b)| {
                let a_fqn = self.reaction_fqn(a).unwrap();
                let b_fqn = self.reaction_fqn(b).unwrap();
                Dependency(a_fqn, b_fqn)
            })
            .collect_vec();

        let reactions = if let Ok(reaction_levels) = self.build_runtime_level_map() {
            reaction_levels
                .iter()
                .map(|(key, level)| {
                    let fqn = self.reaction_fqn(key).unwrap();
                    (format!("{key:?}, prio:{fqn}"), format!("Level({level})"))
                })
                .collect::<BTreeMap<_, _>>()
        } else {
            // There was a cycle in the reaction graph, so don't show the reaction levels.
            self.reaction_builders
                .iter()
                .map(|(key, builder)| {
                    let fqn = self.reaction_fqn(key).unwrap();
                    let priority = builder.get_priority();
                    (format!("{key:?}, {priority}"), fqn.to_string())
                })
                .collect::<BTreeMap<_, _>>()
        };

        let runtime_port_parts = self.build_runtime_ports();
        let port_aliases = runtime_port_parts
            .port_aliases
            .iter()
            .map(|(builder_port_key, port_key)| {
                (
                    self.port_fqn(builder_port_key).unwrap(),
                    format!("{:?}", runtime_port_parts.ports[*port_key]),
                )
            })
            .collect::<BTreeMap<_, _>>();

        f.debug_struct("EnvBuilder")
            .field("reactors", &reactors)
            .field("actions", &actions)
            .field("ports", &ports)
            .field("runtime_port_aliases", &port_aliases)
            .field("reaction_dependency_edges", &edges)
            .field("reactions", &reactions)
            .finish()
    }
}
