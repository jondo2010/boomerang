use boomerang_runtime::{self as runtime};
use itertools::Itertools;
use slotmap::SecondaryMap;

use crate::{
    connection::PortBindings, ActionType, BuilderActionKey, BuilderError, BuilderPortKey,
    BuilderReactionKey, BuilderReactorKey, ParentReactorBuilder, TriggerMode,
};

use super::{collect_transitive_port_triggers, EnvBuilder};

/// Runtime parts of an enclave
#[derive(Debug, Default)]
pub struct EnclaveParts {
    /// The runtime Enclave
    pub enclave: runtime::Enclave,
    /// Aliases from builder keys to runtime keys
    pub aliases: BuilderAliases,
}

pub type EnclavePartsMap = SecondaryMap<BuilderReactorKey, EnclaveParts>;

/// A trait used to defer the building of until the enclave parts are available.
pub trait DeferedBuild {
    type Output;

    fn defer(self) -> impl FnOnce(&EnclavePartsMap) -> Self::Output + 'static;
}

//F: for<'any> FnOnce(&'any EnclavePartsMap) -> runtime::BoxedReactionFn + 'static,
impl<Reaction, State> DeferedBuild for runtime::ReactionAdapter<Reaction, State>
where
    Reaction: runtime::FromRefs + 'static,
    for<'store> Reaction::Marker<'store>: 'store + runtime::Trigger<State>,
    State: runtime::ReactorData,
{
    type Output = runtime::BoxedReactionFn;
    fn defer(self) -> impl FnOnce(&EnclavePartsMap) -> Self::Output + 'static {
        move |_| runtime::BoxedReactionFn::from(self)
    }
}

impl DeferedBuild for runtime::reaction::TimerFn {
    type Output = runtime::BoxedReactionFn;
    fn defer(self) -> impl FnOnce(&EnclavePartsMap) -> Self::Output + 'static {
        move |_| runtime::BoxedReactionFn::from(self)
    }
}

/// Aliasing maps from Builder keys to runtime keys
#[derive(Default)]
pub struct BuilderAliases {
    pub reactor_aliases: SecondaryMap<BuilderReactorKey, runtime::ReactorKey>,
    pub reaction_aliases: SecondaryMap<BuilderReactionKey, runtime::ReactionKey>,
    pub action_aliases: SecondaryMap<BuilderActionKey, runtime::ActionKey>,
    pub port_aliases: SecondaryMap<BuilderPortKey, runtime::PortKey>,
}

/// A map of partitions: each Reactor is mapped to one Enclave Reactor.
pub type PartitionMap = SecondaryMap<BuilderReactorKey, BuilderReactorKey>;

impl EnvBuilder {
    /// Process the connections and reduce them to a set of port bindings.
    pub(super) fn build_connections(
        &mut self,
        partition_map: &mut PartitionMap,
    ) -> Result<PortBindings, BuilderError> {
        let mut port_bindings = PortBindings::default();
        for connection in std::mem::take(&mut self.connection_builders).iter_mut() {
            connection.build(self, partition_map, &mut port_bindings)?;
        }
        Ok(port_bindings)
    }

    /// Build the enclave partitioning map.
    pub(crate) fn build_partition_map(&self) -> PartitionMap {
        let graph = self.build_reactor_graph();

        let mut partitions = vec![];
        let mut node_stack = vec![self.reactor_builders.keys().next().unwrap()];

        petgraph::visit::depth_first_search(&graph, self.reactor_builders.keys(), |event| {
            match event {
                petgraph::visit::DfsEvent::Discover(key, _) => {
                    if self.reactor_builders[key].is_enclave {
                        node_stack.push(key);
                    }
                    partitions.push((key, *node_stack.last().unwrap()));
                }
                petgraph::visit::DfsEvent::Finish(key, _) => {
                    if self.reactor_builders[key].is_enclave {
                        node_stack.pop();
                    }
                }
                _ => {}
            }
        });

        partitions.into_iter().collect()
    }

    fn build_runtime_reactors(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
    ) -> Result<(), BuilderError> {
        for (builder_reactor_key, reactor) in self.reactor_builders.drain() {
            let partition_key = partition_map[builder_reactor_key];
            let partition = &mut partitions[partition_key];
            let bank_info = reactor.bank_info.clone();
            let runtime_reactor_key = partition
                .enclave
                .insert_reactor(reactor.into_runtime(), bank_info);
            partition
                .aliases
                .reactor_aliases
                .insert(builder_reactor_key, runtime_reactor_key);
        }
        Ok(())
    }

    fn build_runtime_actions(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
    ) -> Result<(), BuilderError> {
        let mut new_reactions = Vec::new();

        for (builder_action_key, action) in &self.action_builders {
            let partition_key = partition_map[action.parent_reactor_key().unwrap()];
            let partition = &mut partitions[partition_key];

            match action.r#type() {
                ActionType::Timer(spec) => {
                    let runtime_action_key = partition.enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    partition
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, runtime_action_key);

                    if spec.period.is_some() {
                        // Periodic timers need a reset reaction
                        new_reactions.push((
                            format!("{}_reset", action.name()),
                            runtime::reaction::TimerFn(spec.period),
                            action.reactor_key(),
                            builder_action_key,
                        ));
                    }
                }

                ActionType::Shutdown => {
                    let runtime_action_key = partition.enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    partition
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, runtime_action_key);
                }

