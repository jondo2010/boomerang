//! Runtime graph lowering for [`Assembly`].
//!
//! This module performs the build-time pass that materializes runtime enclaves and completes
//! derived [`boomerang_runtime::ReactionGraph`] data. The scheduler should not need to reconstruct
//! these static indexes during execution.

use boomerang_runtime::{self as runtime};
use core::range::Range;
use itertools::Itertools;
use slotmap::SecondaryMap;
#[cfg(feature = "federated")]
use std::collections::BTreeMap;

use crate::{
    connection::PortBindings, ActionType, AssemblyActionKey, AssemblyError, AssemblyModeKey,
    AssemblyPortKey, AssemblyReactionKey, AssemblyReactorKey, BoundaryKind, InterPartitionEdge,
    InterPartitionPlan, ParentReactorSpec, PartitionRoot, PartitionRootKind, ReactionDeclaration,
    TimerActionKey,
};
#[cfg(feature = "federated")]
use crate::{federated_routes_from_plan, federation_topology_from_plan, FederationPlan};

use super::Assembly;

/// A trait used to defer runtime object creation until the lowered runtime data is available.
pub trait DeferredRuntimeFactory {
    type Output;

    fn defer(self) -> impl FnOnce(&RuntimeAssembly) -> Self::Output + 'static;
}

impl DeferredRuntimeFactory for runtime::reaction::TimerFn {
    type Output = runtime::BoxedReactionFn;
    fn defer(self) -> impl FnOnce(&RuntimeAssembly) -> Self::Output + 'static {
        move |_| runtime::BoxedReactionFn::from(self)
    }
}

/// Aliasing maps from assembly keys to runtime keys.
#[derive(Default)]
pub struct RuntimeAliases {
    pub enclave_aliases: SecondaryMap<AssemblyReactorKey, runtime::EnclaveKey>,
    pub reactor_aliases:
        SecondaryMap<AssemblyReactorKey, (runtime::EnclaveKey, runtime::ReactorKey)>,
    pub reactor_scope_modes: SecondaryMap<AssemblyReactorKey, Option<AssemblyModeKey>>,
    pub reaction_aliases:
        SecondaryMap<AssemblyReactionKey, (runtime::EnclaveKey, runtime::ReactionKey)>,
    pub mode_aliases: SecondaryMap<AssemblyModeKey, (runtime::EnclaveKey, runtime::ModeKey)>,
    pub action_aliases: SecondaryMap<AssemblyActionKey, (runtime::EnclaveKey, runtime::ActionKey)>,
    pub port_aliases: SecondaryMap<AssemblyPortKey, (runtime::EnclaveKey, runtime::PortKey)>,
}

/// A map of partitions: each Reactor is mapped to one Enclave Reactor.
pub type PartitionMap = SecondaryMap<AssemblyReactorKey, AssemblyReactorKey>;

/// Fully lowered static-federation data owned by a [`RuntimeAssembly`].
#[cfg(feature = "federated")]
pub struct LoweredFederation {
    /// Assembly-visible federate, edge, and endpoint metadata.
    pub plan: FederationPlan,
    /// Validated RTI topology and its precomputed coordination indexes.
    pub(crate) topology: boomerang_federated::CompiledTopology,
    /// Prebuilt protocol mailboxes, routes, inbound handlers, and fault state.
    pub(crate) connections: boomerang_federated::FederatedRuntimeConnections,
}

#[derive(Default)]
pub struct RuntimeAssembly {
    /// Executable runtime enclaves keyed by their lowered enclave identities.
    pub enclaves: tinymap::TinyMap<runtime::EnclaveKey, runtime::Enclave>,
    /// Aliases from assembly keys to runtime keys.
    pub aliases: RuntimeAliases,
    /// Assembly-owned metadata for logical edges that cross runtime partitions.
    pub inter_partition_plan: InterPartitionPlan,
    #[cfg(feature = "federated")]
    /// Fully lowered static-federation data, or `None` for a local-only assembly.
    pub federation: Option<LoweredFederation>,
    #[cfg(feature = "replay")]
    /// Action replayers partitioned by their target runtime enclave.
    pub replayers: runtime::replay::ReplayersMap,
}

