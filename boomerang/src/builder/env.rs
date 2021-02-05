use super::{
    action::{ActionBuilder},
    port::BasePortBuilder,
    reaction::ReactionBuilder,
    BuilderError, PortBuilder, PortType, Reactor, ReactorBuilder, ReactorBuilderState, ReactorPart,
};
use crate::runtime;
use itertools::Itertools;
use runtime::{PortData, PortKey};
use slotmap::{SecondaryMap, SlotMap};
use std::{
    collections::{BTreeSet, HashMap},
    convert::TryInto,
    sync::Arc,
};
use tracing::event;

#[derive(Debug)]
pub struct EnvBuilder {
    pub(super) ports: SlotMap<runtime::BasePortKey, Arc<dyn runtime::BasePort>>,
    pub(super) port_builders: SecondaryMap<runtime::BasePortKey, Box<dyn BasePortBuilder>>,

    pub(super) actions: SlotMap<runtime::BaseActionKey, Arc<dyn runtime::BaseAction>>,
    pub(super) action_builders: SecondaryMap<runtime::BaseActionKey, ActionBuilder>,

    pub(super) reactions: SlotMap<runtime::ReactionKey, ReactionBuilder>,

    pub(super) reactors: SlotMap<runtime::ReactorKey, ReactorBuilder>,
}

impl EnvBuilder {
    pub fn new() -> Self {
        Self {
            ports: SlotMap::with_key(),
            port_builders: SecondaryMap::new(),
            actions: SlotMap::with_key(),
            action_builders: SecondaryMap::new(),
            reactions: SlotMap::with_key(),
            reactors: SlotMap::<runtime::ReactorKey, ReactorBuilder>::with_key(),
        }
    }

    /// Add a new Reactor
    /// - name: Instance name of the reactor
    pub fn add_reactor<R>(
        &mut self,
        name: &str,
        parent: Option<runtime::ReactorKey>,
        reactor: R,
    ) -> ReactorBuilderState<R>
    where
        R: Reactor,
        R::Inputs: ReactorPart,
        R::Outputs: ReactorPart,
        R::Actions: ReactorPart,
    {
        ReactorBuilderState::new(name, parent, reactor, self)
    }

