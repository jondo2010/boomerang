use super::{
    action::ActionBuilder, port::BasePortBuilder, reaction::ReactionBuilder, ActionBuilderFn,
    ActionType, BuilderActionKey, BuilderError, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, PortBuilder, PortType, ReactorBuilder, ReactorBuilderState, TypedActionKey,
    TypedPortKey,
};
use crate::runtime;
use itertools::{Either, Itertools};
use petgraph::{graphmap::DiGraphMap, EdgeDirection};
use runtime::ValuedAction;
use slotmap::{SecondaryMap, SlotMap};
use std::{
    collections::{BTreeSet, HashMap},
    convert::TryInto,
};
use tracing::{debug, info};

#[cfg(test)]
mod tests;

pub trait FindElements {
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError>;

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError>;
}

#[derive(Default)]
pub struct EnvBuilder {
    /// Builders for Ports
    pub(super) port_builders: SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
    /// Builders for Reactions
    pub(super) reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    /// Builders for Reactors
    pub(super) reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
}

/// Return type for building runtime parts
struct RuntimePortParts {
    /// All runtime Ports
    ports: tinymap::TinyMap<runtime::PortKey, Box<dyn runtime::BasePort>>,
    /// For each Port, a set of Reactions triggered by it
    port_triggers: tinymap::TinySecondaryMap<runtime::PortKey, Vec<BuilderReactionKey>>,
    /// A mapping from `BuilderPortKey`s to aliased [`runtime::PortKey`]s.
    aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a new Reactor
    /// - name: Instance name of the reactor
    pub fn add_reactor<S: runtime::ReactorState>(
        &mut self,
        name: &str,
        parent: Option<BuilderReactorKey>,
        reactor: S,
    ) -> ReactorBuilderState {
        ReactorBuilderState::new(name, parent, Box::new(reactor), self)
    }