impl RuntimeAssembly {
    /// Create a new `RuntimeAssembly` from a `PartitionMap`.
    fn new(
        partition_map: &PartitionMap,
        inter_partition_plan: InterPartitionPlan,
        enclave_deps: Vec<EnclaveDep>,
        physical_event_q_size: usize,
        #[cfg(feature = "federated")] federation: Option<LoweredFederation>,
    ) -> Self {
        let mut enclaves = tinymap::TinyMap::new();
        let mut aliases = RuntimeAliases::default();
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
                inter_partition_plan,
                #[cfg(feature = "federated")]
                federation,
                replayers,
            }
        }
        #[cfg(not(feature = "replay"))]
        {
            // No replayers, just return the enclaves and aliases
            Self {
                enclaves,
                aliases,
                inter_partition_plan,
                #[cfg(feature = "federated")]
                federation,
            }
        }
    }
}

pub struct EnclaveDep {
    pub upstream: AssemblyReactorKey,
    pub downstream: AssemblyReactorKey,
    pub delay: Option<runtime::Duration>,
}

fn enclave_deps_from_inter_partition_plan(plan: &InterPartitionPlan) -> Vec<EnclaveDep> {
    plan.local_enclave_edges()
        .map(|edge| EnclaveDep {
            upstream: edge.source_partition,
            downstream: edge.target_partition,
            delay: edge.delay,
        })
        .collect()
}

fn push_range<T>(target: &mut Vec<T>, values: impl IntoIterator<Item = T>) -> Range<usize> {
    let start = target.len();
    target.extend(values);
    Range {
        start,
        end: target.len(),
    }
}

fn scope_is_descendant_or_self(
    graph: &runtime::ReactionGraph,
    mut scope: runtime::ScopeKey,
    ancestor: runtime::ScopeKey,
) -> bool {
    loop {
        if scope == ancestor {
            return true;
        }

        let Some(parent) = graph.scopes[scope].parent else {
            return false;
        };
        scope = parent;
    }
}

fn build_modal_schedule_index(graph: &runtime::ReactionGraph) -> runtime::ModalScheduleIndex {
    let mut index = runtime::ModalScheduleIndex::default();

    for scope in graph.scopes.keys() {
        let descendant_range = push_range(
            &mut index.scope_descendants,
            graph
                .scopes
                .keys()
                .filter(|&candidate| scope_is_descendant_or_self(graph, candidate, scope)),
        );
        index
            .scope_descendant_ranges
            .insert(scope, descendant_range);

        let logical_action_range = push_range(
            &mut index.scope_logical_actions,
            graph
                .action_scopes
                .iter()
                .filter_map(|(action_key, &action_scope)| {
                    (graph.action_is_logical[action_key]
                        && scope_is_descendant_or_self(graph, action_scope, scope))
                    .then_some(action_key)
                }),
        );
        index
            .scope_logical_action_ranges
            .insert(scope, logical_action_range);

        let timer_startup_range = push_range(
            &mut index.scope_timer_startups,
            graph
                .timer_startup_actions
                .iter()
                .copied()
                .filter(|(action_key, _)| {
                    let action_scope = graph.action_scopes[*action_key];
                    scope_is_descendant_or_self(graph, action_scope, scope)
                }),
        );
        index
            .scope_timer_startup_ranges
            .insert(scope, timer_startup_range);

        let reset_reaction_range = push_range(
            &mut index.scope_reset_reactions,
            graph
                .reset_reactions
                .iter()
                .filter(|(reaction_scope, reactions)| {
                    !reactions.is_empty()
                        && scope_is_descendant_or_self(graph, *reaction_scope, scope)
                })
                .flat_map(|(_, reactions)| reactions.iter().copied()),
        );
        index
            .scope_reset_reaction_ranges
            .insert(scope, reset_reaction_range);

        let startup_reaction_range = push_range(
            &mut index.scope_startup_reactions,
            graph.startup_reactions[scope].iter().copied(),
        );
        index
            .scope_startup_reaction_ranges
            .insert(scope, startup_reaction_range);
    }

    index.all_shutdown_reactions.extend(
        graph
            .shutdown_reactions_by_scope
            .values()
            .flat_map(|reactions| reactions.iter().copied()),
    );

    for reaction in &index.all_shutdown_reactions {
        if !index.all_shutdown_actions_unique.contains(&reaction.action) {
            index.all_shutdown_actions_unique.push(reaction.action);
        }
    }

    index
}