    pub fn add_port<T: PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
        reactor_key: runtime::ReactorKey,
    ) -> Result<PortKey<T>, BuilderError> {
        // Ensure no duplicates
        if self
            .port_builders
            .values()
            .find(|&port| port.get_name() == name && port.get_reactor_key() == reactor_key)
            .is_some()
        {
            return Err(BuilderError::DuplicatePortDefinition {
                reactor_name: self.reactors[reactor_key].name.clone(),
                port_name: name.into(),
            });
        }

        let port_builders = &mut self.port_builders;
        let key = self.ports.insert_with_key(|port_key| {
            port_builders.insert(
                port_key,
                Box::new(PortBuilder::<T>::new(name, reactor_key, port_type)),
            );
            Arc::new(runtime::Port::<T>::new(name.to_owned()))
        });

        Ok(key.into())
    }
    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
        reactor_key: runtime::ReactorKey,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        let key = self.add_action(name, reactor_key, |action_key| {
            Arc::new(runtime::Timer::new(name, action_key, offset, period))
        })?;

        Ok(key)
    }

    pub fn add_logical_action<T: runtime::PortData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<runtime::ActionKey<T>, BuilderError> {
        let key = self.add_action(name, reactor_key, |_| {
            Arc::new(runtime::Action::<T>::new(
                name,
                true,
                min_delay.unwrap_or_default(),
            ))
        })?;
        Ok(key.into())
    }

    pub fn add_action<F: Fn(runtime::BaseActionKey) -> Arc<dyn runtime::BaseAction>>(
        &mut self,
        name: &str,
        reactor_key: runtime::ReactorKey,
        action_fn: F,
    ) -> Result<runtime::BaseActionKey, BuilderError> {
        // Ensure no duplicates
        if self
            .action_builders
            .values()
            .find(|&action_builder| {
                action_builder.get_name() == name && action_builder.get_reactor_key() == reactor_key
            })
            .is_some()
        {
            return Err(BuilderError::DuplicateActionDefinition {
                reactor_name: self.reactors[reactor_key].name.clone(),
                action_name: name.into(),
            });
        }

        let action_builders = &mut self.action_builders;
        let key = self.actions.insert_with_key(|action_key| {
            action_builders.insert(
                action_key,
                ActionBuilder::new(name, action_key, reactor_key),
            );
            action_fn(action_key)
        });
        Ok(key.into())
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port<T: runtime::PortData>(
        &mut self,
        port_a_key: runtime::PortKey<T>,
        port_b_key: runtime::PortKey<T>,
    ) -> Result<(), BuilderError> {
        let [port_a, port_b] = self
            .port_builders
            .get_disjoint_mut([port_a_key.into(), port_b_key.into()])
            .unwrap();

        if port_b.get_inward_binding().is_some() {
            return Err(BuilderError::PortBindError {
                port_a_key: port_a_key.into(),
                port_b_key: port_b_key.into(),
                what: format!(
                    "Ports may only be connected once, but B is already connected to {:?}",
                    port_b.get_inward_binding()
                ),
            });
        }

        if port_a.get_deps().len() > 0 {
            return Err(BuilderError::PortBindError {
                port_a_key: port_a_key.into(),
                port_b_key: port_b_key.into(),
                what: "Ports with dependencies may not be connected to other ports".to_owned(),
            });
        }

        if port_b.get_antideps().len() > 0 {
            return Err(BuilderError::PortBindError {
                port_a_key: port_a_key.into(),
                port_b_key: port_b_key.into(),
                what: "Ports with antidependencies may not be connected to other ports".to_owned(),
            });
        }

        match (port_a.get_port_type(), port_b.get_port_type()) {
            (PortType::Input, PortType::Input) => {
                self.reactors[port_b.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if port_a.get_reactor_key() == parent_key { Some(()) } else { None }
                     }).ok_or(
                        BuilderError::PortBindError{
                                port_a_key: port_a_key.into(),
                                port_b_key: port_b_key.into(),
                                what: "An input port A may only be bound to another input port B if B is contained by a reactor that in turn is contained by the reactor of A.".into()
                            })
            }
            (PortType::Output, PortType::Input) => {
                // VALIDATE(this->container()->container() == port->container()->container(), "An output port can only be bound to an input port if both ports belong to reactors in the same hierarichal level"
                // VALIDATE(this->container() != port->container(), "An output port can only be bound to an input port if both ports belong to different reactors!");
                Ok(())
            }
            (PortType::Output, PortType::Output) => {
                // VALIDATE( this->container()->container() == port->container(),
                self.reactors[port_a.get_reactor_key()]
                    .parent_reactor_key
                    .and_then(|parent_key| {
                        if parent_key == port_b.get_reactor_key() {
                            Some(())
                        } else {
                            None
                        }
                    }).ok_or(
                        BuilderError::PortBindError {
                                port_a_key: port_a_key.into(),
                                port_b_key: port_b_key.into(),
                                what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".to_owned()
                            })
            }
            (PortType::Input, PortType::Output) =>  {
                Err(BuilderError::PortBindError {
                    port_a_key: port_a_key.into(),
                    port_b_key: port_b_key.into(),
                    what: "Unexpected case: can't bind an input Port to an output Port.".to_owned()
                })
            }
        }?;

        port_b.set_inward_binding(Some(port_a_key.into()));
        port_a.add_outward_binding(port_b_key.into());

        Ok(())
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
                            .map(|parent| format!("{}/{}", parent, reactor.name))
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
            .map(|(reactor_name, reaction_name)| format!("{}/{}", reactor_name, reaction_name))
    }

    pub fn port_fqn<T: runtime::PortData>(
        &self,
        port_key: runtime::PortKey<T>,
    ) -> Result<String, BuilderError> {
        let port_key = port_key.into();
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
                .and_then(|port| port.get_inward_binding())
            {
                cur_key = new_idx;
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

    pub fn reaction_dependency_edges<'a>(
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
                        self.port_builders[source_port_key.into()].get_antideps()
                    })
                    .map(move |dep_key| (reaction_key, dep_key))
            });

        // For all Reactions within a Reactor, create a chain of dependencies by priority
        let internal = self.reactors.values().flat_map(move |reactor| {
            reactor
                .reactions
                .keys()
                .sorted_by_key(|&reaction_key| self.reactions[reaction_key].priority)
                .tuple_windows()
        });

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
                    .map(|base_key| base_key.into())
                    .ok_or_else(|| BuilderError::PortNotFound {
                        reactor_name: reactor_type.name.to_owned(),
                        port_name: port_name.to_owned(),
                    })
            })
    }

    /// Prepare the DAG of Reactions
    fn get_reaction_graph(&self) -> petgraph::graphmap::DiGraphMap<runtime::ReactionKey, ()> {
        let mut graph = petgraph::graphmap::DiGraphMap::from_edges(
            self.reaction_dependency_edges().map(|(a, b)| (b, a)),
        );
        // Ensure all ReactionIndicies are represented
        self.reactions.keys().for_each(|key| {
            graph.add_node(key);
        });

        graph
    }
}

