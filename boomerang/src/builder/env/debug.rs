//! Debug impls for [`EnvBuilder`]

use itertools::Itertools;
use std::{collections::BTreeMap, fmt::Debug};

use crate::builder::{
    BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey, BuilderReactorKey,
};

use super::EnvBuilder;

/// Methods for building "fully-qualified" style names for various elements, useful for debugging output.
impl EnvBuilder {
    /// Get a fully-qualified string name for the given ActionKey
    pub fn action_fqn(&self, action_key: BuilderActionKey) -> Result<String, BuilderError> {
        self.reactor_builders
            .iter()
            .find_map(|(reactor_key, reactor_builder)| {
                reactor_builder
                    .actions
                    .get(action_key)
                    .map(|action_builder| (reactor_key, action_builder))
            })
            .ok_or(BuilderError::ActionKeyNotFound(action_key))
            .and_then(|(reactor_key, action_builder)| {
                self.reactor_fqn(reactor_key)
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!(
                            "Reactor referenced by {:?} not found: {:?}",
                            action_builder, err
                        ),
                    })
                    .map(|reactor_fqn| format!("{}/{}", reactor_fqn, action_builder.get_name()))
            })
    }

    /// Get a fully-qualified string for the given ReactionKey
    pub fn reactor_fqn(&self, reactor_key: BuilderReactorKey) -> Result<String, BuilderError> {
        self.reactor_builders
            .get(reactor_key)
            .ok_or(BuilderError::ReactorKeyNotFound(reactor_key))
            .and_then(|reactor| {
                reactor.parent_reactor_key.map_or_else(
                    || Ok(reactor.get_name().to_owned()),
                    |parent| {
                        self.reactor_fqn(parent)
                            .map(|parent| format!("{}::{}", parent, reactor.get_name()))
                    },
                )
            })
    }

    /// Get a fully-qualified string for the given ReactionKey
    pub fn reaction_fqn(&self, reaction_key: BuilderReactionKey) -> Result<String, BuilderError> {
        self.reaction_builders
            .get(reaction_key)
            .ok_or(BuilderError::ReactionKeyNotFound(reaction_key))
            .and_then(|reaction| {
                self.reactor_fqn(reaction.reactor_key)
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!("Reactor referenced by {:?} not found: {:?}", reaction, err),
                    })
                    .map(|reactor_fqn| (reactor_fqn, reaction.name.clone()))
            })
            .map(|(reactor_name, reaction_name)| format!("{}::{}", reactor_name, reaction_name))
    }

    /// Get a fully-qualified string for the given PortKey
    pub fn port_fqn(&self, port_key: BuilderPortKey) -> Result<String, BuilderError> {
        self.port_builders
            .get(port_key)
            .ok_or(BuilderError::PortKeyNotFound(port_key))
            .and_then(|port_builder| {
                self.reactor_fqn(port_builder.get_reactor_key())
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!(
                            "Reactor referenced by port {:?} not found: {:?}",
                            port_builder.get_name(),
                            err
                        ),
                    })
                    .map(|reactor_fqn| (reactor_fqn, port_builder.get_name()))
            })
            .map(|(reactor_name, port_name)| format!("{}.{}", reactor_name, port_name))
    }
}

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