impl Assembly {
    fn build_inter_partition_plan(
        &self,
        partition_map: &PartitionMap,
    ) -> Result<InterPartitionPlan, AssemblyError> {
        let mut plan = InterPartitionPlan::default();

        #[cfg(feature = "federated")]
        let federate_id_by_partition = {
            let mut federate_id_by_partition = SecondaryMap::<AssemblyReactorKey, String>::new();
            let mut seen_ids = BTreeMap::<String, AssemblyReactorKey>::new();

            for (reactor_key, reactor) in &self.reactor_specs {
                let Some(spec) = reactor.federate_spec() else {
                    continue;
                };

                if spec.id.trim().is_empty() {
                    return Err(AssemblyError::UnsupportedFederationTopology {
                        what: format!(
                            "federate reactor '{}' must have a non-empty id",
                            self.fqn_for(reactor_key, false)?
                        ),
                    });
                }

                if spec.transient {
                    return Err(AssemblyError::UnsupportedFederationTopology {
                        what: format!(
                            "transient federate '{}' is reserved for a later milestone",
                            spec.id
                        ),
                    });
                }

                if partition_map[reactor_key] != reactor_key {
                    return Err(AssemblyError::UnsupportedFederationTopology {
                        what: format!(
                            "federate '{}' must be an enclave root in this milestone",
                            spec.id
                        ),
                    });
                }

                if let Some(previous) = seen_ids.insert(spec.id.clone(), reactor_key) {
                    return Err(AssemblyError::UnsupportedFederationTopology {
                        what: format!(
                            "duplicate federate id '{}' for '{}' and '{}'",
                            spec.id,
                            self.fqn_for(previous, false)?,
                            self.fqn_for(reactor_key, false)?,
                        ),
                    });
                }

                federate_id_by_partition.insert(reactor_key, spec.id.clone());
            }

            federate_id_by_partition
        };

        for partition in partition_map.values().copied().unique() {
            #[cfg(feature = "federated")]
            let kind = federate_id_by_partition
                .get(partition)
                .map(|federate| PartitionRootKind::Federated {
                    federate: federate.clone(),
                })
                .unwrap_or(PartitionRootKind::LocalEnclave);

            #[cfg(not(feature = "federated"))]
            let kind = PartitionRootKind::LocalEnclave;

            plan.partition_roots.push(PartitionRoot {
                reactor: partition,
                reactor_fqn: self.fqn_for(partition, false)?.to_string(),
                kind,
            });
        }

        for connection in &self.connection_specs {
            let source_port_key = connection.source_key();
            let target_port_key = connection.target_key();
            let source_port = &self.port_specs[source_port_key];
            let target_port = &self.port_specs[target_port_key];
            let source_reactor_key = source_port.parent_reactor_key().ok_or_else(|| {
                AssemblyError::InternalError("source port has no parent reactor".to_owned())
            })?;
            let target_reactor_key = target_port.parent_reactor_key().ok_or_else(|| {
                AssemblyError::InternalError("target port has no parent reactor".to_owned())
            })?;
            let source_partition = partition_map[source_reactor_key];
            let target_partition = partition_map[target_reactor_key];

            if source_partition == target_partition {
                continue;
            }

            #[cfg(feature = "federated")]
            let kind = {
                let source_federate = federate_id_by_partition.get(source_partition);
                let target_federate = federate_id_by_partition.get(target_partition);

                match (source_federate, target_federate) {
                    (None, None) => BoundaryKind::LocalEnclave,
                    (Some(source_federate), Some(target_federate)) => BoundaryKind::Federated {
                        source_federate: source_federate.clone(),
                        target_federate: target_federate.clone(),
                    },
                    _ => {
                        return Err(AssemblyError::UnsupportedFederationTopology {
                            what: format!(
                                "connection '{}' -> '{}' crosses a federated boundary, but both enclave roots are not federates",
                                self.fqn_for(source_port_key, false)?,
                                self.fqn_for(target_port_key, false)?,
                            ),
                        });
                    }
                }
            };

            #[cfg(not(feature = "federated"))]
            let kind = BoundaryKind::LocalEnclave;

            if matches!(kind, BoundaryKind::Federated { .. }) && connection.physical() {
                return Err(AssemblyError::UnsupportedFederationTopology {
                    what: format!(
                        "cross-federate physical connection '{}' -> '{}' is reserved for a later milestone",
                        self.fqn_for(source_port_key, false)?,
                        self.fqn_for(target_port_key, false)?,
                    ),
                });
            }

            plan.edges.push(InterPartitionEdge {
                kind,
                source_partition,
                target_partition,
                source_port: source_port_key,
                target_port: target_port_key,
                delay: connection.after(),
                physical: connection.physical(),
            });
        }

        #[cfg(feature = "federated")]
        self.validate_federation_zero_delay_cycles(&plan)?;

        Ok(plan)
    }

