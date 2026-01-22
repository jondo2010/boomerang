use boomerang_runtime::{self as runtime};
use itertools::Itertools;
use slotmap::SecondaryMap;

use crate::{
    connection::PortBindings, ActionType, BuilderActionKey, BuilderError, BuilderPortKey,
    BuilderReactionKey, BuilderReactorKey, ParentReactorBuilder, PartialReactionBuilder,
    TimerActionKey,
};

use super::EnvBuilder;

/// A trait used to defer the building of until the enclave parts are available.
pub trait DeferedBuild {
    type Output;

    fn defer(self) -> impl FnOnce(&BuilderRuntimeParts) -> Self::Output + 'static;
}

impl DeferedBuild for runtime::reaction::TimerFn {
    type Output = runtime::BoxedReactionFn;
    fn defer(self) -> impl FnOnce(&BuilderRuntimeParts) -> Self::Output + 'static {
        move |_| runtime::BoxedReactionFn::from(self)
    }
}

/// Aliasing maps from Builder keys to runtime keys
#[derive(Default)]
pub struct BuilderAliases {
    pub enclave_aliases: SecondaryMap<BuilderReactorKey, runtime::EnclaveKey>,
    pub reactor_aliases:
        SecondaryMap<BuilderReactorKey, (runtime::EnclaveKey, runtime::ReactorKey)>,
    pub reaction_aliases:
        SecondaryMap<BuilderReactionKey, (runtime::EnclaveKey, runtime::ReactionKey)>,
    pub action_aliases: SecondaryMap<BuilderActionKey, (runtime::EnclaveKey, runtime::ActionKey)>,
    pub port_aliases: SecondaryMap<BuilderPortKey, (runtime::EnclaveKey, runtime::PortKey)>,
}

/// A map of partitions: each Reactor is mapped to one Enclave Reactor.
pub type PartitionMap = SecondaryMap<BuilderReactorKey, BuilderReactorKey>;

#[derive(Default)]
pub struct BuilderRuntimeParts {
    /// The runtime Enclaves
    pub enclaves: tinymap::TinyMap<runtime::EnclaveKey, runtime::Enclave>,
    /// Aliases from builder keys to runtime keys
    pub aliases: BuilderAliases,
    #[cfg(feature = "replay")]
    /// The action replayers for each enclave
    pub replayers: runtime::replay::ReplayersMap,
}

impl BuilderRuntimeParts {
    /// Create a new `BuilderRuntimeParts` from a `PartitionMap`.
    fn new(
        partition_map: &PartitionMap,
        enclave_deps: Vec<EnclaveDep>,
        physical_event_q_size: usize,
    ) -> Self {
        let mut enclaves = tinymap::TinyMap::new();
        let mut aliases = BuilderAliases::default();
        // Create all the unique enclaves
        for reactor_key in partition_map.values().unique() {
            let enclave_key =
                enclaves.insert(runtime::Enclave::with_event_q_size(physical_event_q_size));
            aliases.enclave_aliases.insert(*reactor_key, enclave_key);
        }
        // Add any missing aliases
        for (reactor_key, reactor_enclave_key) in partition_map {
            if !aliases.enclave_aliases.contains_key(reactor_key) {
                let enclave_key = aliases.enclave_aliases[*reactor_enclave_key];
                aliases.enclave_aliases.insert(reactor_key, enclave_key);
            }
        }
        // Add any enclave dependencies
        for EnclaveDep {
            upstream,
            downstream,
            delay,
        } in enclave_deps
        {
            let upstream_enclave_key = aliases.enclave_aliases[upstream];
            let downstream_enclave_key = aliases.enclave_aliases[downstream];

            runtime::crosslink_enclaves(
                &mut enclaves,
                upstream_enclave_key,
                downstream_enclave_key,
                delay,
            );
        }
        #[cfg(feature = "replay")]
        {
            // Pre-fill the replayers map with empty maps
            let replayers = enclaves
                .keys()
                .map(|enclave_key| {
                    let replayers = tinymap::TinySecondaryMap::new();
                    (enclave_key, replayers)
                })
                .collect();

            Self {
                enclaves,
                aliases,
                replayers,
            }
        }
        #[cfg(not(feature = "replay"))]
        {
            // No replayers, just return the enclaves and aliases
            Self { enclaves, aliases }
        }
    }
}

