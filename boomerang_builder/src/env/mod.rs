use crate::{ActionTag, BuilderFqnSegment, ParentReactorBuilder, PortType};

use super::{
    action::ActionBuilder, port::BasePortBuilder, reaction::ReactionBuilder, runtime, ActionType,
    BuilderActionKey, BuilderError, BuilderFqn, BuilderPortKey, BuilderReactionKey,
    BuilderReactorKey, Input, Logical, Output, Physical, PortBuilder, PortTag,
    ReactionBuilderState, ReactorBuilder, ReactorBuilderState, TypedActionKey, TypedPortKey,
};
use itertools::Itertools;
use petgraph::{prelude::DiGraphMap, EdgeDirection};
use slotmap::{SecondaryMap, SlotMap};
use std::{
    collections::{BTreeSet, HashMap},
    convert::TryInto,
};

mod build;
mod debug;
#[cfg(test)]
mod tests;

mod util {
    use petgraph::visit::{IntoNeighborsDirected, IntoNodeIdentifiers, Visitable};
    use std::hash::Hash;

    /// Find a minimal cycle in a graph using DFS
    pub fn find_minimal_cycle<G>(graph: G, start_node: G::NodeId) -> Vec<G::NodeId>
    where
        G: IntoNeighborsDirected + IntoNodeIdentifiers + Visitable,
        G::NodeId: Hash + Eq,
    {
        let mut dfs = petgraph::visit::Dfs::new(&graph, start_node);
        let mut stack = Vec::new();
        let mut visited = std::collections::HashSet::new();

        while let Some(nx) = dfs.next(&graph) {
            if visited.contains(&nx) {
                // We've found a cycle, backtrack to find the minimal cycle
                while let Some(&last) = stack.last() {
                    if last == nx {
                        break;
                    }
                    stack.pop();
                }
                return stack.to_vec();
            }
            visited.insert(nx);
            stack.push(nx);
        }

        // This shouldn't happen if there's definitely a cycle
        vec![start_node]
    }
}

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

impl EnvBuilder {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a new Reactor
    /// - name: Instance name of the reactor
    pub fn add_reactor<S: runtime::ReactorData>(
        &mut self,
        name: &str,
        parent: Option<BuilderReactorKey>,
        bank_info: Option<runtime::BankInfo>,
        state: S,
    ) -> ReactorBuilderState {
        ReactorBuilderState::new(name, parent, bank_info, state, self)
    }

    /// Get a previously built reactor
    pub fn get_reactor_builder(
        &mut self,
        reactor_key: BuilderReactorKey,
    ) -> Result<ReactorBuilderState, BuilderError> {
        if !self.reactor_builders.contains_key(reactor_key) {
            return Err(BuilderError::ReactorKeyNotFound(reactor_key));
        }
        Ok(ReactorBuilderState::from_pre_existing(reactor_key, self))
    }

    /// Add an Input port to the Reactor
    pub fn add_input_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T, Input>, BuilderError> {
        self.internal_add_port::<T, Input>(name, reactor_key, None)
            .map(From::from)
    }

    /// Add an Output port to the Reactor
    pub fn add_output_port<T: runtime::ReactorData>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedPortKey<T, Output>, BuilderError> {
        self.internal_add_port::<T, Output>(name, reactor_key, None)
            .map(From::from)
    }

