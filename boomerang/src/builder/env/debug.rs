//! Debug impls for [`EnvBuilder`]

use itertools::Itertools;
use std::{collections::BTreeMap, fmt::Debug};

use crate::builder::{
    BasePortBuilder, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, ReactionBuilder,
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

#[derive(Debug)]
#[allow(dead_code)]
struct PortDebug {
    key: BuilderPortKey,
    deps: Vec<String>,
    anti_deps: Vec<String>,
    outward_bindings: Vec<String>,
    inward_bindings: Option<String>,
    triggers: Vec<String>,
}

impl PortDebug {
    fn new(key: BuilderPortKey, port: &Box<dyn BasePortBuilder>, env: &EnvBuilder) -> Self {
        Self {
            key,
            deps: port
                .get_deps()
                .iter()
                .map(|reaction_key| env.reaction_fqn(*reaction_key).unwrap())
                .collect(),
            anti_deps: port
                .get_antideps()
                .map(|reaction_key| env.reaction_fqn(reaction_key).unwrap())
                .collect(),
            outward_bindings: port
                .get_outward_bindings()
                .map(|port_key| env.port_fqn(port_key).unwrap())
                .collect(),
            inward_bindings: port
                .get_inward_binding()
                .map(|port_key| env.port_fqn(port_key).unwrap()),
            triggers: port
                .get_triggers()
                .iter()
                .map(|reaction_key| env.reaction_fqn(*reaction_key).unwrap())
                .collect(),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct ReactionDebug {
    key: BuilderReactionKey,
    level: String,
    input_ports: Vec<String>,
    output_ports: Vec<String>,
    trigger_actions: Vec<String>,
    schedulable_actions: Vec<String>,
}

impl ReactionDebug {
    fn new(
        key: BuilderReactionKey,
        reaction: &ReactionBuilder,
        level: usize,
        env: &EnvBuilder,
    ) -> Self {
        Self {
            key,
            level: format!("Level({level})"),
            input_ports: reaction
                .input_ports
                .keys()
                .map(|port_key| env.port_fqn(port_key).unwrap())
                .collect(),
            output_ports: reaction
                .output_ports
                .keys()
                .map(|port_key| env.port_fqn(port_key).unwrap())
                .collect(),
            trigger_actions: reaction
                .trigger_actions
                .keys()
                .map(|action_key| env.action_fqn(action_key).unwrap())
                .collect(),
            schedulable_actions: reaction
                .schedulable_actions
                .keys()
                .map(|action_key| env.action_fqn(action_key).unwrap())
                .collect(),
        }
    }
}

impl Debug for EnvBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ports = self
            .port_builders
            .iter()
            .map(|(port_key, port)| {
                let fqn = self.port_fqn(port_key).unwrap();
                let debug_struct = PortDebug::new(port_key, port, self);
                (fqn, debug_struct)
            })
            .collect::<BTreeMap<_, _>>();

        let reactors = self
            .reactor_builders
            .iter()
            .map(|(reactor_key, reactor)| {
                let fqn = self.reactor_fqn(reactor_key).unwrap();
                (fqn, reactor)
            })
            .collect::<BTreeMap<_, _>>();

        let all_reactor_keys = self.reactor_builders.keys().collect_vec();

        let edges = self
            .reaction_dependency_edges(&all_reactor_keys)
            .map(|(a, b)| {
                let a_fqn = self.reaction_fqn(a).unwrap();
                let b_fqn = self.reaction_fqn(b).unwrap();
                Dependency(a_fqn, b_fqn)
            })
            .collect_vec();

        let reaction_levels = self.build_runtime_level_map(&all_reactor_keys).unwrap();

        let reactions = reaction_levels
            .iter()
            .map(|(reaction_key, level)| {
                let reaction = &self.reaction_builders[reaction_key];
                let fqn = self.reaction_fqn(reaction_key).unwrap();
                let debug_struct = ReactionDebug::new(reaction_key, reaction, *level, self);
                (fqn, debug_struct)
            })
            .collect::<BTreeMap<_, _>>();

        let runtime_port_parts = self.build_runtime_ports(all_reactor_keys);
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
}