pub struct EnclaveDep {
    pub upstream: BuilderReactorKey,
    pub downstream: BuilderReactorKey,
    pub delay: Option<runtime::Duration>,
}

impl EnvBuilder {
    /// Process the connections and reduce them to a set of port bindings.
    pub(super) fn build_connections(
        &mut self,
        partition_map: &mut PartitionMap,
    ) -> Result<(PortBindings, Vec<EnclaveDep>), BuilderError> {
        let mut port_bindings = PortBindings::default();
        let mut enclave_deps = vec![];
        for connection in std::mem::take(&mut self.connection_builders).iter_mut() {
            connection.build(self, partition_map, &mut port_bindings, &mut enclave_deps)?;
        }
        Ok((port_bindings, enclave_deps))
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
        builder_parts: &mut BuilderRuntimeParts,
    ) -> Result<(), BuilderError> {
        let reactor_fqns: SecondaryMap<BuilderReactorKey, String> = self
            .reactor_builders
            .keys()
            .map(|reactor_key| {
                self.fqn_for(reactor_key, false)
                    .map(|fqn| (reactor_key, fqn.to_string()))
            })
            .collect::<Result<_, _>>()?;

        for (builder_reactor_key, reactor) in self.reactor_builders.drain() {
            let partition_key = partition_map[builder_reactor_key];
            let enclave_key = builder_parts.aliases.enclave_aliases[partition_key];
            let enclave = &mut builder_parts.enclaves[enclave_key];
            let bank_info = reactor.bank_info.clone();
            let reactor_fqn = &reactor_fqns[builder_reactor_key];
            let runtime_reactor_key =
                enclave.insert_reactor(reactor.into_runtime(reactor_fqn), bank_info);
            builder_parts
                .aliases
                .reactor_aliases
                .insert(builder_reactor_key, (enclave_key, runtime_reactor_key));
        }
        Ok(())
    }

    /// Build the runtime actions.
    ///
    /// Timer and Shutdown actions that are not used by any reactions are culled.
    fn build_runtime_actions(
        &mut self,
        partition_map: &PartitionMap,
        builder_parts: &mut BuilderRuntimeParts,
    ) -> Result<(), BuilderError> {
        let mut new_reactions = Vec::new();

        for (builder_action_key, action) in &self.action_builders {
            let partition_key = partition_map[action.parent_reactor_key().unwrap()];
            let enclave_key = builder_parts.aliases.enclave_aliases[partition_key];
            let enclave = &mut builder_parts.enclaves[enclave_key];

            let action_referenced = self
                .reaction_builders
                .iter()
                .any(|(_, reaction)| reaction.action_relations.contains_key(builder_action_key));

            match action.r#type() {
                ActionType::Timer(spec) if action_referenced => {
                    let runtime_action_key = enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    builder_parts
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, (enclave_key, runtime_action_key));

                    if spec.period.is_some() {
                        // Periodic timers need a reset reaction
                        new_reactions.push((
                            format!("{}_reset", action.name()),
                            runtime::reaction::TimerFn(spec.period),
                            action.reactor_key(),
                            TimerActionKey::from(builder_action_key),
                        ));
                    }
                }

                ActionType::Shutdown if action_referenced => {
                    let runtime_action_key = enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    builder_parts
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, (enclave_key, runtime_action_key));
                }

