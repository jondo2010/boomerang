use super::{
    action::ActionBuilder, port::BasePortBuilder, reaction::ReactionBuilder, ActionBuilderFn,
    ActionType, BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, Logical, Physical, PortBuilder, PortType, ReactionBuilderState,
    ReactorBuilder, ReactorBuilderState, TypedActionKey, TypedPortKey,
};
use crate::runtime;
use boomerang_runtime::Level;
use itertools::Itertools;
use petgraph::{graphmap::DiGraphMap, EdgeDirection};
use runtime::{LogicalAction, PhysicalAction};
use slotmap::{SecondaryMap, SlotMap};
use std::{
    collections::{BTreeSet, HashMap},
    convert::TryInto,
};

mod debug;
#[cfg(test)]
mod tests;

pub trait FindElements {
    fn get_port_by_name(&self, port_name: &str) -> Result<BuilderPortKey, BuilderError>;

    fn get_action_by_name(&self, action_name: &str) -> Result<BuilderActionKey, BuilderError>;
}

#[derive(Default)]
pub struct EnvBuilder {
    /// Builder for Actions
    pub(super) action_builders: SlotMap<BuilderActionKey, ActionBuilder>,
    /// Builders for Ports
    pub(super) port_builders: SlotMap<BuilderPortKey, Box<dyn BasePortBuilder>>,
    /// Builders for Reactions
    pub(super) reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    /// Builders for Reactors
    pub(super) reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
}