/// Build a HashMap of the runtime-levels for each node corresponding to the parallelizable
/// schedule.
fn build_runtime_level_map<N, E>(graph: &petgraph::graphmap::DiGraphMap<N, E>) -> HashMap<N, usize>
where
    N: petgraph::graphmap::NodeTrait,
{
    graph
        .nodes()
        .filter_map(|reaction_key| {
            // Filter all nodes that have no incoming edges
            if graph
                .neighbors_directed(reaction_key, petgraph::EdgeDirection::Incoming)
                .count()
                == 0
            {
                // Run Dijkstra on each of them
                Some(petgraph::algo::dijkstra(&graph, reaction_key, None, |_| {
                    1usize
                }))
            } else {
                None
            }
        })
        // Now fold the resultant (Node -> level) maps into a single one.
        .fold(HashMap::new(), |mut acc, fold| {
            for (&key, &level) in fold.iter() {
                let entry = acc.entry(key).or_insert(level);
                if level > *entry {
                    *entry = level;
                }
            }
            acc
        })
}

impl TryInto<runtime::Environment> for EnvBuilder {
    type Error = BuilderError;
    fn try_into(mut self) -> Result<runtime::Environment, Self::Error> {
        let graph = self.get_reaction_graph();

        let ordered_reactions =
            petgraph::algo::toposort(&graph, None).map_err(|_| BuilderError::ReactionGraphCycle)?;

        let runtime_level_map = build_runtime_level_map(&graph);

        event!(
            tracing::Level::DEBUG,
            "reaction_dependency_edges: {:#?}",
            self.reaction_dependency_edges()
                .map(|(a, b)| {
                    format!(
                        "{} : {:?} -> {} : {:?}",
                        self.reaction_fqn(a).unwrap(),
                        a,
                        self.reaction_fqn(b).unwrap(),
                        b
                    )
                })
                .collect::<Vec<_>>()
        );

        event!(
            tracing::Level::DEBUG,
            "runtime_level_map: {:?}",
            runtime_level_map
                .iter()
                .map(|(&key, level)| format!("{:?}: {}", key, level))
        );

        // Build Reactions in topo-sorted order
        event!(
            tracing::Level::DEBUG,
            "Building Reactions in order: {:?}",
            ordered_reactions
                .iter()
                .map(|reaction_key| (reaction_key, self.reaction_fqn(*reaction_key).unwrap())) /*  */
        );

        let mut runtime_ports = self.ports.clone();
        let mut runtime_port_triggers: SecondaryMap<
            runtime::BasePortKey,
            SecondaryMap<runtime::ReactionKey, ()>,
        > = SecondaryMap::new();
        let runtime_actions = self.actions.clone();
        let runtime_action_triggers = self
            .action_builders
            .iter()
            .map(|(action_key, action_builder)| (action_key, action_builder.triggers.clone()))
            .collect();

        let mut runtime_reactions = SlotMap::with_key();

        for reaction_key in ordered_reactions.iter() {
            let reaction = self.reactions.remove(*reaction_key).unwrap();

            // Build all required Ports and aliases
            for port_key in reaction.antideps.keys().chain(reaction.deps.keys()) {
                let inward_port_key = self.follow_port_inward_binding(port_key);
                if inward_port_key == port_key {
                    // Only set the port_triggers for non-aliased ports.
                    let transitive_port_triggers = self.collect_transitive_port_triggers(port_key);
                    runtime_port_triggers
                        .entry(inward_port_key)
                        .unwrap()
                        .or_insert(SecondaryMap::new())
                        .extend(transitive_port_triggers);
                } else {
                    runtime_ports[port_key] = runtime_ports[inward_port_key].clone();
                }
            }

            runtime_reactions.insert(Arc::new(runtime::Reaction::new(
                reaction.name,
                runtime_level_map[reaction_key],
                reaction.reaction_fn,
                None,
            )));
        }

        Ok(runtime::Environment {
            ports: runtime_ports,
            port_triggers: runtime_port_triggers,
            actions: runtime_actions,
            action_triggers: runtime_action_triggers,
            reactions: runtime_reactions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::tests::*;

    #[test]
    fn test_duplicate_ports() {
        let mut env_builder = EnvBuilder::new();
        let (reactor_key, _, _) = env_builder
            .add_reactor("test_reactor", None, TestReactorDummy)
            .finish()
            .unwrap();
        let _ = env_builder
            .add_port::<()>("port0", PortType::Input, reactor_key)
            .unwrap();
        assert_eq!(
            env_builder
                .add_port::<()>("port0", PortType::Output, reactor_key)
                .expect_err("Expected duplicate"),
            BuilderError::DuplicatePortDefinition {
                reactor_name: "test_reactor".into(),
                port_name: "port0".into(),
            }
        );
    }

    #[test]
    fn test_duplicate_actions() {
        let mut env_builder = EnvBuilder::new();
        let (reactor_key, _, _) = env_builder
            .add_reactor("test_reactor", None, TestReactorDummy)
            .finish()
            .unwrap();

        env_builder
            .add_logical_action::<()>("action0", None, reactor_key)
            .unwrap();

        assert_eq!(
            env_builder
                .add_logical_action::<()>("action0", None, reactor_key)
                .expect_err("Expected duplicate"),
            BuilderError::DuplicateActionDefinition {
                reactor_name: "test_reactor".into(),
                action_name: "action0".into(),
            }
        );

        assert_eq!(
            env_builder
                .add_timer(
                    "action0",
                    runtime::Duration::from_micros(0),
                    runtime::Duration::from_micros(0),
                    reactor_key
                )
                .expect_err("Expected duplicate"),
            BuilderError::DuplicateActionDefinition {
                reactor_name: "test_reactor".into(),
                action_name: "action0".into(),
            }
        )
    }

    #[test]
    fn test_reactions1() {
        let mut env_builder = EnvBuilder::new();
        let mut reactor_builder = env_builder.add_reactor("test_reactor", None, TestReactorDummy);

        let r0_key = reactor_builder
            .add_reaction(|_, _, _, _, _| {})
            .finish()
            .unwrap();
        let r1_key = reactor_builder
            .add_reaction(|_, _, _, _, _| {})
            .finish()
            .unwrap();
        let (reactor_key, _, _) = reactor_builder.finish().unwrap();

        assert_eq!(env_builder.reactors.len(), 1);
        assert_eq!(env_builder.reactions.len(), 2);
        assert_eq!(
            env_builder.reactions.keys().collect::<Vec<_>>(),
            vec![r0_key, r1_key]
        );

        assert_eq!(env_builder.reactors[reactor_key].reactions.len(), 2);

        let dep_edges = env_builder.reaction_dependency_edges().collect::<Vec<_>>();
        assert_eq!(dep_edges, vec![(r0_key, r1_key)]);

        let env: runtime::Environment = env_builder.try_into().unwrap();
        assert_eq!(env.reactions.len(), 2);
    }
}
