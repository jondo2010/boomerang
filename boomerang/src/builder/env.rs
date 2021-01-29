use embedded_time::duration::Seconds;
use itertools::Itertools;
use runtime::PortData;
use slotmap::{DefaultKey, Key, SecondaryMap, SlotMap};
use std::{collections::BTreeSet, sync::Arc};
use tracing::event;

use crate::runtime;

use super::{
    action::ActionBuilder,
    port::{self, BasePortBuilder},
    reaction::ReactionBuilder,
    BuilderError, Reactor, ReactorBuilder, ReactorBuilderState,
};
#[derive(Debug)]
pub struct EnvBuilder {
    pub(super) reactors: SlotMap<runtime::ReactorKey, ReactorBuilder>,

    pub(super) ports: SlotMap<runtime::BasePortKey, Arc<dyn runtime::BasePort>>,
    pub(super) port_builders: SecondaryMap<runtime::BasePortKey, Box<dyn BasePortBuilder>>,

    pub(super) actions: SlotMap<runtime::BaseActionKey, ActionBuilder>,
    pub(super) reactions: SlotMap<runtime::ReactionKey, ReactionBuilder>,
    pub(super) connections: Vec<(Box<dyn BasePortBuilder>, Box<dyn BasePortBuilder>)>,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Self {
            reactors: SlotMap::<runtime::ReactorKey, ReactorBuilder>::with_key(),
            ports: SlotMap::new(),
            port_builders: SecondaryMap::new(),
            actions: SlotMap::new(),
            // reactions: Vec::new(),
            reactions: SlotMap::with_key(),
            connections: Vec::new(),
        }
    }

    /// Add a new Reactor
    /// - name: Instance name of the reactor
    pub fn add_reactor<R: Reactor>(
        &mut self,
        name: &str,
        parent: Option<runtime::ReactorKey>,
        reactor: R,
    ) -> ReactorBuilderState<R> {
        ReactorBuilderState::new(name, parent, reactor, self)
    }

    /// Bind one port to another
    fn port_bind_to<T: runtime::PortData>(
        &mut self,
        port_a_key: runtime::PortKey<T>,
        port_b_key: runtime::PortKey<T>,
    ) {
        let [port_a, port_b] = self
            .port_builders
            .get_disjoint_mut([port_a_key.data().into(), port_b_key.data().into()])
            .unwrap();

        assert!(
            port_b.get_inward_binding().is_none(),
            format!(
                "Ports may only be connected once {:?}->{:?}, ({:?})",
                port_a_key,
                port_b_key,
                port_b.get_inward_binding()
            )
        );
        assert!(
            port_b.get_antideps().is_empty(),
            "Ports with antidependencies may not be connected to other ports"
        );
        port_b.set_inward_binding(Some(port_a_key.data().into()));

        assert!(
            port_a.get_deps().is_empty(),
            "Ports with dependencies may not be connected to other ports"
        );
        port_a.add_outward_binding(port_b_key.data().into());

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

    pub fn reactor_fqn(&self, reactor_key: runtime::ReactorKey) -> Result<String, BuilderError> {
        self.reactors
            .get(reactor_key)
            .ok_or(BuilderError::ReactorKeyNotFound(reactor_key))
            .and_then(|reactor| {
                reactor.parent_reactor_key.map_or_else(
                    || Ok(reactor.name.clone()),
                    |parent| {
                        self.reactor_fqn(parent)
                            .map(|parent| format!("{}.{}", parent, reactor.name))
                    },
                )
            })
    }

    pub fn reaction_fqn(&self, reaction_key: runtime::ReactionKey) -> Result<String, BuilderError> {
        self.reactions
            .get(reaction_key)
            .ok_or(BuilderError::ReactionKeyNotFound(reaction_key))
            .and_then(|reaction| {
                self.reactor_fqn(reaction.reactor_key)
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!("Reactor referenced by {:?} not found: {:?}", reaction, err),
                    })
                    .map(|reactor_fqn| (reactor_fqn, reaction.name.clone()))
            })
            .map(|(reactor_name, reaction_name)| format!("{}.{}", reactor_name, reaction_name))
    }

    pub fn port_fqn<T: runtime::PortData>(
        &self,
        port_key: runtime::PortKey<T>,
    ) -> Result<String, BuilderError> {
        let port_key = port_key.data().into();
        self.port_builders
            .get(port_key)
            .ok_or(BuilderError::PortKeyNotFound(port_key))
            .and_then(|port_builder| {
                self.reactor_fqn(port_builder.get_reactor_key())
                    .map_err(|err| BuilderError::InconsistentBuilderState {
                        what: format!(
                            "Reactor referenced by {:?} not found: {:?}",
                            port_builder, err
                        ),
                    })
                    .map(|reactor_fqn| (reactor_fqn, port_builder.get_name().clone()))
            })
            .map(|(reactor_name, port_name)| format!("{}.{}", reactor_name, port_name))
    }

    /// Follow the inward_binding's of Ports to the source
    pub fn follow_port_inward_binding(
        &self,
        port_key: runtime::BasePortKey,
    ) -> runtime::BasePortKey {
        let mut cur_key = port_key;
        loop {
            if let Some(new_idx) = self
                .port_builders
                .get(cur_key)
                .and_then(|port| port.get_inward_binding().as_ref())
            {
                cur_key = *new_idx;
            } else {
                break;
            }
        }
        cur_key
    }

    /// Transitively collect all Reactions triggered by this Port being set
    fn collect_transitive_port_triggers(
        &self,
        port_key: runtime::BasePortKey,
    ) -> SecondaryMap<runtime::ReactionKey, ()> {
        let mut all_triggers = SecondaryMap::new();
        let mut port_set = BTreeSet::<runtime::BasePortKey>::new();
        port_set.insert(port_key);
        while !port_set.is_empty() {
            let port_key = port_set.pop_first().unwrap();
            let port_builder = &self.port_builders[port_key];
            all_triggers.extend(port_builder.get_triggers().iter().map(|&key| (key, ())));
            port_set.extend(port_builder.get_outward_bindings());
        }
        all_triggers
    }

    pub(crate) fn reaction_dependency_edges<'a>(
        &'a self,
    ) -> impl Iterator<Item = (runtime::ReactionKey, runtime::ReactionKey)> + 'a {
        let deps = self
            .reactions
            .iter()
            .flat_map(move |(reaction_key, reaction)| {
                // Connect all reactions this reaction depends upon
                reaction
                    .deps
                    .keys()
                    .flat_map(move |port_key| {
                        let source_port_key = self.follow_port_inward_binding(port_key);
                        self.port_builders[source_port_key.data().into()]
                            .get_antideps()
                            .iter()
                            .copied()
                    })
                    .map(move |dep_key| (reaction_key, dep_key))
            });

        // Connect internal reactions by priority
        let internal = self
            .reactions
            .iter()
            .sorted()
            .map(|(key, _)| key)
            .tuple_windows();

        deps.chain(internal)
    }

    /// Get an Iterator over the connections in a Reactor
    /// - parent_reactor_type_idx: The index of the Reactor
    /// - parent_reactor_type: A reference to the Reactor
    #[cfg(feature = "old")]
    fn iter_reactor_type_connections<'a, T: PortData>(
        &'a self,
        parent_reactor_type_idx: runtime::ReactorKey,
        parent_reactor_type: &'a ReactorBuilder,
    ) -> impl 'a
           + Iterator<
        Item = (
            Result<runtime::PortKey<T>, BuilderError>,
            Result<runtime::PortKey<T>, BuilderError>,
        ),
    > {
        parent_reactor_type.connections.iter().map(move |connection| {
            let reactor_types_iter = self.reactors.iter()
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
                Ok((reactor_type_idx,port_name)) => self.find_port_key(reactor_type_idx, port_name),
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
                Ok((reactor_type_idx,port_name)) => self.find_port_key(reactor_type_idx, port_name),
                Err(error) => Err(error())
            };

            (from_idx, to_idx)
        })
    }

    /// Find the index of a Port given a runtime::ReactorKey and the port name.
    /// This works by finding the runtime::ReactorKey in the list of ReactorBuilder, and then
    /// matching the port name.
    pub fn find_port_key<T: PortData>(
        &self,
        reactor_key: runtime::ReactorKey,
        port_name: &str,
    ) -> Result<runtime::PortKey<T>, BuilderError> {
        self.reactors
            .get(reactor_key)
            .ok_or_else(|| BuilderError::ReactorKeyNotFound(reactor_key))
            .and_then(|reactor_type| {
                reactor_type
                    .ports
                    .keys()
                    .find(|&port_key| self.port_builders[port_key].get_name().eq(port_name))
                    .map(|base_key| base_key.data().into())
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
            self.reactions.keys().for_each(|key| {
                graph.add_node(key);
            });
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

        let mut builder_state = runtime::Environment {
            ports: self.ports,
            port_triggers: SecondaryMap::new(),
            runtime_actions: SlotMap::new(),
            reactions: SlotMap::with_key(),
        };

        // Build Reactions in topo-sorted order
        event!(
            tracing::Level::DEBUG,
            "Building Reactions in order: {:?}",
            ordered_reactions
                .iter()
                .map(|reaction_key| (reaction_key, self.reaction_fqn(*reaction_key).unwrap()))
                .collect::<Vec<_>>()
        );

        for reaction_key in ordered_reactions.iter() {
            let reaction = self.reactions.remove(*reaction_key).unwrap();

            // Build all required Ports and aliases
            for port_key in reaction.antideps.keys().chain(reaction.deps.keys()) {
                let inward_port_key = self.follow_port_inward_binding(port_key);
                if inward_port_key == port_key {
                    // Only set the port_triggers for non-aliased ports.
                    let transitive_port_triggers = self.collect_transitive_port_triggers(port_key);
                    builder_state.port_triggers[inward_port_key].extend(transitive_port_triggers);
                } else {
                    builder_state.ports[port_key] = builder_state.ports[inward_port_key].clone();
                }
            }

            let runtime_level: usize = runtime_level_map[reaction_key];
            builder_state.reactions.insert(
                //*reaction_key,
                Arc::new(runtime::Reaction::new(
                    reaction.name,
                    runtime_level,
                    reaction.reaction_fn,
                    None,
                )),
            );
        }

        // for (idx, action_builder) in self.actions.iter().enumerate() {
        // builder_state
        // .runtime_actions
        // .insert(runtime::ActionIndex(idx), action_builder.build());
        // }

        Ok(builder_state)
    }
}