                ActionType::Standard {
                    is_logical: _,
                    min_delay: _,
                    build_fn,
                } => {
                    let runtime_action_key =
                        enclave.insert_action(|key| build_fn(action.name(), key));

                    builder_parts
                        .aliases
                        .action_aliases
                        .insert(builder_action_key, (enclave_key, runtime_action_key));
                }

                _ => {
                    tracing::info!(
                        "Action {} is unused, won't build",
                        self.fqn_for(builder_action_key, false).unwrap()
                    );
                }
            }
        }

        // Now create the reset reactions for periodic timers, since we can now get &mut self.
        for (name, reaction_fn, reactor_key, action_key) in new_reactions {
            let _ = PartialReactionBuilder::<()>::new(Some(&name), reactor_key, self)
                .with_trigger(action_key)
                .with_defered_reaction_fn(reaction_fn.defer())
                .finish()?;
        }

        Ok(())
    }

    fn build_runtime_ports(
        &mut self,
        partition_map: &PartitionMap,
        builder_parts: &mut BuilderRuntimeParts,
        port_bindings: &PortBindings,
    ) -> Result<(), BuilderError> {
        let port_groups = self
            .port_builders
            .keys()
            .map(|port_key| (port_key, port_bindings.follow_port_inward(port_key)))
            .sorted_by(|key_a, key_b| key_a.1.cmp(&key_b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let port = &self.port_builders[inward_port_key];
            let partition_key = partition_map[port.parent_reactor_key().unwrap()];
            let enclave_key = builder_parts.aliases.enclave_aliases[partition_key];
            let enclave = &mut builder_parts.enclaves[enclave_key];

            let runtime_port_key = enclave.insert_port(|key| port.build_runtime_port(key));

            builder_parts
                .aliases
                .port_aliases
                .insert(inward_port_key, (enclave_key, runtime_port_key));

            builder_parts.aliases.port_aliases.extend(
                group.map(|(port_key, _inward_key)| (port_key, (enclave_key, runtime_port_key))),
            );
        }
        Ok(())
    }

    fn build_runtime_reactions(
        &mut self,
        partition_map: &PartitionMap,
        builder_parts: &mut BuilderRuntimeParts,
        reaction_levels: &SecondaryMap<BuilderReactionKey, runtime::Level>,
    ) -> Result<(), BuilderError> {
        for (builder_reaction_key, reaction) in self.reaction_builders.drain() {
            let reaction_body = (reaction.reaction_fn)(builder_parts);

            let reaction_name = reaction.name.clone().unwrap_or_else(|| {
                let reaction_u64 = slotmap::Key::data(&builder_reaction_key).as_ffi();
                format!("reaction{reaction_u64}")
            });

            let partition_key = partition_map[reaction.reactor_key];
            let enclave_key = builder_parts.aliases.enclave_aliases[partition_key];
            let enclave = &mut builder_parts.enclaves[enclave_key];
            let runtime_reactor_key = {
                let (alias_enclave_key, reactor_key) =
                    builder_parts.aliases.reactor_aliases[reaction.reactor_key];
                assert_eq!(enclave_key, alias_enclave_key, "Crosscheck");
                reactor_key
            };

            let use_port_keys: Vec<_> = reaction
                .port_order
                .iter()
                .filter_map(|port_key| {
                    reaction
                        .port_relations
                        .get(*port_key)
                        .and_then(|tm| tm.is_uses().then_some(*port_key))
                })
                .collect();

            let effect_port_keys: Vec<_> = reaction
                .port_order
                .iter()
                .filter_map(|port_key| {
                    reaction
                        .port_relations
                        .get(*port_key)
                        .and_then(|tm| tm.is_effects().then_some(*port_key))
                })
                .collect();

            let action_keys: Vec<_> = reaction
                .action_order
                .iter()
                .filter_map(|action_key| {
                    reaction
                        .action_relations
                        .get(*action_key)
                        .and_then(|tm| (tm.is_effects() || tm.is_uses()).then_some(*action_key))
                })
                .collect();

            if tracing::enabled!(tracing::Level::TRACE) {
                tracing::trace!(
                    reaction = %reaction_name,
                    ?use_port_keys,
                    ?effect_port_keys,
                    ?action_keys,
                    "Assigning reaction dependencies"
                );
            }

            let use_ports = use_port_keys
                .iter()
                .map(|builder_port_key| builder_parts.aliases.port_aliases[*builder_port_key].1);

            let effect_ports = effect_port_keys
                .iter()
                .map(|builder_port_key| builder_parts.aliases.port_aliases[*builder_port_key].1);

            let actions = action_keys
                .iter()
                .map(|builder_action_key| builder_parts.aliases.action_aliases[*builder_action_key].1);

            let runtime_reaction_key = enclave.insert_reaction(
                runtime::Reaction::new(&reaction_name, reaction_body, None),
                runtime_reactor_key,
                use_ports,
                effect_ports,
                actions,
            );

            let level_reaction = (reaction_levels[builder_reaction_key], runtime_reaction_key);

            for (builder_port_key, tm) in &reaction.port_relations {
                let port_key = builder_parts.aliases.port_aliases[builder_port_key].1;
                if tm.is_triggers() {
                    enclave.insert_port_trigger(port_key, level_reaction);
                }
            }

            for (builder_action_key, tm) in &reaction.action_relations {
                let action_key = builder_parts.aliases.action_aliases[builder_action_key].1;
                if tm.is_triggers() {
                    enclave.insert_action_trigger(action_key, level_reaction);

                    match self.action_builders[builder_action_key].r#type() {
                        ActionType::Timer(timer_spec) => {
                            let tag = runtime::Tag::new(timer_spec.offset.unwrap_or_default(), 0);
                            enclave.insert_startup_action(action_key, tag);
                        }
                        ActionType::Shutdown => {
                            enclave.insert_shutdown_action(action_key);
                        }
                        _ => {}
                    }
                }
            }

            builder_parts
                .aliases
                .reaction_aliases
                .insert(builder_reaction_key, (enclave_key, runtime_reaction_key));
        }
        Ok(())
    }

    /// Build the runtime replayers.
    #[cfg(feature = "replay")]
    fn build_runtime_replayers(
        &mut self,
        builder_parts: &mut BuilderRuntimeParts,
    ) -> Result<(), BuilderError> {
        for (builder_action_key, replayer_builder) in self.replay_builders.drain() {
            let replayer = (replayer_builder)(builder_parts);
            let (enclave_key, action_key) =
                builder_parts.aliases.action_aliases[builder_action_key];
            builder_parts.replayers[enclave_key].insert(action_key, replayer);
        }
        Ok(())
    }

    /// Convert the [`EnvBuilder`] into parts suitable for execution by the runtime.
    pub fn into_runtime_parts(
        mut self,
        config: &runtime::Config,
    ) -> Result<BuilderRuntimeParts, BuilderError> {
        let mut partition_map = self.build_partition_map();
        let (port_bindings, enclave_deps) = self.build_connections(&mut partition_map)?;
        let mut builder_parts =
            BuilderRuntimeParts::new(&partition_map, enclave_deps, config.physical_event_q_size);

        self.build_runtime_actions(&partition_map, &mut builder_parts)?;
        self.build_runtime_ports(&partition_map, &mut builder_parts, &port_bindings)?;

        // this must be done before build_runtime_reactors, since that drains self.reaction_builders
        let reaction_levels = self.build_runtime_level_map(&port_bindings)?;

        self.build_runtime_reactors(&partition_map, &mut builder_parts)?;

        // must be done last, since building other parts may add new reactions
        self.build_runtime_reactions(&partition_map, &mut builder_parts, &reaction_levels)?;

        #[cfg(feature = "replay")]
        self.build_runtime_replayers(&mut builder_parts)?;

        Ok(builder_parts)
    }
}