    pub fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T>, BuilderError> {
        // Ensure no duplicates
        if self
            .port_builders
            .values()
            .any(|port| port.get_name() == name && port.get_reactor_key() == reactor_key)
        {
            return Err(BuilderError::DuplicatePortDefinition {
                reactor_name: self.reactor_builders[reactor_key].name.clone(),
                port_name: name.into(),
            });
        }

        let key = self.port_builders.insert_with_key(|port_key| {
            self.reactor_builders[reactor_key]
                .ports
                .insert(port_key, ());
            Box::new(PortBuilder::<T>::new(name, reactor_key, port_type))
        });

        Ok(TypedPortKey::new(key))
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), _>(
            name,
            ActionType::Timer { period, offset },
            reactor_key,
            move |name: &'_ str| runtime::InternalAction::Timer {
                name: name.into(),
                offset,
                period,
            },
        )
    }

    pub fn add_shutdown_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), _>(name, ActionType::Shutdown, reactor_key, |name: &'_ str| {
            runtime::InternalAction::Shutdown { name: name.into() }
        })
    }

    pub fn add_logical_action<T: runtime::PortData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T>, BuilderError> {
        self.add_action::<T, _>(
            name,
            ActionType::Logical { min_delay },
            reactor_key,
            move |name: &'_ str| {
                runtime::InternalAction::Valued(ValuedAction::new::<T>(
                    name,
                    true,
                    min_delay.unwrap_or_default(),
                ))
            },
        )
    }

    /// Add an Action to a given Reactor using closure F
    pub fn add_action<T, F>(
        &mut self,
        name: &str,
        ty: ActionType,
        reactor_key: BuilderReactorKey,
        action_fn: F,
    ) -> Result<TypedActionKey<T>, BuilderError>
    where
        T: runtime::PortData,
        F: ActionBuilderFn + 'static,
    {
        let reactor_builder = &mut self.reactor_builders[reactor_key];

        // Ensure no duplicates
        if reactor_builder
            .actions
            .values()
            .any(|action_builder| action_builder.get_name() == name)
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: self.reactor_builders[reactor_key].name.clone(),
                action_name: name.into(),
            });
        }

        let key = reactor_builder
            .actions
            .insert(ActionBuilder::new(name, ty, Box::new(action_fn)));

        Ok(key.into())
    }

    /// Find a Port matching a given name and ReactorKey
    pub fn get_port(
        &self,
        port_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderPortKey, BuilderError> {
        self.port_builders
            .iter()
            .find(|(_, port_builder)| {
                port_builder.get_name() == port_name
                    && port_builder.get_reactor_key() == reactor_key
            })
            .map(|(port_key, _)| port_key)
            .ok_or_else(|| BuilderError::NamedPortNotFound(port_name.to_string()))
    }

    /// Find an Action matching a given name and ReactorKey
    pub fn find_action_by_name(
        &self,
        action_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.reactor_builders[reactor_key]
            .actions
            .iter()
            .find(|(_, action_builder)| action_builder.get_name() == action_name)
            .map(|(action_key, _)| action_key)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_name.to_string()))
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port<T: runtime::PortData>(
        &mut self,
        port_a_key: TypedPortKey<T>,
        port_b_key: TypedPortKey<T>,
    ) -> Result<(), BuilderError> {
        let port_a_key = port_a_key.into();
        let port_b_key = port_b_key.into();

        let port_a_fqn = self.port_fqn(port_a_key)?;
        let port_b_fqn = self.port_fqn(port_b_key)?;

        let [port_a, port_b] = self
            .port_builders
            .get_disjoint_mut([port_a_key, port_b_key])
            .unwrap();

        if port_b.get_inward_binding().is_some() {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: format!(
                    "Ports may only be connected once, but B is already connected to {:?}",
                    port_b.get_inward_binding()
                ),
            });
        }

        if !port_a.get_deps().is_empty() {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: "Ports with dependencies may not be connected to other ports".to_owned(),
            });
        }

        if port_b.get_antideps().len() > 0 {
            return Err(BuilderError::PortBindError {
                port_a_key,
                port_b_key,
                what: "Ports with antidependencies may not be connected to other ports".to_owned(),
            });
        }

        match (port_a.get_port_type(), port_b.get_port_type()) {
            (PortType::Input, PortType::Input) => {
                self.reactor_builders[port_b.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if port_a.get_reactor_key() == parent_key { Some(()) } else { None }
                     }).ok_or(
                        BuilderError::PortBindError{
                                port_a_key,
                                port_b_key,
                                what: "An input port A may only be bound to another input port B if B is contained by a reactor that in turn is contained by the reactor of A.".into()
                            })
            }
            (PortType::Output, PortType::Input) => {
                let port_a_grandparent = self.reactor_builders[port_a.get_reactor_key()].parent_reactor_key;
                let port_b_grandparent = self.reactor_builders[port_b.get_reactor_key()].parent_reactor_key;
                // VALIDATE(this->container()->container() == port->container()->container(), 
                if !matches!((port_a_grandparent, port_b_grandparent), (Some(key_a), Some(key_b)) if key_a == key_b) {
                    Err(BuilderError::PortBindError{
                        port_a_key,
                        port_b_key,
                        what: format!("An output port ({}) can only be bound to an input port ({}) if both ports belong to reactors in the same hierarichal level", port_a_fqn, port_b_fqn),
                    })
                }
                // VALIDATE(this->container() != port->container(), );
                else if port_a.get_reactor_key() == port_b.get_reactor_key() {
                    Err(BuilderError::PortBindError{
                        port_a_key,
                        port_b_key,
                        what: format!("An output port ({}) can only be bound to an input port ({}) if both ports belong to different reactors!", port_a_fqn, port_b_fqn),
                    })
                }
                else {
                    Ok(())
                }
            }
            (PortType::Output, PortType::Output) => {
                // VALIDATE( this->container()->container() == port->container(),
                self.reactor_builders[port_a.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if parent_key == port_b.get_reactor_key() {
                            Some(())
                        } else {
                            None
                        }
                    }).ok_or(
                        BuilderError::PortBindError {
                                port_a_key,
                                port_b_key,
                                what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".to_owned()
                            })
            }
            (PortType::Input, PortType::Output) =>  {
                Err(BuilderError::PortBindError {
                    port_a_key,
                    port_b_key,
                    what: "Unexpected case: can't bind an input Port to an output Port.".to_owned()
                })
            }
        }?;

        port_b.set_inward_binding(Some(port_a_key));
        port_a.add_outward_binding(port_b_key);

        Ok(())
    }

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
                    || Ok(reactor.name.clone()),
                    |parent| {
                        self.reactor_fqn(parent)
                            .map(|parent| format!("{}::{}", parent, reactor.name))
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

    /// Follow the inward_binding's of Ports to the source
    pub fn follow_port_inward_binding(&self, port_key: BuilderPortKey) -> BuilderPortKey {
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

    pub fn reaction_dependency_edges<'b>(
        &'b self,
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

    /// Prepare the DAG of Reactions
    pub fn get_reaction_graph(&self) -> DiGraphMap<BuilderReactionKey, ()> {
        let mut graph =
            DiGraphMap::from_edges(self.reaction_dependency_edges().map(|(a, b)| (b, a)));
        // Ensure all ReactionIndicies are represented
        self.reaction_builders.keys().for_each(|key| {
            graph.add_node(key);
        });

        graph
    }

    /// Build a DAG of Reactors
    pub fn build_reactor_graph(&self) -> DiGraphMap<BuilderReactorKey, ()> {
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

    /// Build a Mapping of ReactionKey -> level corresponding to the parallelizable schedule
    ///
    /// This implements the Coffman-Graham algorithm for job scheduling.
    /// See https://en.m.wikipedia.org/wiki/Coffman%E2%80%93Graham_algorithm
    pub fn build_runtime_level_map(
        &self,
    ) -> Result<SecondaryMap<BuilderReactionKey, usize>, BuilderError> {
        use petgraph::{algo::tred, graph::DefaultIx, graph::NodeIndex};

        let mut graph = self.get_reaction_graph().into_graph::<DefaultIx>();

        // Transitive reduction and closures
        let toposort = petgraph::algo::toposort(&graph, None).map_err(|e| {
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

        // Collect and return a HashMap with ReactionKey indices instead of NodeIndex
        Ok(levels
            .iter()
            .map(|(&idx, &level)| (graph[idx], level - 1))
            .collect())
    }

    /// Construct runtime port structures from the builders.
    fn build_runtime_ports(&self) -> RuntimePortParts {
        let mut runtime_ports = tinymap::TinyMap::new();
        let mut runtime_port_triggers = tinymap::TinySecondaryMap::new();
        let mut alias_map = SecondaryMap::new();

        let port_groups = self
            .port_builders
            .keys()
            .map(|port_key| (port_key, self.follow_port_inward_binding(port_key)))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .group_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let runtime_port_key = runtime_ports.insert_with_key(|key| {
                self.port_builders[inward_port_key].create_runtime_port(key)
            });

            runtime_port_triggers.insert(
                runtime_port_key,
                self.collect_transitive_port_triggers(inward_port_key)
                    .keys()
                    .collect_vec(),
            );

            alias_map.extend(group.map(move |(port_key, _inward_key)| (port_key, runtime_port_key)))
        }

        RuntimePortParts {
            ports: runtime_ports,
            port_triggers: runtime_port_triggers,
            aliases: alias_map,
        }
    }

    pub fn debug_info(&self) {
        let reactors = self
            .reactor_builders
            .iter()
            .map(|(reactor_key, reactor_builder)| {
                let fqn = self.reactor_fqn(reactor_key).unwrap();
                format!(" - {reactor_key:?}: \"{fqn}\"")
            })
            .join("\n");

        // let actions = self
        //    .action_builders
        //    .keys()
        //    .map(|action_key| {
        //        let fqn = self.action_fqn(action_key).unwrap();
        //        format!(" - {action_key:?}: \"{fqn}\"")
        //    })
        //    .join("\n");
        // info!("actions:\n{actions}");

        let ports = self
            .port_builders
            .keys()
            .map(|port_key| {
                let fqn = self.port_fqn(port_key).unwrap();
                format!(" - {port_key:?}: {fqn}")
            })
            .join("\n");
        info!("ports:\n{ports}");

        let edges = self
            .reaction_dependency_edges()
            .map(|(a, b)| {
                let a_fqn = self.reaction_fqn(a).unwrap();
                let b_fqn = self.reaction_fqn(b).unwrap();
                format!(" - ({a_fqn} : {a:?}) -> ({b_fqn} : {b:?})")
            })
            .join("\n");
        info!("reaction_dependency_edges:\n{edges}");

        let reaction_levels = self.build_runtime_level_map().unwrap();
        let levels = reaction_levels
            .iter()
            .map(|(key, level)| {
                let fqn = self.reaction_fqn(key).unwrap();
                format!(" - {fqn}: {level}")
            })
            .join("\n");
        info!("runtime_level_map:\n{levels}");

        let runtime_port_parts = self.build_runtime_ports();
        for (builder_port_key, port_key) in runtime_port_parts.aliases.iter() {
            debug!(
                "Alias {} -> {:?}",
                self.port_fqn(builder_port_key).unwrap(),
                runtime_port_parts.ports[*port_key]
            )
        }

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

/// Build a new `SlotMap` of `runtime::Reactor` from `ReactorBuilder`s.
fn build_runtime_reactors(
    reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
    reaction_levels: &SecondaryMap<BuilderReactionKey, usize>,
) -> (
    tinymap::TinyMap<runtime::ReactorKey, runtime::Reactor>,
    SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
) {
    let mut runtime_reactors = tinymap::TinyMap::with_capacity(reactor_builders.len());

    // Mapping from builder::ReactorKey to runtime::ReactorKey
    let mut key_map = SecondaryMap::new();

    for (builder_key, reactor_builder) in reactor_builders.into_iter() {
        let ReactorBuilder {
            name,
            state,
            actions,
            ..
        } = reactor_builder;

        let mut runtime_actions = tinymap::TinyMap::with_capacity(actions.len());
        let mut runtime_action_triggers = tinymap::TinySecondaryMap::new();

        for (builder_key, action_builder) in actions.into_iter() {
            let triggers = action_builder
                .triggers
                .keys()
                .map(|reaction_key| (reaction_levels[reaction_key], reaction_key))
                .collect();
            let runtime_key = runtime_actions.insert(action_builder.into());
            runtime_action_triggers.insert(runtime_key, triggers);
        }
        let reactor_key = runtime_reactors.insert(runtime::Reactor::new(
            &name,
            state,
            runtime_actions,
            runtime_action_triggers,
        ));
        key_map.insert(builder_key, reactor_key);
    }
    (runtime_reactors, key_map)
}

impl TryInto<(runtime::Env, runtime::DepInfo)> for EnvBuilder {
    type Error = BuilderError;
    fn try_into(self) -> Result<(runtime::Env, runtime::DepInfo), Self::Error> {
        let reaction_levels = self.build_runtime_level_map()?;
        let runtime_port_parts = self.build_runtime_ports();
        let (runtime_reactors, reactor_key_map) =
            build_runtime_reactors(self.reactor_builders, &reaction_levels);

        // Build the SlotMap of runtime::Reaction from the ReactionBuilders.
        // This depends on iter() being stable and that inserting in the same order results in the
        // same keys.
        let (
            reactions,
            reaction_inputs,
            reaction_outputs,
            // reaction_trig_actions,
            // reaction_sched_actions,
        ) = {
            let mut reactions = tinymap::TinyMap::<runtime::ReactionKey, _>::with_capacity(
                self.reaction_builders.len(),
            );

            let mut reaction_inputs = tinymap::TinySecondaryMap::new();
            let mut reaction_outputs = tinymap::TinySecondaryMap::new();

            // let mut reaction_trig_actions = SecondaryMap::new();
            // let mut reaction_sched_actions = SecondaryMap::new();

            for (builder_key, reaction_builder) in self.reaction_builders.into_iter() {
                let new_key = reactions.insert_with_key(|reaction_key| {
                    // Create the Vec of input ports for this reaction sorted by order
                    reaction_inputs.insert(
                        reaction_key,
                        reaction_builder
                            .input_ports
                            .iter()
                            .sorted_by_key(|(_, &order)| order)
                            .map(|(builder_port_key, _)| {
                                runtime_port_parts.aliases[builder_port_key]
                            })
                            .collect::<Vec<_>>(),
                    );
                    // Create the Vec of output ports for this reaction sorted by order
                    reaction_outputs.insert(
                        reaction_key,
                        reaction_builder
                            .output_ports
                            .iter()
                            .sorted_by_key(|(_, &order)| order)
                            .map(|(builder_port_key, _)| {
                                runtime_port_parts.aliases[builder_port_key]
                            })
                            .collect::<Vec<_>>(),
                    );

                    // Create grouped iterable of actions associated with this reaction, and whether
                    // they are schedulable or not
                    // let grouped_actions = reaction_builder
                    //    .schedulable_actions
                    //    .iter()
                    //    .map(|(action_key, order)| (action_key, order, true))
                    //    .chain(
                    //        reaction_builder
                    //            .trigger_actions
                    //            .iter()
                    //            .map(|(action_key, order)| (action_key, order, false)),
                    //    )
                    //    .sorted_by_key(|(action_key, _, _)| *action_key)
                    //    .group_by(|(action_key, _, _)| *action_key);

                    // let (actions_trigger, actions_schedulable) = grouped_actions
                    //    .into_iter()
                    //    .filter_map(|(_, group)| {
                    //        group.reduce(|acc, item| (acc.0, acc.1.max(item.1),
                    // acc.2.max(item.2)))    })
                    //    .sorted_by_key(|(_, &order, _)| order)
                    //    .map(|(action_key, _, schedulable)| (action_key, schedulable))
                    //    .partition_map(|(action_key, schedulable)| {
                    //        if schedulable {
                    //            Either::Right(action_key)
                    //        } else {
                    //            Either::Left(action_key)
                    //        }
                    //    });

                    // Create the Vec of Actions that trigger this reaction
                    // reaction_trig_actions.insert(reaction_key, actions_trigger);

                    // Create the Vec of schedulable actions for this reaction sorted by order
                    // reaction_sched_actions.insert(reaction_key, actions_schedulable);

                    reaction_builder.build_reaction(&reactor_key_map)
                });
            }
            (
                reactions,
                reaction_inputs,
                reaction_outputs,
                // reaction_trig_actions,
                // reaction_sched_actions,
            )
        };

        Ok((
            runtime::Env {
                reactors: runtime_reactors,
                ports: runtime_port_parts.ports,
                reactions,
            },
            runtime::DepInfo {
                port_triggers: runtime_port_parts.port_triggers,
                reaction_levels,
                reaction_inputs,
                reaction_outputs,
                // reaction_trig_actions,
                // reaction_sched_actions,
            },
        ))
    }
}