                ActionType::Standard {
                    is_logical: _,
                    min_delay: _,
                    build_fn,
                } => {
                    let runtime_action_key = partition
                        .enclave
                        .insert_action(|key| build_fn(action.name(), key));

                    partition
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, runtime_action_key);
                }
            }
        }

        // Now create the reset reactions for periodic timers, since we can now get &mut self.
        for (name, reaction_fn, reactor_key, action_key) in new_reactions {
            let _ = self
                .add_reaction(&name, reactor_key, reaction_fn.defer())
                .with_action(action_key, 0, TriggerMode::TriggersAndEffects)?
                .finish()?;
        }

        Ok(())
    }

    fn build_runtime_ports(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
        port_bindings: &PortBindings,
    ) -> Result<(), BuilderError> {
        let port_groups = self
            .port_builders
            .keys()
            .map(|port_key| (port_key, port_bindings.follow_port_inward(port_key)))
            .sorted_by(|a, b| a.1.cmp(&b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let port = &self.port_builders[inward_port_key];
            let partition_key = partition_map[port.parent_reactor_key().unwrap()];
            let partition = &mut partitions[partition_key];

            let runtime_port_key = partition
                .enclave
                .insert_port(|key| port.build_runtime_port(key));

            partition
                .aliases
                .port_aliases
                .insert(inward_port_key, runtime_port_key);

            partition
                .aliases
                .port_aliases
                .extend(group.map(|(port_key, _inward_key)| (port_key, runtime_port_key)));
        }
        Ok(())
    }

    fn build_runtime_reactions(
        &mut self,
        partition_map: &PartitionMap,
        partitions: &mut SecondaryMap<BuilderReactorKey, EnclaveParts>,
        reaction_levels: &SecondaryMap<BuilderReactionKey, runtime::Level>,
    ) -> Result<(), BuilderError> {
        for (builder_reaction_key, reaction) in self.reaction_builders.drain() {
            let reaction_body = (reaction.reaction_fn)(partitions);

            let partition_key = partition_map[reaction.reactor_key];
            let partition = &mut partitions[partition_key];
            let runtime_reactor_key = partition.aliases.reactor_aliases[reaction.reactor_key];

            let use_ports = reaction
                .port_relations
                .iter()
                .filter_map(|(port_key, tm)| tm.is_uses().then_some(port_key))
                .map(|builder_port_key| partition.aliases.port_aliases[builder_port_key]);

            let effect_ports = reaction
                .port_relations
                .iter()
                .filter_map(|(port_key, tm)| tm.is_effects().then_some(port_key))
                .map(|builder_port_key| partition.aliases.port_aliases[builder_port_key]);

            let actions = reaction
                .action_relations
                .iter()
                .filter_map(|(action_key, tm)| {
                    (tm.is_effects() || tm.is_uses()).then_some(action_key)
                })
                .map(|builder_action_key| partition.aliases.action_aliases[builder_action_key]);

            let runtime_reaction_key = partition.enclave.insert_reaction(
                runtime::Reaction::new(&reaction.name, reaction_body, None),
                runtime_reactor_key,
                use_ports,
                effect_ports,
                actions,
            );

            let level_reaction = (reaction_levels[builder_reaction_key], runtime_reaction_key);

            for (builder_port_key, tm) in &reaction.port_relations {
                let port_key = partition.aliases.port_aliases[builder_port_key];
                if tm.is_triggers() {
                    partition
                        .enclave
                        .insert_port_trigger(port_key, level_reaction);
                }
            }

            for (builder_action_key, tm) in &reaction.action_relations {
                let action_key = partition.aliases.action_aliases[builder_action_key];
                if tm.is_triggers() {
                    partition
                        .enclave
                        .insert_action_trigger(action_key, level_reaction);

                    match self.action_builders[builder_action_key].r#type() {
                        ActionType::Timer(timer_spec) => {
                            partition
                                .enclave
                                .insert_startup_reaction(level_reaction, timer_spec.offset);
                        }
                        ActionType::Shutdown => {
                            partition.enclave.insert_shutdown_reaction(level_reaction);
                        }
                        _ => {}
                    }
                }
            }

            partition
                .aliases
                .reaction_aliases
                .insert(builder_reaction_key, runtime_reaction_key);
        }
        Ok(())
    }

    /// Convert the [`EnvBuilder`] into a  [`tinymap::TinySecondaryMap`] of [`EnclaveParts`], one for each partition.
    pub fn into_runtime_parts(mut self) -> Result<Vec<EnclaveParts>, BuilderError> {
        let mut partition_map = self.build_partition_map();
        let port_bindings = self.build_connections(&mut partition_map)?;

        dbg!(&port_bindings);

        let mut partitions: SecondaryMap<BuilderReactorKey, EnclaveParts> = partition_map
            .values()
            .unique()
            .map(|reactor_key| (*reactor_key, EnclaveParts::default()))
            .collect();

        self.build_runtime_actions(&partition_map, &mut partitions)?;
        self.build_runtime_ports(&partition_map, &mut partitions, &port_bindings)?;

        self.build_runtime_reactors(&partition_map, &mut partitions)?;

        // must be done last, since building other parts may add new reactions
        let reaction_levels = self.build_runtime_level_map(&port_bindings)?;
        self.build_runtime_reactions(&partition_map, &mut partitions, &reaction_levels)?;

        /*
        //TODO: Figure out if this is needed
        for partition in partitions.values_mut() {
            for (port_key, runtime_port_key) in &partition.aliases.port_aliases {
                let downstream_reactions =
                    collect_transitive_port_triggers(port_key, &self.port_builders, &port_bindings);
                for downstream_reaction in downstream_reactions {
                    let level_reaction = (
                        reaction_levels[downstream_reaction],
                        partition.aliases.reaction_aliases[downstream_reaction],
                    );
                    partition
                        .enclave
                        .insert_port_trigger(*runtime_port_key, level_reaction);
                }
            }
        }
        */

        Ok(partitions.into_iter().map(|(_, parts)| parts).collect())
    }
}