    #[cfg(feature = "federated")]
    fn validate_federation_zero_delay_cycles(
        &self,
        plan: &InterPartitionPlan,
    ) -> Result<(), AssemblyError> {
        let mut graph = petgraph::prelude::DiGraphMap::<AssemblyReactorKey, ()>::new();

        for root in &plan.partition_roots {
            if matches!(root.kind, PartitionRootKind::Federated { .. }) {
                graph.add_node(root.reactor);
            }
        }

        for edge in plan.federated_edges() {
            let has_positive_delay = edge
                .delay
                .is_some_and(|delay| delay > runtime::Duration::ZERO);
            if !has_positive_delay {
                graph.add_edge(edge.source_partition, edge.target_partition, ());
            }
        }

        if let Err(cycle) = petgraph::algo::toposort(&graph, None) {
            let cycle = super::util::find_minimal_cycle(&graph, cycle.node_id());
            let cycle = cycle
                .into_iter()
                .map(|reactor_key| {
                    self.reactor_specs[reactor_key]
                        .federate_spec()
                        .map(|spec| spec.id.clone())
                        .unwrap_or_else(|| format!("{reactor_key:?}"))
                })
                .join(" -> ");
            return Err(AssemblyError::UnsupportedFederationTopology {
                what: format!(
                    "distributed zero-delay cycle is unsupported in the static MVP: {cycle}"
                ),
            });
        }

        Ok(())
    }

    /// Process the connections and reduce them to a set of port bindings.
    pub(super) fn build_connections(
        &mut self,
        partition_map: &mut PartitionMap,
    ) -> Result<PortBindings, AssemblyError> {
        let mut port_bindings = PortBindings::default();
        for connection in std::mem::take(&mut self.connection_specs).iter_mut() {
            connection.build(self, partition_map, &mut port_bindings)?;
        }
        Ok(port_bindings)
    }

    /// Build the enclave partitioning map.
    pub(crate) fn build_partition_map(&self) -> PartitionMap {
        let graph = self.build_reactor_graph();

        let mut partitions = vec![];
        let mut node_stack = vec![self.reactor_specs.keys().next().unwrap()];

        petgraph::visit::depth_first_search(
            &graph,
            self.reactor_specs.keys(),
            |event| match event {
                petgraph::visit::DfsEvent::Discover(key, _) => {
                    if self.reactor_specs[key].placement().starts_enclave() {
                        node_stack.push(key);
                    }
                    partitions.push((key, *node_stack.last().unwrap()));
                }
                petgraph::visit::DfsEvent::Finish(key, _)
                    if self.reactor_specs[key].placement().starts_enclave() =>
                {
                    node_stack.pop();
                }
                _ => {}
            },
        );

        partitions.into_iter().collect()
    }