    pub fn internal_add_port<T: runtime::ReactorData, Q: PortTag>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        bank_info: Option<runtime::BankInfo>,
    ) -> Result<BuilderPortKey, BuilderError> {
        // Ensure no duplicates on (name, reactor_key, bank_info)
        if self.port_builders.values().any(|port| {
            port.name() == name
                && port.get_reactor_key() == reactor_key
                && port.bank_info() == bank_info.as_ref()
        }) {
            return Err(BuilderError::DuplicatePortDefinition {
                reactor_name: self.reactor_builders[reactor_key].name().to_owned(),
                port_name: name.into(),
            });
        }

        let key = self.port_builders.insert_with_key(|port_key| {
            self.reactor_builders[reactor_key]
                .ports
                .insert(port_key, ());
            Box::new(PortBuilder::<T, Q>::new(name, reactor_key, bank_info))
        });

        Ok(key)
    }

    pub fn add_startup_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), Logical>(name, reactor_key, ActionType::Startup)
    }

    pub fn add_shutdown_action(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey, BuilderError> {
        self.add_action::<(), Logical>(name, reactor_key, ActionType::Shutdown)
    }

    pub fn internal_add_action<T: runtime::ReactorData, Q: ActionTag>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: BuilderReactorKey,
    ) -> Result<TypedActionKey<T, Q>, BuilderError> {
        self.add_action::<T, Q>(
            name,
            reactor_key,
            ActionType::Standard {
                is_logical: Q::IS_LOGICAL,
                min_delay,
                build_fn: Box::new(move |name, key| {
                    runtime::Action::<T>::new(name, key, min_delay, Q::IS_LOGICAL).boxed()
                }),
            },
        )
    }

    /// Add a Reaction to a given Reactor
    pub fn add_reaction(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        reaction_fn: impl Into<runtime::BoxedReactionFn>,
    ) -> ReactionBuilderState {
        let priority = self.reactor_builders[reactor_key].reactions.len();
        ReactionBuilderState::new(name, priority, reactor_key, reaction_fn.into(), self)
    }

    /// Add an Action to a given Reactor using closure F
    pub fn add_action<T, Q: ActionTag>(
        &mut self,
        name: &str,
        reactor_key: BuilderReactorKey,
        r#type: ActionType,
    ) -> Result<TypedActionKey<T, Q>, BuilderError>
    where
        T: runtime::ReactorData,
    {
        let reactor_builder = &mut self.reactor_builders[reactor_key];

        // Ensure no duplicates
        if reactor_builder
            .actions
            .keys()
            .any(|action_key| self.action_builders[action_key].name() == name)
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: reactor_builder.name().to_owned(),
                action_name: name.into(),
            });
        }

        let key = self
            .action_builders
            .insert(ActionBuilder::new(name, reactor_key, r#type));

        reactor_builder.actions.insert(key, ());

        Ok(key.into())
    }

    /// Get a previously built action
    pub fn get_action(&self, action_key: BuilderActionKey) -> Result<&ActionBuilder, BuilderError> {
        self.action_builders
            .get(action_key)
            .ok_or(BuilderError::ActionKeyNotFound(action_key))
    }

    /// Get a previously built port
    pub fn get_port(&self, port_key: BuilderPortKey) -> Result<&dyn BasePortBuilder, BuilderError> {
        self.port_builders
            .get(port_key)
            .map(|builder| builder.as_ref())
            .ok_or(BuilderError::PortKeyNotFound(port_key))
    }

    /// Find a Port matching a given name and ReactorKey
    pub fn find_port_by_name(
        &self,
        port_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderPortKey, BuilderError> {
        self.port_builders
            .iter()
            .find(|(_, port_builder)| {
                port_builder.name() == port_name
                    && port_builder.parent_reactor_key() == Some(reactor_key)
            })
            .map(|(port_key, _)| port_key)
            .ok_or_else(|| BuilderError::NamedPortNotFound(port_name.to_string()))
    }

    /// Find a Reaction matching a given name and ReactorKey
    pub fn find_reaction_by_name(
        &self,
        reaction_name: &str,
        reactor_key: BuilderReactorKey,
    ) -> Result<BuilderReactionKey, BuilderError> {
        self.reaction_builders
            .iter()
            .find(|(_, reaction_builder)| {
                reaction_builder.name() == reaction_name
                    && reaction_builder.parent_reactor_key() == Some(reactor_key)
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
            .find(|action_key| self.action_builders[*action_key].name() == action_name)
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_name.to_string()))
    }

    /// Find a Reactor in the EnvBuilder given its fully-qualified name
    pub fn find_reactor_by_fqn<T>(&self, reactor_fqn: T) -> Result<BuilderReactorKey, BuilderError>
    where
        T: TryInto<BuilderFqn>,
        T::Error: Into<BuilderError>,
    {
        let reactor_fqn: BuilderFqn = reactor_fqn.try_into().map_err(Into::into)?;
        let (_, segment) = reactor_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(reactor_fqn.to_string()))?;
        self.reactor_builders
            .iter()
            .find_map(|(reactor_key, reactor_builder)| {
                if BuilderFqnSegment::from_reactor(reactor_builder, false) == segment {
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

        let (reactor_fqn, segment) = action_fqn
            .clone()
            .split_last()
            .ok_or(BuilderError::InvalidFqn(action_fqn.to_string()))?;

        let reactor = self.find_reactor_by_fqn(reactor_fqn)?;

        self.reactor_builders[reactor]
            .actions
            .keys()
            .find(|action_key| {
                BuilderFqnSegment::from_action(&self.action_builders[*action_key], false) == segment
            })
            .ok_or_else(|| BuilderError::NamedActionNotFound(action_fqn.to_string()))
    }

    /// Find a possible common parent Reactor for two Reactor elements in the EnvBuilder (if it exists).
    pub fn common_reactor_key<E0, E1>(&self, e0: &E0, e1: &E1) -> Option<BuilderReactorKey>
    where
        E0: ParentReactorBuilder,
        E1: ParentReactorBuilder,
    {
        let mut e0_key = e0.parent_reactor_key();
        let mut e1_key = e1.parent_reactor_key();
        while e0_key != e1_key {
            match (e0_key, e1_key) {
                (Some(key0), Some(key1)) => {
                    e0_key = self.reactor_builders[key0].parent_reactor_key;
                    e1_key = self.reactor_builders[key1].parent_reactor_key;
                }
                _ => return None,
            }
        }
        e0_key
    }

    /// Connect two ports together
    ///
    /// ## Arguments
    ///
    /// * `source_key` - The key of the first port to connect
    /// * `target_key` - The key of the second port to connect
    /// * `after` - An optional delay to wait before triggering the downstream ports.
    /// * `physical` - Whether the connection is physical (or logical). Logical connections will trigger any downstream
    ///     ports at the current logical time (with any `after` delay), while physical connections will trigger the
    ///     downstream ports at the current physical time (with any `after` delay).
    pub fn connect_ports<T, P1, P2>(
        &mut self,
        source_key: P1,
        target_key: P2,
        after: Option<runtime::Duration>,
        physical: bool,
    ) -> Result<(), BuilderError>
    where
        T: runtime::ReactorData + Clone,
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
    {
        if after.is_none() && !physical {
            self.bind_port(source_key, target_key)
        } else {
            // Ports connected with a delay and/or physical connections are implemented as a pair of Reactions that trigger and react to an action.

            let source_key = source_key.into();
            let target_key = target_key.into();

            let parent_reactor_key = self
                .common_reactor_key(
                    &self.port_builders[source_key],
                    &self.port_builders[target_key],
                )
                .ok_or(BuilderError::PortConnectionError {
                    port_a_key: source_key,
                    port_b_key: target_key,
                    what: "Ports must belong to the same reactor or a common parent reactor to be connected".to_owned(),
                })?;

            let source_fqn = self.port_fqn(source_key, false)?;
            let target_fqn = self.port_fqn(target_key, false)?;
            let reactor_name = format!("connection_{source_fqn}->{target_fqn}");

            // 1. create a new reactor to hold the action and reactions
            let (input_port, output_port) = if physical {
                let reactor =
                    <crate::connection::ConnectionBuilder<T, Physical> as crate::Reactor>::build(
                        &reactor_name,
                        after.unwrap_or_default(),
                        Some(parent_reactor_key),
                        None,
                        self,
                    )?;
                (reactor.input, reactor.output)
            } else {
                let reactor =
                    <crate::connection::ConnectionBuilder<T, Logical> as crate::Reactor>::build(
                        &reactor_name,
                        after.unwrap_or_default(),
                        Some(parent_reactor_key),
                        None,
                        self,
                    )?;
                (reactor.input, reactor.output)
            };

            // Bind the input and output ports to the source and target ports
            self.bind_port(source_key, input_port)?;
            self.bind_port(output_port, target_key)?;

            Ok(())
        }
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port<P1, P2>(&mut self, port_a_key: P1, port_b_key: P2) -> Result<(), BuilderError>
    where
        P1: Into<BuilderPortKey>,
        P2: Into<BuilderPortKey>,
    {
        let port_a_key = port_a_key.into();
        let port_b_key = port_b_key.into();

        let port_a_fqn = self.port_fqn(port_a_key, false)?;
        let port_b_fqn = self.port_fqn(port_b_key, false)?;

        tracing::debug!("Binding ports: {port_a_fqn:?} -> {port_b_fqn:?}",);

        let port_a = &self.port_builders[port_a_key];
        let port_b = &self.port_builders[port_b_key];

        if port_b.get_inward_binding().is_some() {
            return Err(BuilderError::PortConnectionError {
                port_a_key,
                port_b_key,
                what: format!(
                    "Ports may only be connected once, but B is already connected to {:?}",
                    port_b.get_inward_binding()
                ),
            });
        }

        if !port_a.deps().is_empty() {
            return Err(BuilderError::PortConnectionError {
                port_a_key,
                port_b_key,
                what: "Ports with dependencies may not be connected to other ports".to_owned(),
            });
        }

        if port_b.antideps().len() > 0 {
            return Err(BuilderError::PortConnectionError {
                port_a_key,
                port_b_key,
                what: "Ports with antidependencies may not be connected to other ports".to_owned(),
            });
        }

        match (port_a.port_type(), port_b.port_type()) {
            (PortType::Input, PortType::Input) => {
                self.reactor_builders[port_b.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if port_a.get_reactor_key() == parent_key { Some(()) } else { None }
                     }).ok_or(
                        BuilderError::PortConnectionError{
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
                    Err(BuilderError::PortConnectionError{
                        port_a_key,
                        port_b_key,
                        what: format!("An output port ({}) can only be bound to an input port ({}) if both ports belong to reactors in the same hierarichal level", port_a_fqn, port_b_fqn),
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
                        BuilderError::PortConnectionError {
                                port_a_key,
                                port_b_key,
                                what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".to_owned()
                            })
            }
            (PortType::Input, PortType::Output) =>  {
                Err(BuilderError::PortConnectionError {
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
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    pub fn action_fqn(
        &self,
        action_key: BuilderActionKey,
        grouped: bool,
    ) -> Result<BuilderFqn, BuilderError> {
        let action = self
            .action_builders
            .get(action_key)
            .ok_or(BuilderError::ActionKeyNotFound(action_key))?;
        let segment = BuilderFqnSegment::from_action(action, grouped);
        self.reactor_fqn(action.reactor_key(), true)?
            .append(segment)
    }

    /// Get a fully-qualified string for the given ReactionKey
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    pub fn reactor_fqn(
        &self,
        reactor_key: BuilderReactorKey,
        grouped: bool,
    ) -> Result<BuilderFqn, BuilderError> {
        let reactor = self
            .reactor_builders
            .get(reactor_key)
            .ok_or(BuilderError::ReactorKeyNotFound(reactor_key))?;

        let segment = BuilderFqnSegment::from_reactor(reactor, grouped);
        if let Some(parent) = reactor.parent_reactor_key() {
            self.reactor_fqn(parent, grouped)?.append(segment)
        } else {
            Ok(std::iter::once(segment).collect())
        }
    }

    /// Get a fully-qualified string for the given ReactionKey
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    pub fn reaction_fqn(
        &self,
        reaction_key: BuilderReactionKey,
        grouped: bool,
    ) -> Result<BuilderFqn, BuilderError> {
        let reaction = self
            .reaction_builders
            .get(reaction_key)
            .ok_or(BuilderError::ReactionKeyNotFound(reaction_key))?;
        let segment = BuilderFqnSegment::from_reaction(reaction);
        self.reactor_fqn(reaction.reactor_key, grouped)?
            .append(segment)
    }

    /// Get a fully-qualified string for the given PortKey
    ///
    /// If `grouped` is true, the returned Fqn will be grouped by bank
    pub fn port_fqn(
        &self,
        port_key: BuilderPortKey,
        grouped: bool,
    ) -> Result<BuilderFqn, BuilderError> {
        let port = self
            .port_builders
            .get(port_key)
            .ok_or(BuilderError::PortKeyNotFound(port_key))?;
        let segment = BuilderFqnSegment::from_port(port.as_ref(), grouped);
        self.reactor_fqn(port.get_reactor_key(), grouped)?
            .append(segment)
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
            all_triggers.extend(port_builder.triggers().iter().map(|&key| (key, ())));
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
                        self.port_builders[source_port_key].antideps()
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
                .rev()
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
    /// See <https://en.m.wikipedia.org/wiki/Coffman%E2%80%93Graham_algorithm>
    pub fn build_runtime_level_map(
        &self,
    ) -> Result<SecondaryMap<BuilderReactionKey, runtime::Level>, BuilderError> {
        use petgraph::{algo::tred, graph::DefaultIx, graph::NodeIndex};

        let mut graph = self.build_reaction_graph().into_graph::<DefaultIx>();

        // Transitive reduction and closures
        let toposort = petgraph::algo::toposort(&graph, None).map_err(|cycle_error| {
            // A Cycle was found in the reaction graph.

            let res = petgraph::algo::astar(
                &graph,
                cycle_error.node_id(),
                |finish| finish == cycle_error.node_id(),
                |_| 1,
                |_| 0,
            );
            dbg!(res);

            // let fas = petgraph::algo::greedy_feedback_arc_set(&graph);
            // let cycle = petgraph::prelude::DiGraphMap::<BuilderReactionKey, ()>::from_edges(fas);
            let cycle = util::find_minimal_cycle(&graph, cycle_error.node_id())
                .into_iter()
                .map(|node_index| graph[node_index])
                .collect_vec();

            BuilderError::ReactionGraphCycle { what: cycle }
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

        let mut levels: HashMap<_, runtime::Level> = HashMap::new();
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
}