/// Return type for building runtime parts
#[derive(Debug)]
struct RuntimePortParts {
    /// All runtime Ports
    ports: tinymap::TinyMap<runtime::PortKey, Box<dyn runtime::BasePort>>,
    /// For each Port, a set of Reactions triggered by it
    port_triggers: tinymap::TinySecondaryMap<runtime::PortKey, Vec<BuilderReactionKey>>,
    /// A mapping from `BuilderPortKey`s to aliased [`runtime::PortKey`]s.
    port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

#[derive(Debug)]
struct RuntimeActionParts {
    actions: tinymap::TinyMap<runtime::ActionKey, runtime::Action>,
    action_triggers: tinymap::TinySecondaryMap<runtime::ActionKey, Vec<BuilderReactionKey>>,
    aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
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
        state: S,
    ) -> ReactorBuilderState {
        ReactorBuilderState::new(name, parent, state, self)
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
                reactor_name: self.reactor_builders[reactor_key].get_name().to_owned(),
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

    pub fn add_startup_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), Logical, _>(
            name,
            ActionType::Startup,
            reactor_key,
            |_: &'_ str, _: runtime::ActionKey| runtime::Action::Startup,
        )
    }

    pub fn add_shutdown_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), Logical, _>(
            name,
            ActionType::Shutdown,
            reactor_key,
            |_: &'_ str, _: runtime::ActionKey| runtime::Action::Shutdown,
        )
    }

    pub fn add_logical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Logical>, BuilderError> {
        self.add_action::<T, Logical, _>(
            name,
            ActionType::Logical { min_delay },
            reactor_key,
            move |name: &'_ str, key: runtime::ActionKey| {
                runtime::Action::Logical(LogicalAction::new::<T>(
                    name,
                    key,
                    min_delay.unwrap_or_default(),
                ))
            },
        )
    }

    pub fn add_physical_action<T: runtime::ActionData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Physical>, BuilderError> {
        self.add_action::<T, Physical, _>(
            name,
            ActionType::Physical { min_delay },
            reactor_key,
            move |name: &'_ str, action_key| {
                runtime::Action::Physical(PhysicalAction::new::<T>(
                    name,
                    action_key,
                    min_delay.unwrap_or_default(),
                ))
            },
        )
    }

    /// Add a Reaction to a given Reactor
    pub fn add_reaction(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        reaction_fn: Box<dyn runtime::ReactionFn>,
    ) -> ReactionBuilderState {
        let priority = self.reactor_builders[reactor_key].reactions.len();
        ReactionBuilderState::new(name, priority, reactor_key, reaction_fn, self)
    }

    /// Add an Action to a given Reactor using closure F
    pub fn add_action<T, Q, F>(
        &mut self,
        name: &str,
        ty: ActionType,
        reactor_key: BuilderReactorKey,
        action_fn: F,
    ) -> Result<TypedActionKey<T, Q>, BuilderError>
    where
        T: runtime::ActionData,
        F: ActionBuilderFn + 'static,
    {
        let reactor_builder = &mut self.reactor_builders[reactor_key];

        // Ensure no duplicates
        if reactor_builder
            .actions
            .keys()
            .any(|action_key| self.action_builders[action_key].get_name() == name)
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: reactor_builder.get_name().to_owned(),
                action_name: name.into(),
            });
        }

        let key = self.action_builders.insert(ActionBuilder::new(
            name,
            reactor_key,
            ty,
            Box::new(action_fn),
        ));

        reactor_builder.actions.insert(key, ());

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

    /// Find a Reaction matching a given name and ReactorKey
    pub fn get_reaction(
        &self,
        reaction_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderReactionKey, BuilderError> {
        self.reaction_builders
            .iter()
            .find(|(_, reaction_builder)| {
                reaction_builder.get_name() == reaction_name
                    && reaction_builder.get_reactor_key() == reactor_key
            })
            .map(|(reaction_key, _)| reaction_key)
            .ok_or_else(|| BuilderError::NamedReactionNotFound(reaction_name.to_string()))
    }

    /// Find an Action matching a given name and ReactorKey
    pub fn find_action_by_name(
        &self,
        action_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderActionKey, BuilderError> {
        self.reactor_builders[reactor_key]
            .actions
            .keys()
            .find(|action_key| self.action_builders[*action_key].get_name() == action_name)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_name.to_string()))
    }

    /// Find a Reactor in the EnvBuilder given its fully-qualified name
    pub fn find_reactor_by_fqn<T>(&self, reactor_fqn: T) -> Result<BuilderReactorKey, BuilderError>
    where
        T: TryInto<BuilderFqn>,
        T::Error: Into<BuilderError>,
    {
        let reactor_fqn: BuilderFqn = reactor_fqn.try_into().map_err(Into::into)?;
        let (_, reactor_name) = reactor_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(reactor_fqn.to_string()))?;
        self.reactor_builders
            .iter()
            .find_map(|(reactor_key, reactor_builder)| {
                if reactor_builder.get_name() == reactor_name {
                    Some(reactor_key)
                } else {
                    None
                }
            })
            .ok_or_else(|| BuilderError::NamedReactorNotFound(reactor_fqn.to_string()))
    }

    /// Find a PhysicalAction globally in the EnvBuilder given its fully-qualified name
    pub fn find_physical_action_by_fqn<T>(
        &self,
        action_fqn: T,
    ) -> Result<BuilderActionKey, BuilderError>
    where
        T: TryInto<BuilderFqn>,
        T::Error: Into<BuilderError>,
    {
        let action_fqn: BuilderFqn = action_fqn.try_into().map_err(Into::into)?;

        let (reactor_fqn, action_name) = action_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(action_fqn.to_string()))?;

        let reactor = self.find_reactor_by_fqn(reactor_fqn)?;

        self.reactor_builders[reactor]
            .actions
            .keys()
            .find(|action_key| self.action_builders[*action_key].get_name() == action_name)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_fqn.to_string()))
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port<P>(&mut self, port_a_key: P, port_b_key: P) -> Result<(), BuilderError>
    where
        P: Into<BuilderPortKey>,
    {
        let port_a_key = port_a_key.into();
        let port_b_key = port_b_key.into();

        let port_a_fqn = self.port_fqn(port_a_key)?;
        let port_b_fqn = self.port_fqn(port_b_key)?;

        let port_a = &self.port_builders[port_a_key];
        let port_b = &self.port_builders[port_b_key];

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

        // All validity checks passed, so we can now bind the ports
        self.port_builders[port_b_key].set_inward_binding(Some(port_a_key));
        self.port_builders[port_a_key].add_outward_binding(port_b_key);

        Ok(())
    }

    /// Get a fully-qualified string name for the given ActionKey
    pub fn action_fqn(&self, action_key: BuilderActionKey) -> Result<BuilderFqn, BuilderError> {
        let action = &self.action_builders[action_key];
        let reactor_fqn = self.reactor_fqn(action.get_reactor_key())?;
        Ok(reactor_fqn.append(action.get_name()))
    }

    /// Get a fully-qualified string for the given ReactionKey
    pub fn reactor_fqn(&self, reactor_key: BuilderReactorKey) -> Result<BuilderFqn, BuilderError> {
        self.reactor_builders
            .get(reactor_key)
            .ok_or(BuilderError::ReactorKeyNotFound(reactor_key))
            .and_then(|reactor| {
                reactor.parent_reactor_key.map_or_else(
                    || Ok(reactor.get_name().try_into().unwrap()),
                    |parent| {
                        self.reactor_fqn(parent)
                            .map(|parent| parent.append(reactor.get_name()))
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

    /// Build an iterator of all Reaction dependency edges in the graph
    pub fn reaction_dependency_edges(
        &self,
    ) -> impl Iterator<Item = (BuilderReactionKey, BuilderReactionKey)> + '_ {
        let deps = self
            .reaction_builders
            .iter()
            .flat_map(move |(reaction_key, reaction)| {
                // Connect all reactions this reaction depends upon
                reaction
                    .trigger_ports
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
    pub fn build_reaction_graph(&self) -> DiGraphMap<BuilderReactionKey, ()> {
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

    /// Build a Mapping of `BuilderReactionKey` -> `Level` corresponding to the parallelizable
    /// schedule
    ///
    /// This implements the Coffman-Graham algorithm for job scheduling.
    /// See https://en.m.wikipedia.org/wiki/Coffman%E2%80%93Graham_algorithm
    pub fn build_runtime_level_map(
        &self,
    ) -> Result<SecondaryMap<BuilderReactionKey, Level>, BuilderError> {
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

        let mut levels: HashMap<_, Level> = HashMap::new();
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
    fn build_runtime_ports(&self) -> RuntimePortParts {
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
}

fn build_runtime_reactions(
    reaction_builders: SlotMap<BuilderReactionKey, ReactionBuilder>,
    port_aliases: &SecondaryMap<BuilderPortKey, boomerang_runtime::PortKey>,
    action_aliases: &SecondaryMap<BuilderActionKey, boomerang_runtime::ActionKey>,
) -> (
    tinymap::TinyMap<boomerang_runtime::ReactionKey, boomerang_runtime::Reaction>,
    SecondaryMap<BuilderReactionKey, boomerang_runtime::ReactionKey>,
    SecondaryMap<BuilderReactionKey, BuilderReactorKey>,
) {
    let mut runtime_reactions = tinymap::TinyMap::with_capacity(reaction_builders.len());
    let mut reaction_aliases = SecondaryMap::new();
    let mut reaction_reactor_aliases = SecondaryMap::new();

    for (builder_key, reaction_builder) in reaction_builders.into_iter() {
        reaction_reactor_aliases.insert(builder_key, reaction_builder.reactor_key);
        let reaction_key = runtime_reactions.insert(reaction_builder.build_runtime_reaction(
            runtime::ReactorKey::default(),
            port_aliases,
            action_aliases,
        ));
        reaction_aliases.insert(builder_key, reaction_key);
    }
    (
        runtime_reactions,
        reaction_aliases,
        reaction_reactor_aliases,
    )
}

fn build_runtime_reactors(
    reactor_builders: SlotMap<BuilderReactorKey, ReactorBuilder>,
    reaction_levels: &SecondaryMap<BuilderReactionKey, Level>,
    reaction_aliases: &SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    action_triggers: &tinymap::TinySecondaryMap<runtime::ActionKey, Vec<BuilderReactionKey>>,
) -> (
    tinymap::TinyMap<runtime::ReactorKey, runtime::Reactor>,
    SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
) {
    let mut runtime_reactors = tinymap::TinyMap::with_capacity(reactor_builders.len());
    let mut reactor_alias = SecondaryMap::new();

    for (builder_key, reactor_builder) in reactor_builders.into_iter() {
        // Build the runtime_action_triggers from the action_triggers part, mapping
        // BuilderReactionKey -> runtime::ReactionKey
        let runtime_action_triggers = action_triggers
            .iter()
            .map(|(action_key, triggers)| {
                let downstream = triggers
                    .iter()
                    .map(|&builder_reaction_key| {
                        (
                            reaction_levels[builder_reaction_key],
                            reaction_aliases[builder_reaction_key],
                        )
                    })
                    .collect();
                (action_key, downstream)
            })
            .collect();

        let reactor_key =
            runtime_reactors.insert(reactor_builder.build_runtime(runtime_action_triggers));
        reactor_alias.insert(builder_key, reactor_key);
    }
    (runtime_reactors, reactor_alias)
}

/// Aliasing maps from Builder keys to runtime keys
pub struct BuilderAliases {
    pub reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    pub action_aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

impl EnvBuilder {
    /// Convert the `EnvBuilder` into a [`runtime::Env`], [`runtime::TriggerMap`] and [`BuilderAliases`]
    pub fn into_runtime_parts(
        self,
    ) -> Result<(runtime::Env, runtime::TriggerMap, BuilderAliases), BuilderError> {
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

        let (mut runtime_reactions, reaction_aliases, reaction_reactor_aliases) =
            build_runtime_reactions(self.reaction_builders, &port_aliases, &action_aliases);

        let (runtime_reactors, reactor_aliases) = build_runtime_reactors(
            self.reactor_builders,
            &reaction_levels,
            &reaction_aliases,
            &action_triggers,
        );

        // Update the Reactions with the Reactor keys
        for (builder_reaction_key, builder_reactor_key) in reaction_reactor_aliases {
            let runtime_reactor_key = reactor_aliases[builder_reactor_key];
            let runtime_reaction_key = reaction_aliases[builder_reaction_key];
            runtime_reactions[runtime_reaction_key].set_reactor_key(runtime_reactor_key);
        }

        let runtime_port_triggers = port_triggers
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

        let runtime_action_triggers = action_triggers
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

        Ok((
            runtime::Env {
                reactors: runtime_reactors,
                actions: runtime_actions,
                ports: runtime_ports,
                reactions: runtime_reactions,
            },
            runtime::TriggerMap {
                port_triggers: runtime_port_triggers,
                action_triggers: runtime_action_triggers,
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