    fn build_runtime_reactors(
        &mut self,
        partition_map: &PartitionMap,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        let reactor_fqns: SecondaryMap<AssemblyReactorKey, String> = self
            .reactor_specs
            .keys()
            .map(|reactor_key| {
                self.fqn_for(reactor_key, false)
                    .map(|fqn| (reactor_key, fqn.to_string()))
            })
            .collect::<Result<_, _>>()?;

        for (assembly_reactor_key, reactor) in self.reactor_specs.drain() {
            let partition_key = partition_map[assembly_reactor_key];
            let enclave_key = runtime_assembly.aliases.enclave_aliases[partition_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let bank_info = reactor.bank_info.clone();
            let scope_mode = reactor.scope_mode;
            let reactor_fqn = &reactor_fqns[assembly_reactor_key];
            let runtime_reactor_key =
                enclave.insert_reactor(reactor.into_runtime(reactor_fqn), bank_info);
            runtime_assembly
                .aliases
                .reactor_scope_modes
                .insert(assembly_reactor_key, scope_mode);
            runtime_assembly
                .aliases
                .reactor_aliases
                .insert(assembly_reactor_key, (enclave_key, runtime_reactor_key));
        }
        Ok(())
    }

    fn assign_runtime_reactor_scope_parents(
        &mut self,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        for (assembly_reactor_key, scope_mode) in &runtime_assembly.aliases.reactor_scope_modes {
            let Some(scope_mode) = scope_mode else {
                continue;
            };
            let (enclave_key, runtime_reactor_key) =
                runtime_assembly.aliases.reactor_aliases[assembly_reactor_key];
            let (mode_enclave_key, runtime_mode_key) =
                runtime_assembly.aliases.mode_aliases[*scope_mode];
            assert_eq!(enclave_key, mode_enclave_key, "Crosscheck");
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let parent_scope = enclave.mode_scope(runtime_mode_key);
            enclave.set_reactor_scope_parent(runtime_reactor_key, parent_scope);
        }
        Ok(())
    }

    fn build_runtime_modes(
        &mut self,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        for (assembly_mode_key, mode) in self.mode_specs.drain() {
            let (enclave_key, reactor_key) =
                runtime_assembly.aliases.reactor_aliases[mode.reactor_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let runtime_mode_key =
                enclave.insert_mode(reactor_key, &mode.name, mode.kind.is_initial());
            runtime_assembly
                .aliases
                .mode_aliases
                .insert(assembly_mode_key, (enclave_key, runtime_mode_key));
        }
        Ok(())
    }

    /// Build the runtime actions.
    ///
    /// Timer and Shutdown actions that are not used by any reactions are culled.
    fn build_runtime_actions(
        &mut self,
        partition_map: &PartitionMap,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        let mut new_reactions = Vec::new();

        for (assembly_action_key, action) in &self.action_specs {
            let partition_key = partition_map[action.parent_reactor_key().unwrap()];
            let enclave_key = runtime_assembly.aliases.enclave_aliases[partition_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];

            let action_referenced = self
                .reaction_specs
                .iter()
                .any(|(_, reaction)| reaction.action_relations.contains_key(assembly_action_key));

            match action.r#type() {
                ActionType::Timer(spec) if action_referenced => {
                    let runtime_action_key = enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    runtime_assembly
                        .aliases
                        .action_aliases
                        .insert(assembly_action_key, (enclave_key, runtime_action_key));

                    if spec.period.is_some() {
                        // Periodic timers need a reset reaction
                        new_reactions.push((
                            format!("{}_reset", action.name()),
                            runtime::reaction::TimerFn(spec.period),
                            action.reactor_key(),
                            TimerActionKey::from(assembly_action_key),
                        ));
                    }
                }

                ActionType::Shutdown if action_referenced => {
                    let runtime_action_key = enclave.insert_action(|key| {
                        runtime::Action::<()>::new(action.name(), key, None, true).boxed()
                    });
                    runtime_assembly
                        .aliases
                        .action_aliases
                        .insert(assembly_action_key, (enclave_key, runtime_action_key));
                }

                ActionType::Standard {
                    is_logical: _,
                    min_delay: _,
                    build_fn,
                } => {
                    let runtime_action_key =
                        enclave.insert_action(|key| build_fn(action.name(), key));

                    runtime_assembly
                        .aliases
                        .action_aliases
                        .insert(assembly_action_key, (enclave_key, runtime_action_key));
                }

                _ => {
                    tracing::info!(
                        "Action {} is unused, won't build",
                        self.fqn_for(assembly_action_key, false).unwrap()
                    );
                }
            }
        }

        // Now create the reset reactions for periodic timers, since we can now get &mut self.
        for (name, reaction_fn, reactor_key, action_key) in new_reactions {
            let scope_mode = self.action_specs[AssemblyActionKey::from(action_key)].scope_mode();
            let mut reaction = ReactionDeclaration::<()>::new(Some(&name), reactor_key, self)
                .with_trigger(action_key);
            if let Some(scope_mode) = scope_mode {
                reaction = reaction.in_mode_scope(scope_mode);
            }
            let _ = reaction
                .with_deferred_reaction_factory(reaction_fn.defer())
                .finish()?;
        }

        Ok(())
    }

    #[cfg(feature = "federated")]
    fn build_federated_inbound_endpoints(
        &mut self,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        if self.federated_inbound_endpoint_factories.is_empty() {
            return Ok(());
        }
        let mut connections = std::mem::take(
            &mut runtime_assembly
                .federation
                .as_mut()
                .ok_or_else(|| AssemblyError::InconsistentAssemblyState {
                    what: "federated inbound endpoints exist without a lowered federation".into(),
                })?
                .connections,
        );
        for endpoint_factory in self.federated_inbound_endpoint_factories.drain(..) {
            endpoint_factory(runtime_assembly, &mut connections)?;
        }
        runtime_assembly
            .federation
            .as_mut()
            .expect("lowered federation was checked before endpoint construction")
            .connections = connections;
        Ok(())
    }

    fn assign_runtime_action_and_port_scopes(
        &mut self,
        runtime_assembly: &mut RuntimeAssembly,
        port_bindings: &PortBindings,
    ) -> Result<(), AssemblyError> {
        for (assembly_action_key, action) in &self.action_specs {
            let (enclave_key, action_key) = match runtime_assembly
                .aliases
                .action_aliases
                .get(assembly_action_key)
            {
                Some(alias) => *alias,
                None => continue,
            };
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let scope = if let Some(mode_key) = action.scope_mode() {
                let (mode_enclave_key, runtime_mode_key) =
                    runtime_assembly.aliases.mode_aliases[mode_key];
                assert_eq!(enclave_key, mode_enclave_key, "Crosscheck");
                enclave.mode_scope(runtime_mode_key)
            } else {
                let (reactor_enclave_key, runtime_reactor_key) =
                    runtime_assembly.aliases.reactor_aliases[action.reactor_key()];
                assert_eq!(enclave_key, reactor_enclave_key, "Crosscheck");
                enclave.root_scope(runtime_reactor_key)
            };
            enclave.insert_action_scope(action_key, scope);
        }

        for (assembly_port_key, _port) in &self.port_specs {
            let (enclave_key, port_key) =
                match runtime_assembly.aliases.port_aliases.get(assembly_port_key) {
                    Some(alias) => *alias,
                    None => continue,
                };
            let inward_port_key = port_bindings.follow_port_inward(assembly_port_key);
            let port = &self.port_specs[inward_port_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let (reactor_enclave_key, runtime_reactor_key) =
                runtime_assembly.aliases.reactor_aliases[port.get_reactor_key()];
            assert_eq!(enclave_key, reactor_enclave_key, "Crosscheck");
            let scope = enclave.root_scope(runtime_reactor_key);
            enclave.insert_port_scope(port_key, scope);
        }

        Ok(())
    }

    fn build_runtime_ports(
        &mut self,
        partition_map: &PartitionMap,
        runtime_assembly: &mut RuntimeAssembly,
        port_bindings: &PortBindings,
    ) -> Result<(), AssemblyError> {
        let port_groups = self
            .port_specs
            .keys()
            .map(|port_key| (port_key, port_bindings.follow_port_inward(port_key)))
            .sorted_by(|key_a, key_b| key_a.1.cmp(&key_b.1))
            .chunk_by(|(_port_key, inward_key)| *inward_key);

        for (inward_port_key, group) in port_groups.into_iter() {
            let port = &self.port_specs[inward_port_key];
            let partition_key = partition_map[port.parent_reactor_key().unwrap()];
            let enclave_key = runtime_assembly.aliases.enclave_aliases[partition_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];

            let runtime_port_key = enclave.insert_port(|key| port.build_runtime_port(key));

            runtime_assembly
                .aliases
                .port_aliases
                .insert(inward_port_key, (enclave_key, runtime_port_key));

            runtime_assembly.aliases.port_aliases.extend(
                group.map(|(port_key, _inward_key)| (port_key, (enclave_key, runtime_port_key))),
            );
        }
        Ok(())
    }

    fn build_runtime_reactions(
        &mut self,
        partition_map: &PartitionMap,
        runtime_assembly: &mut RuntimeAssembly,
        reaction_levels: &SecondaryMap<AssemblyReactionKey, runtime::Level>,
    ) -> Result<(), AssemblyError> {
        for (assembly_reaction_key, reaction) in self.reaction_specs.drain() {
            let reaction_body = (reaction.reaction_fn)(runtime_assembly);

            let reaction_name = reaction.name.clone().unwrap_or_else(|| {
                let reaction_u64 = slotmap::Key::data(&assembly_reaction_key).as_ffi();
                format!("reaction{reaction_u64}")
            });

            let partition_key = partition_map[reaction.reactor_key];
            let enclave_key = runtime_assembly.aliases.enclave_aliases[partition_key];
            let enclave = &mut runtime_assembly.enclaves[enclave_key];
            let runtime_reactor_key = {
                let (alias_enclave_key, reactor_key) =
                    runtime_assembly.aliases.reactor_aliases[reaction.reactor_key];
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

            let use_ports = use_port_keys.iter().map(|assembly_port_key| {
                runtime_assembly.aliases.port_aliases[*assembly_port_key].1
            });

            let effect_ports = effect_port_keys.iter().map(|assembly_port_key| {
                runtime_assembly.aliases.port_aliases[*assembly_port_key].1
            });

            let actions = action_keys.iter().map(|assembly_action_key| {
                runtime_assembly.aliases.action_aliases[*assembly_action_key].1
            });

            let mode_filter = reaction.enabled_modes.as_ref().map(|modes| {
                let runtime_modes = modes
                    .iter()
                    .map(|mode_key| runtime_assembly.aliases.mode_aliases[*mode_key].1)
                    .collect();
                runtime::ModeFilter::new(runtime_modes)
            });

            let reaction_scope = if let Some(mode_key) = reaction.scope_mode {
                let (mode_enclave_key, runtime_mode_key) =
                    runtime_assembly.aliases.mode_aliases[mode_key];
                assert_eq!(enclave_key, mode_enclave_key, "Crosscheck");
                enclave.mode_scope(runtime_mode_key)
            } else {
                enclave.root_scope(runtime_reactor_key)
            };

            let runtime_reaction_key = enclave.insert_reaction(
                runtime::Reaction::new(&reaction_name, reaction_body, None),
                runtime_reactor_key,
                use_ports,
                effect_ports,
                actions,
                reaction_scope,
                mode_filter,
            );

            let level_reaction = (reaction_levels[assembly_reaction_key], runtime_reaction_key);

            if reaction.reset_trigger {
                enclave.insert_reset_trigger(reaction_scope, level_reaction);
            }

            for (assembly_port_key, tm) in &reaction.port_relations {
                let port_key = runtime_assembly.aliases.port_aliases[assembly_port_key].1;
                if tm.is_triggers() {
                    enclave.insert_port_trigger(port_key, level_reaction);
                }
            }

            for (assembly_action_key, tm) in &reaction.action_relations {
                let action_key = runtime_assembly.aliases.action_aliases[assembly_action_key].1;
                if tm.is_triggers() {
                    enclave.insert_action_trigger(action_key, level_reaction);

                    let action_spec = &self.action_specs[assembly_action_key];
                    match action_spec.r#type() {
                        ActionType::Timer(timer_spec) => {
                            let tag = runtime::Tag::new(timer_spec.offset.unwrap_or_default(), 0);
                            enclave.insert_startup_action(action_key, tag);
                            if action_spec.name() == "__startup" {
                                enclave.insert_startup_trigger(
                                    reaction_scope,
                                    action_key,
                                    level_reaction,
                                );
                            } else {
                                enclave.insert_timer_startup_action(action_key, tag);
                            }
                        }
                        ActionType::Shutdown => {
                            enclave.insert_shutdown_action(action_key);
                            enclave.insert_shutdown_trigger(
                                reaction_scope,
                                action_key,
                                level_reaction,
                            );
                        }
                        _ => {}
                    }
                }
            }

            runtime_assembly
                .aliases
                .reaction_aliases
                .insert(assembly_reaction_key, (enclave_key, runtime_reaction_key));
        }
        Ok(())
    }

    /// Build the runtime replayers.
    #[cfg(feature = "replay")]
    fn build_runtime_replayers(
        &mut self,
        runtime_assembly: &mut RuntimeAssembly,
    ) -> Result<(), AssemblyError> {
        for (assembly_action_key, replay_factory) in self.replay_factories.drain() {
            let replayer = (replay_factory)(runtime_assembly);
            let (enclave_key, action_key) =
                runtime_assembly.aliases.action_aliases[assembly_action_key];
            runtime_assembly.replayers[enclave_key].insert(action_key, replayer);
        }
        Ok(())
    }

    /// Convert the [`Assembly`] into parts suitable for execution by the runtime.
    pub fn into_runtime_assembly(
        mut self,
        config: &runtime::Config,
    ) -> Result<RuntimeAssembly, AssemblyError> {
        let mut partition_map = self.build_partition_map();
        let inter_partition_plan = self.build_inter_partition_plan(&partition_map)?;
        #[cfg(feature = "federated")]
        let federation_plan =
            FederationPlan::from_inter_partition_plan(&inter_partition_plan, |port| {
                self.fqn_for(port, false).map(|fqn| fqn.to_string())
            })?;
        #[cfg(feature = "federated")]
        let compiled_federation_topology = if federation_plan.is_empty() {
            None
        } else {
            Some(boomerang_federated::CompiledTopology::new(
                federation_topology_from_plan(&federation_plan)?,
            )?)
        };
        #[cfg(feature = "federated")]
        let federated_connections = boomerang_federated::FederatedRuntimeConnections::new(
            federation_plan
                .federates
                .iter()
                .map(|federate| boomerang_federated::FederateId::new(federate.id.clone())),
            federated_routes_from_plan(&federation_plan)?
                .into_iter()
                .map(|route| {
                    boomerang_federated::FederateClientRoute::new(
                        route.endpoint,
                        route.source,
                        route.target,
                    )
                }),
        )?;
        #[cfg(feature = "federated")]
        let federation = compiled_federation_topology.map(|topology| LoweredFederation {
            plan: federation_plan,
            topology,
            connections: federated_connections,
        });
        let enclave_deps = enclave_deps_from_inter_partition_plan(&inter_partition_plan);
        let port_bindings = self.build_connections(&mut partition_map)?;
        let mut runtime_assembly = RuntimeAssembly::new(
            &partition_map,
            inter_partition_plan,
            enclave_deps,
            config.physical_event_q_size,
            #[cfg(feature = "federated")]
            federation,
        );

        self.build_runtime_actions(&partition_map, &mut runtime_assembly)?;
        #[cfg(feature = "federated")]
        self.build_federated_inbound_endpoints(&mut runtime_assembly)?;
        self.build_runtime_ports(&partition_map, &mut runtime_assembly, &port_bindings)?;

        // this must be done before build_runtime_reactors, since that drains self.reaction_specs
        let reaction_levels = self.build_runtime_level_map(&port_bindings)?;

        self.build_runtime_reactors(&partition_map, &mut runtime_assembly)?;
        self.build_runtime_modes(&mut runtime_assembly)?;
        self.assign_runtime_reactor_scope_parents(&mut runtime_assembly)?;
        self.assign_runtime_action_and_port_scopes(&mut runtime_assembly, &port_bindings)?;

        // must be done last, since building other parts may add new reactions
        self.build_runtime_reactions(&partition_map, &mut runtime_assembly, &reaction_levels)?;

        #[cfg(feature = "replay")]
        self.build_runtime_replayers(&mut runtime_assembly)?;

        for enclave in runtime_assembly.enclaves.values_mut() {
            let modal_schedule_index = build_modal_schedule_index(&enclave.graph);
            enclave.graph.modal_schedule_index = modal_schedule_index;
        }

        Ok(runtime_assembly)
    }
}
