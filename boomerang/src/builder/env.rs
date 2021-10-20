use super::{ActionType, BuilderError, PortBuilder, PortType, Reactor, ReactorBuilder, ReactorBuilderState, action::ActionBuilder, port::BasePortBuilder, reaction::ReactionBuilder};
use crate::runtime;
use itertools::Itertools;
use slotmap::{SecondaryMap, SlotMap};
use std::{
    collections::{BTreeSet, HashMap},
    convert::TryInto,
    sync::Arc,
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct EnvBuilder<S> {
    pub(super) ports: SlotMap<runtime::PortKey, Arc<dyn runtime::BasePort>>,
    pub(super) port_builders: SecondaryMap<runtime::PortKey, Box<dyn BasePortBuilder>>,

    pub(super) actions: SlotMap<runtime::ActionKey, Arc<dyn runtime::BaseAction<S>>>,
    pub(super) action_builders: SecondaryMap<runtime::ActionKey, ActionBuilder>,

    pub(super) reaction_builders: SlotMap<runtime::ReactionKey, ReactionBuilder<S>>,

    pub(super) reactors: SlotMap<runtime::ReactorKey, ReactorBuilder>,
}

impl<S> EnvBuilder<S>
where
    S: runtime::SchedulerPoint,
{
    pub fn new() -> Self {
        Self {
            ports: SlotMap::with_key(),
            port_builders: SecondaryMap::new(),
            actions: SlotMap::with_key(),
            action_builders: SecondaryMap::new(),
            reaction_builders: SlotMap::with_key(),
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
    ) -> ReactorBuilderState<S, R>
    where
        R: Reactor<S>,
    {
        ReactorBuilderState::new(name, parent, reactor, self)
    }

    pub fn add_port<T: runtime::PortData>(
        &mut self,
        name: &str,
        port_type: PortType,
        reactor_key: runtime::ReactorKey,
    ) -> Result<runtime::PortKey, BuilderError> {
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

        self.reactors[reactor_key].ports.insert(key, ());

        Ok(key.into())
    }

    pub fn add_timer(
        &mut self,
        name: &str,
        period: runtime::Duration,
        offset: runtime::Duration,
        reactor_key: runtime::ReactorKey,
    ) -> Result<runtime::ActionKey, BuilderError> {
        let key = self.add_action(name, reactor_key, |action_key| {
            Arc::new(runtime::Timer::new(name, action_key, offset, period))
        })?;

        self.action_builders.insert(
            key,
            ActionBuilder::new(name, ActionType::Timer { period, offset }, key, reactor_key),
        );

        Ok(key)
    }

    pub fn add_logical_action<T: runtime::PortData>(
        &mut self,
        name: &str,
        min_delay: Option<runtime::Duration>,
        reactor_key: runtime::ReactorKey,
    ) -> Result<runtime::ActionKey, BuilderError> {
        let key = self.add_action(name, reactor_key, |_| {
            Arc::new(runtime::Action::<T>::new(
                name,
                true,
                min_delay.unwrap_or_default(),
            ))
        })?;
        self.action_builders.insert(
            key,
            ActionBuilder::new(name, ActionType::Logical { min_delay }, key, reactor_key),
        );
        Ok(key.into())
    }

    /// Add an Action to a given Reactor using closure F
    pub fn add_action<F: Fn(runtime::ActionKey) -> Arc<dyn runtime::BaseAction<S>>>(
        &mut self,
        name: &str,
        reactor_key: runtime::ReactorKey,
        action_fn: F,
    ) -> Result<runtime::ActionKey, BuilderError> {
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

        let key = self
            .actions
            .insert_with_key(|action_key| action_fn(action_key));
        self.reactors[reactor_key].actions.insert(key, ());
        Ok(key.into())
    }

    /// Bind Port A to Port B
    /// The nominal case is to bind Input A to Output B
    pub fn bind_port(
        &mut self,
        port_a_key: runtime::PortKey,
        port_b_key: runtime::PortKey,
    ) -> Result<(), BuilderError> {
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

        if port_a.get_deps().len() > 0 {
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
                self.reactors[port_b.get_reactor_key()]
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
                let port_a_grandparent = self.reactors[port_a.get_reactor_key()].parent_reactor_key;
                let port_b_grandparent = self.reactors[port_b.get_reactor_key()].parent_reactor_key;
                // VALIDATE(this->container()->container() == port->container()->container(), 
                if !matches!((port_a_grandparent, port_b_grandparent), (Some(key_a), Some(key_b)) if key_a == key_b) {
                    Err(BuilderError::PortBindError{
                        port_a_key: port_a_key,
                        port_b_key: port_b_key,
                        what: "An output port can only be bound to an input port if both ports belong to reactors in the same hierarichal level".to_owned(),
                    })
                }
                // VALIDATE(this->container() != port->container(), );
                else if port_a.get_reactor_key() == port_b.get_reactor_key() {
                    Err(BuilderError::PortBindError{
                        port_a_key: port_a_key,
                        port_b_key: port_b_key,
                        what: "An output port can only be bound to an input port if both ports belong to different reactors!".to_owned(),
                    })
                }
                else {
                    Ok(())
                }
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
                                port_a_key: port_a_key,
                                port_b_key: port_b_key,
                                what: "An output port A may only be bound to another output port B if A is contained by a reactor that in turn is contained by the reactor of B".to_owned()
                            })
            }
            (PortType::Input, PortType::Output) =>  {
                Err(BuilderError::PortBindError {
                    port_a_key: port_a_key,
                    port_b_key: port_b_key,
                    what: "Unexpected case: can't bind an input Port to an output Port.".to_owned()
                })
            }
        }?;

        port_b.set_inward_binding(Some(port_a_key));
        port_a.add_outward_binding(port_b_key);

        Ok(())
    }

    /// Get a fully-qualified string for the given ReactionKey
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

    /// Get a fully-qualified string for the given ReactionKey
    pub fn reaction_fqn(&self, reaction_key: runtime::ReactionKey) -> Result<String, BuilderError> {
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
            .map(|(reactor_name, reaction_name)| format!("{}/{}", reactor_name, reaction_name))
    }

    /// Get a fully-qualified string for the given PortKey
    pub fn port_fqn<T: runtime::PortData>(
        &self,
        port_key: runtime::PortKey,
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
    pub fn follow_port_inward_binding(&self, port_key: runtime::PortKey) -> runtime::PortKey {
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
        port_key: runtime::PortKey,
    ) -> SecondaryMap<runtime::ReactionKey, ()> {
        let mut all_triggers = SecondaryMap::new();
        let mut port_set = BTreeSet::<runtime::PortKey>::new();
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
    ) -> impl Iterator<Item = (runtime::ReactionKey, runtime::ReactionKey)> + 'b {
        let deps = self
            .reaction_builders
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
                .sorted_by_key(|&reaction_key| self.reaction_builders[reaction_key].priority)
                .tuple_windows()
        });

        deps.chain(internal)
    }

    /// Prepare the DAG of Reactions
    fn get_reaction_graph(&self) -> petgraph::graphmap::DiGraphMap<runtime::ReactionKey, ()> {
        let mut graph = petgraph::graphmap::DiGraphMap::from_edges(
            self.reaction_dependency_edges().map(|(a, b)| (b, a)),
        );
        // Ensure all ReactionIndicies are represented
        self.reaction_builders.keys().for_each(|key| {
            graph.add_node(key);
        });

        graph
    }

    /// Build a DAG of Reactors
    pub fn build_reactor_graph(&self) -> petgraph::graphmap::DiGraphMap<runtime::ReactorKey, ()> {
        let mut graph = petgraph::graphmap::DiGraphMap::from_edges(
            self.reactors.iter().filter_map(|(key, reactor)| {
                reactor
                    .parent_reactor_key
                    .map(|parent_key| (parent_key, key))
            }),
        );
        // ensure all Reactors are represented
        self.reactors.keys().for_each(|key| {
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

impl<S> TryInto<runtime::Env<S>> for EnvBuilder<S>
where
    S: runtime::SchedulerPoint,
{
    type Error = BuilderError;
    fn try_into(self) -> Result<runtime::Env<S>, Self::Error> {
        use tracing::event;

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
            runtime::PortKey,
            SecondaryMap<runtime::ReactionKey, ()>,
        > = SecondaryMap::new();
        let runtime_actions = self.actions.clone();
        let runtime_action_triggers = self
            .action_builders
            .iter()
            .map(|(action_key, action_builder)| (action_key, action_builder.triggers.clone()))
            .collect();

        for reaction_builder in self.reaction_builders.values() {
            // Build all required Ports and aliases
            for port_key in reaction_builder
                .antideps
                .keys()
                .chain(reaction_builder.deps.keys())
            {
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
        }

        // Build the SlotMap of runtime::Reaction from the ReactionBuilders.
        // This depends on iter() being stable and that inserting in the same order results in the
        // same keys.
        let reactions = {
            let mut reactions = SlotMap::with_key();
            for (key, builder) in self.reaction_builders.into_iter() {
                let new_key = reactions.insert(runtime::Reaction::new(
                    builder.name,
                    runtime_level_map[&key],
                    builder.reaction_fn,
                    None,
                ));
                assert!(key == new_key);
            }
            reactions
        };

        Ok(runtime::Env {
            ports: runtime_ports,
            port_triggers: runtime_port_triggers,
            actions: runtime_actions,
            action_triggers: runtime_action_triggers,
            reactions: reactions,
        })
    }
}
