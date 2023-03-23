use itertools::Itertools;
use std::{collections::BTreeMap, fmt::Debug};

use super::EnvBuilder;

struct Dependency(String, String);

impl Debug for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\" -> \"{}\"", self.0, self.1)
    }
}

impl Debug for EnvBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ports = self
            .port_builders
            .keys()
            .map(|port_key| self.port_fqn(port_key).unwrap())
            .collect_vec();

        let reactors = self
            .reactor_builders
            .keys()
            .map(|reactor_key| self.reactor_fqn(reactor_key).unwrap())
            .collect_vec();

        let edges = self
            .reaction_dependency_edges()
            .map(|(a, b)| {
                let a_fqn = self.reaction_fqn(a).unwrap();
                let b_fqn = self.reaction_fqn(b).unwrap();
                Dependency(a_fqn, b_fqn)
            })
            .collect_vec();

        let reaction_levels = self.build_runtime_level_map().unwrap();
        let reactions = reaction_levels
            .iter()
            .map(|(key, level)| {
                let fqn = self.reaction_fqn(key).unwrap();
                (format!("{key:?}, {fqn}"), format!("Level({level})"))
            })
            .collect::<BTreeMap<_, _>>();

        let runtime_port_parts = self.build_runtime_ports();
        let port_aliases = runtime_port_parts
            .aliases
            .iter()
            .map(|(builder_port_key, port_key)| {
                (
                    self.port_fqn(builder_port_key).unwrap(),
                    format!("{:?}", runtime_port_parts.ports[*port_key]),
                )
            })
            .collect::<BTreeMap<_, _>>();

        f.debug_struct("EnvBuilder")
            .field("reactor_builders", &reactors)
            .field("port_builders", &ports)
            .field("runtime_port_aliases", &port_aliases)
            .field("reaction_dependency_edges", &edges)
            .field("reactions", &reactions)
            .finish()
    }

    #[cfg(feature = "disabled")]
    fn debug_info(&self) {
        for (runtime_port_key, triggers) in runtime_port_parts.port_triggers.iter() {
            // reverse look up the builder::port_key from the runtime::port_key
            let port_key = runtime_port_parts
                .aliases
                .iter()
                .find_map(|(port_key, runtime_port_key_b)| {
                    if &runtime_port_key == runtime_port_key_b {
                        Some(port_key)
                    } else {
                        None
                    }
                })
                .expect("Illegal internal state.");
            debug!(
                "{:?}: {:?}",
                self.port_fqn(port_key).unwrap(),
                triggers
                    .iter()
                    .map(|key| self.reaction_fqn(*key))
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            );
        }
    }
}
