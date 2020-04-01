use itertools::Itertools;
use std::{collections::BTreeSet, sync::Arc};
use tracing::event;

use super::{
    action::ActionBuilder, port::BasePortBuilder, reaction::ReactionProto,
    reactor::ReactorTypeChildRef, BuilderError, ReactorType, ReactorTypeBuilder,
    ReactorTypeBuilderIndex, ReactorTypeBuilderState, ReactorTypeIndex,
};
use crate::runtime::{self, PortIndex, ReactionIndex};

/// Stores all top-level ReactorTypeBuilder prototypes
#[derive(Debug)]
pub struct EnvBuilder {
    reactor_type_builders: Vec<ReactorTypeBuilder>,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Self {
            reactor_type_builders: Vec::new(),
        }
    }

    pub fn add_reactor_type<F>(&mut self, name: &str, builder_fn: F) -> ReactorTypeBuilderIndex
    where
        F: 'static + Fn(ReactorTypeBuilderState) -> ReactorType,
    {
        let idx = ReactorTypeBuilderIndex(self.reactor_type_builders.len());
        self.reactor_type_builders
            .push(ReactorTypeBuilder::new(name, idx, Box::new(builder_fn)));
        idx
    }

    pub fn build(&mut self, main_ref_index: ReactorTypeBuilderIndex) -> EnvBuilderState {
        let mut env = EnvBuilderState::new();
        let mut stack = Vec::new();
        stack.push(ReactorTypeChildRef {
            name: "main".to_owned(),
            reactor_type_builder_idx: main_ref_index,
            parent_type_idx: None,
            parent_builder_child_ref_idx: None,
        });

        while !stack.is_empty() {
            let child = stack.pop().unwrap();

            // Build a ReactorType from ReactorTypeBuilder
            let reactor_type = self
                .reactor_type_builders
                .get_mut(child.reactor_type_builder_idx.0)
                .map(|type_builder| type_builder.build(&mut env, &child))
                .unwrap();

            stack.extend(reactor_type.children.iter().cloned());
            env.expanded_reactor_types.push(reactor_type);
        }

        // Handle connection definitions in each ReactorType
        let connections = env
            .expanded_reactor_types
            .iter()
            .enumerate()
            .flat_map(|(idx, reactor_type)| {
                env.iter_reactor_type_connections(ReactorTypeIndex(idx), reactor_type)
            })
            .collect::<Vec<_>>();

        // The connections must be collected here since we need to mutably borrow env
        for connection_pair in connections.iter() {
            match *connection_pair {
                (Ok(from_port_idx), Ok(to_port_idx)) => {
                    println!(
                        "Connect {} -> {}",
                        env.port_fqn(from_port_idx).unwrap(),
                        env.port_fqn(to_port_idx).unwrap()
                    );
                    env.port_bind_to(from_port_idx, to_port_idx);
                }
                _ => todo!(),
            };
        }

        env
    }
}

#[derive(Debug)]
pub struct EnvBuilderState {
    pub expanded_reactor_types: Vec<ReactorType>,
    pub ports: Vec<Box<dyn BasePortBuilder>>,
    pub actions: Vec<ActionBuilder>,
    pub reactions: Vec<ReactionProto>,
    pub connections: Vec<(Box<dyn BasePortBuilder>, Box<dyn BasePortBuilder>)>,
}

impl EnvBuilderState {
    pub fn new() -> Self {
        Self {
            expanded_reactor_types: Vec::new(),
            ports: Vec::new(),
            actions: Vec::new(),
            reactions: Vec::new(),
            connections: Vec::new(),
        }
    }

    /// Bind one port to another
    fn port_bind_to(&mut self, port_a_idx: PortIndex, port_b_idx: PortIndex) {
        use super::TupleSlice;
        let (port_a, port_b) = self.ports.tuple_at_mut((port_a_idx.0, port_b_idx.0));

        assert!(
            port_b.get_inward_binding().is_none(),
            format!(
                "Ports may only be connected once {:?}->{:?}, ({:?})",
                port_a_idx,
                port_b_idx,
                port_b.get_inward_binding()
            )
        );
        assert!(
            port_b.get_antideps().is_empty(),
            "Ports with antidependencies may not be connected to other ports"
        );
        port_b.set_inward_binding(Some(port_a_idx));

        assert!(
            port_a.get_deps().is_empty(),
            "Ports with dependencies may not be connected to other ports"
        );
        port_a.add_outward_binding(port_b_idx);

        // match (self.get_port_type(), port.get_port_type()) {
        // (runtime::PortType::Input, runtime::PortType::Input) => {
        // assert!( this->container() == port->container()->container(), "An input port A may only
        // be bound to another input port B if B is contained by a reactor that in turn is contained
        // by the reactor of A"); },
        // (runtime::PortType::Output, runtime::PortType::Input) => {
        // VALIDATE(this->container()->container() == port->container()->container(), "An output
        // port can only be bound to an input port if both ports belong to reactors in the same
        // hierarichal level"); VALIDATE(this->container() != port->container(), "An output
        // port can only be bound to an input port if both ports belong to different reactors!");
        // },
        // (runtime::PortType::Output, runtime::PortType::Output) => {
        // VALIDATE( this->container()->container() == port->container(), "An output port A may only
        // be bound to another output port B if A is contained by a reactor that in turn is
        // contained by the reactor of B"); }
        // }
    }

    pub fn reactor_fqn(&self, reactor_idx: ReactorTypeIndex) -> Result<String, BuilderError> {
        self.expanded_reactor_types
            .get(reactor_idx.0)
            .ok_or(BuilderError::ReactorTypeIndexNotFound(reactor_idx))
            .and_then(|reactor_type| {
                reactor_type.parent_reactor_type_idx.map_or_else(
                    || Ok(reactor_type.name.clone()),
                    |parent| {
                        self.reactor_fqn(parent)
                            .map(|parent| format!("{}.{}", parent, reactor_type.name))
                    },
                )
            })
    }

    pub fn reaction_fqn(&self, reaction_idx: ReactionIndex) -> Result<String, BuilderError> {
        self.reactions
            .get(reaction_idx.0)
            .ok_or(BuilderError::ReactionIndexNotFound(reaction_idx))
            .and_then(|reaction: &ReactionProto| {
                self.reactor_fqn(reaction.reactor_type_idx)
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!("Reactor referenced by {:?} not found: {:?}", reaction, err),
                    })
                    .map(|reactor_fqn| (reactor_fqn, reaction.name.clone()))
            })
            .map(|(reactor_name, reaction_name)| format!("{}.{}", reactor_name, reaction_name))
    }

    pub fn port_fqn(&self, port_idx: PortIndex) -> Result<String, BuilderError> {
        self.ports
            .get(port_idx.0)
            .ok_or(BuilderError::PortIndexNotFound(port_idx))
            .and_then(|port| {
                self.reactor_fqn(port.get_reactor_type_idx())
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!("Reactor referenced by {:?} not found: {:?}", port, err),
                    })
                    .map(|reactor_fqn| (reactor_fqn, port.get_name().clone()))
            })
            .map(|(reactor_name, port_name)| format!("{}.{}", reactor_name, port_name))
    }

    /// Follow the inward_binding's of Ports to the source
    pub fn follow_port_inward_binding(&self, port_idx: PortIndex) -> PortIndex {
        let mut cur_idx = port_idx;
        loop {
            if let Some(new_idx) = self
                .ports
                .get(cur_idx.0)
                .and_then(|port| port.get_inward_binding().as_ref())
            {
                cur_idx = *new_idx;
            } else {
                break;
            }
        }
        cur_idx
    }

    /// Transitively collect all Reactions triggered by this Port being set
    fn collect_transitive_port_triggers(
        &self,
        port_idx: PortIndex,
    ) -> BTreeSet<runtime::ReactionIndex> {
        let mut all_triggers = BTreeSet::new();
        let mut port_set = BTreeSet::new();
        port_set.insert(port_idx);
        while !port_set.is_empty() {
            let port_idx = port_set.pop_first().unwrap();
            let port_builder = &self.ports[port_idx.0];
            all_triggers.extend(port_builder.get_triggers());
            port_set.extend(port_builder.get_outward_bindings());
        }
        all_triggers
    }

    pub(crate) fn reaction_dependency_edges<'a>(
        &'a self,
    ) -> impl Iterator<Item = (runtime::ReactionIndex, runtime::ReactionIndex)> + 'a {
        let deps = self
            .reactions
            .iter()
            .enumerate()
            .flat_map(move |(idx, reaction)| {
                // Connect all reactions this reaction depends upon
                reaction
                    .deps
                    .iter()
                    .flat_map(move |&port_idx| {
                        let source_port_idx = self.follow_port_inward_binding(port_idx);
                        self.ports[source_port_idx.0].get_antideps().iter()
                    })
                    .map(move |dep_idx| (runtime::ReactionIndex(idx), *dep_idx))
            });

        // Connect internal reactions by priority
        let internal = self
            .reactions
            .iter()
            .sorted()
            .enumerate()
            .map(|(idx, _)| runtime::ReactionIndex(idx))
            .tuple_windows();

        deps.chain(internal)
    }

    /// Get an Iterator over the connections in a ReactorType
    /// - parent_reactor_type_idx: The index of the ReactorType
    /// - parent_reactor_type: A reference to the ReactorType
    fn iter_reactor_type_connections<'a>(
        &'a self,
        parent_reactor_type_idx: ReactorTypeIndex,
        parent_reactor_type: &'a ReactorType,
    ) -> impl 'a
           + Iterator<
        Item = (
            Result<PortIndex, BuilderError>,
            Result<PortIndex, BuilderError>,
        ),
    > {
        parent_reactor_type.connections.iter().map(move |connection| {
            let reactor_types_iter = self.expanded_reactor_types.iter()
                .enumerate()
                .filter( |(_, reactor_type)| {
                matches!(reactor_type.parent_reactor_type_idx, Some(idx) if idx == parent_reactor_type_idx)
            });

            let from_idx = match reactor_types_iter.clone().find_map(|(idx,reactor_type)| {
                match reactor_type.parent_builder_child_ref_idx{
                    Some(child_ref_idx)
                        if child_ref_idx == connection.from_reactor_idx => Some((ReactorTypeIndex(idx), connection.from_port.as_str())),
                    _ => None
                }
            })
            .ok_or(|| BuilderError::InconsistentBuilderState{what: format!("ReactorTypeBuilderChildReference not found")}) {
                Ok((reactor_type_idx,port_name)) => self.find_port_idx(reactor_type_idx, port_name),
                Err(error) => Err(error())
            };

            let to_idx = match reactor_types_iter.clone().find_map(|(idx,reactor_type)| {
                match reactor_type.parent_builder_child_ref_idx{
                    Some(child_ref_idx)
                        if child_ref_idx == connection.to_reactor_idx => Some((ReactorTypeIndex(idx), connection.to_port.as_str())),
                    _ => None
                }
            })
            .ok_or(|| BuilderError::InconsistentBuilderState{what: format!("ReactorTypeBuilderChildReference not found")}) {
                Ok((reactor_type_idx,port_name)) => self.find_port_idx(reactor_type_idx, port_name),
                Err(error) => Err(error())
            };

            (from_idx, to_idx)
        })
    }

    /// Find the index of a Port given a ReactorTypeIndex and the port name.
    /// This works by finding the ReactorTypeIndex in the list of ReactorProtos, and then matching
    /// the port name.
    pub fn find_port_idx(
        &self,
        reactor_type_idx: ReactorTypeIndex,
        port_name: &str,
    ) -> Result<PortIndex, BuilderError> {
        self.expanded_reactor_types
            .get(reactor_type_idx.0)
            .ok_or_else(|| BuilderError::ReactorTypeIndexNotFound(reactor_type_idx))
            .and_then(|reactor_type| {
                reactor_type
                    .ports
                    .iter()
                    .find(|&port_idx| self.ports[port_idx.0].get_name().eq(port_name))
                    .copied()
                    .ok_or_else(|| BuilderError::PortNotFound {
                        reactor_name: reactor_type.name.to_owned(),
                        port_name: port_name.to_owned(),
                    })
            })
    }

    pub fn build(mut self) -> Result<runtime::Environment, BuilderError> {
        // Prepare the DAG of Reactions
        let graph = {
            let mut graph = petgraph::graphmap::DiGraphMap::<_, ()>::from_edges(
                self.reaction_dependency_edges().map(|(a, b)| (b, a)),
            );
            // Ensure all ReactionIndicies are represented
            for ix in 0..self.reactions.len() {
                graph.add_node(ReactionIndex(ix));
            }
            graph
        };

        let mut space = petgraph::algo::DfsSpace::new(&graph);
        let ordered_reactions = petgraph::algo::toposort(&graph, Some(&mut space))
            .map_err(|_| BuilderError::ReactionGraphCycle)?;
        // Run dijkstra's algorithm over all nodes to generate the runtime indices
        let runtime_level_map =
            petgraph::algo::dijkstra(&graph, *ordered_reactions.first().unwrap(), None, |_| {
                1usize
            });

        let mut builder_state = runtime::Environment::new();

        // Build Reactions in topo-sorted order
        event!(
            tracing::Level::DEBUG,
            "Building Reactions in order: {:?}",
            ordered_reactions
                .iter()
                .map(|&reaction_idx| (reaction_idx, self.reaction_fqn(reaction_idx).unwrap()))
                .collect::<Vec<_>>()
        );

        for reaction_index in ordered_reactions.iter() {
            let reaction_proto = self.reactions.remove(reaction_index.0);

            // Build all required Ports and aliases
            for &port_idx in reaction_proto
                .antideps
                .iter()
                .chain(reaction_proto.deps.iter())
            {
                let inward_port_idx = self.follow_port_inward_binding(port_idx);

                let port = builder_state
                    .runtime_ports
                    .entry(inward_port_idx)
                    .or_insert_with(|| {
                        let transitive_port_triggers =
                            self.collect_transitive_port_triggers(port_idx);
                        let port_builder = &self.ports[inward_port_idx.0];
                        port_builder.build(transitive_port_triggers)
                    })
                    .clone();

                // We do a second insert in case the port was aliased (inward_port_idx != port_idx)
                builder_state.runtime_ports.insert(port_idx, port);
            }

            let runtime_level: usize = runtime_level_map[reaction_index];
            builder_state.runtime_reactions.insert(
                *reaction_index,
                Arc::new(runtime::Reaction::new(
                    reaction_proto.name,
                    runtime_level,
                    reaction_proto.reaction_fn,
                    None,
                )),
            );
        }

        for (idx, action_builder) in self.actions.iter().enumerate() {
            builder_state
                .runtime_actions
                .insert(runtime::ActionIndex(idx), action_builder.build());
        }

        Ok(builder_state)
    }
}
