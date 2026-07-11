//! Builder-visible metadata for static federated enclave topologies.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    runtime, BoundaryKind, BuilderError, BuilderPortKey, BuilderReactorKey, InterPartitionPlan,
    PartitionRootKind,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FederatedEndpointId(String);

impl FederatedEndpointId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederateBuildInfo {
    pub id: String,
    pub reactor: BuilderReactorKey,
    pub reactor_fqn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedEndpoint {
    pub id: FederatedEndpointId,
    pub source_federate: String,
    pub target_federate: String,
    pub source_port: BuilderPortKey,
    pub target_port: BuilderPortKey,
    pub source_port_fqn: String,
    pub target_port_fqn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedEdge {
    pub endpoint: FederatedEndpointId,
    pub source_federate: String,
    pub target_federate: String,
    pub source_federate_reactor: BuilderReactorKey,
    pub target_federate_reactor: BuilderReactorKey,
    pub source_port: BuilderPortKey,
    pub target_port: BuilderPortKey,
    pub delay: Option<runtime::Duration>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FederationPlan {
    pub federates: Vec<FederateBuildInfo>,
    pub edges: Vec<FederatedEdge>,
    pub endpoints: Vec<FederatedEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedRoute {
    pub endpoint: runtime::FederatedEndpointId,
    pub source: boomerang_federated::FederateId,
    pub target: boomerang_federated::FederateId,
}

impl FederationPlan {
    pub fn is_empty(&self) -> bool {
        self.federates.is_empty() && self.edges.is_empty() && self.endpoints.is_empty()
    }

    pub fn from_inter_partition_plan<E>(
        plan: &InterPartitionPlan,
        mut port_fqn: impl FnMut(BuilderPortKey) -> Result<String, E>,
    ) -> Result<Self, E> {
        let mut federation_plan = Self {
            federates: plan
                .partition_roots
                .iter()
                .filter_map(|root| match &root.kind {
                    PartitionRootKind::LocalEnclave => None,
                    PartitionRootKind::Federated { federate } => Some(FederateBuildInfo {
                        id: federate.clone(),
                        reactor: root.reactor,
                        reactor_fqn: root.reactor_fqn.clone(),
                    }),
                })
                .collect(),
            edges: Vec::new(),
            endpoints: Vec::new(),
        };

        for edge in plan.federated_edges() {
            let source_port_fqn = port_fqn(edge.source_port)?;
            let target_port_fqn = port_fqn(edge.target_port)?;
            let BoundaryKind::Federated {
                source_federate,
                target_federate,
            } = &edge.kind
            else {
                unreachable!("federated_edges only yields federated boundary edges");
            };
            let endpoint =
                FederatedEndpointId::new(format!("{}->{}", source_port_fqn, target_port_fqn));

            federation_plan.endpoints.push(FederatedEndpoint {
                id: endpoint.clone(),
                source_federate: source_federate.clone(),
                target_federate: target_federate.clone(),
                source_port: edge.source_port,
                target_port: edge.target_port,
                source_port_fqn,
                target_port_fqn,
            });
            federation_plan.edges.push(FederatedEdge {
                endpoint,
                source_federate: source_federate.clone(),
                target_federate: target_federate.clone(),
                source_federate_reactor: edge.source_partition,
                target_federate_reactor: edge.target_partition,
                source_port: edge.source_port,
                target_port: edge.target_port,
                delay: edge.delay,
            });
        }

        Ok(federation_plan)
    }
}

pub fn federation_topology_from_plan(
    plan: &FederationPlan,
) -> Result<boomerang_federated::FederatedTopology, BuilderError> {
    let federate_ids = checked_federate_id_set(plan)?;
    let mut seen_endpoints = BTreeSet::<FederatedEndpointId>::new();
    let edges = plan
        .edges
        .iter()
        .map(|edge| {
            validate_endpoint_id("topology edge", edge.endpoint.as_str())?;
            validate_edge_federates(
                edge.endpoint.as_str(),
                &edge.source_federate,
                &edge.target_federate,
                &federate_ids,
            )?;
            if !seen_endpoints.insert(edge.endpoint.clone()) {
                return Err(federation_bridge_error(format!(
                    "duplicate topology edge endpoint '{}'",
                    edge.endpoint.as_str()
                )));
            }

            Ok(boomerang_federated::TopologyEdge::new(
                edge.source_federate.clone(),
                edge.target_federate.clone(),
                edge.endpoint.as_str(),
                wire_delay_from_runtime_delay(edge.delay)?,
            ))
        })
        .collect::<Result<Vec<_>, BuilderError>>()?;

    Ok(boomerang_federated::FederatedTopology::with_edges(
        plan.federates
            .iter()
            .map(|federate| boomerang_federated::FederateId::new(federate.id.clone())),
        edges,
    ))
}

pub fn federated_routes_from_plan(
    plan: &FederationPlan,
) -> Result<Vec<FederatedRoute>, BuilderError> {
    let federate_ids = checked_federate_id_set(plan)?;
    let mut edge_by_endpoint = BTreeMap::<FederatedEndpointId, (String, String)>::new();

    for edge in &plan.edges {
        validate_endpoint_id("route edge", edge.endpoint.as_str())?;
        validate_edge_federates(
            edge.endpoint.as_str(),
            &edge.source_federate,
            &edge.target_federate,
            &federate_ids,
        )?;
        if edge_by_endpoint
            .insert(
                edge.endpoint.clone(),
                (edge.source_federate.clone(), edge.target_federate.clone()),
            )
            .is_some()
        {
            return Err(federation_bridge_error(format!(
                "duplicate route edge endpoint '{}'",
                edge.endpoint.as_str()
            )));
        }
    }

    let mut endpoint_ids = BTreeSet::<FederatedEndpointId>::new();
    let mut routes = Vec::with_capacity(plan.endpoints.len());
    for endpoint in &plan.endpoints {
        validate_endpoint_id("route endpoint", endpoint.id.as_str())?;
        validate_edge_federates(
            endpoint.id.as_str(),
            &endpoint.source_federate,
            &endpoint.target_federate,
            &federate_ids,
        )?;

        let Some((edge_source, edge_target)) = edge_by_endpoint.get(&endpoint.id) else {
            return Err(federation_bridge_error(format!(
                "route endpoint '{}' has no matching federated edge",
                endpoint.id.as_str()
            )));
        };
        if edge_source != &endpoint.source_federate || edge_target != &endpoint.target_federate {
            return Err(federation_bridge_error(format!(
                "route endpoint '{}' maps {} -> {}, but edge maps {} -> {}",
                endpoint.id.as_str(),
                endpoint.source_federate,
                endpoint.target_federate,
                edge_source,
                edge_target
            )));
        }

        if !endpoint_ids.insert(endpoint.id.clone()) {
            return Err(federation_bridge_error(format!(
                "duplicate route endpoint '{}'",
                endpoint.id.as_str()
            )));
        }

        routes.push(FederatedRoute {
            endpoint: runtime::FederatedEndpointId::new(endpoint.id.as_str()),
            source: boomerang_federated::FederateId::new(endpoint.source_federate.clone()),
            target: boomerang_federated::FederateId::new(endpoint.target_federate.clone()),
        });
    }

    for edge_endpoint in edge_by_endpoint.keys() {
        if !endpoint_ids.contains(edge_endpoint) {
            return Err(federation_bridge_error(format!(
                "route edge '{}' has no matching endpoint metadata",
                edge_endpoint.as_str()
            )));
        }
    }

    Ok(routes)
}

/// Execute a static federation in memory using the real RTI session and federate clients.
///
/// This is an explicit federated execution path. It does not replace
/// [`runtime::execute_enclaves`], which remains local-only.
pub fn execute_federation_in_memory(
    parts: crate::BuilderRuntimeParts,
    config: runtime::Config,
) -> Result<tinymap::TinySecondaryMap<runtime::EnclaveKey, runtime::Env>, BuilderError> {
    let runtime_parts = static_federation_runtime_parts(parts)?;
    boomerang_federated::static_runner::execute_federation_in_memory(runtime_parts, config)
        .map_err(BuilderError::from)
}

/// Execute a static federation over TCP using the real RTI session and federate clients.
///
/// This is a single-process runner that connects each federate scheduler to a runner-owned TCP
/// listener. It does not replace [`execute_federation_in_memory`] or
/// [`runtime::execute_enclaves`].
pub fn execute_federation_over_tcp(
    parts: crate::BuilderRuntimeParts,
    config: runtime::Config,
    tcp: boomerang_federated::TcpStaticFederationConfig,
) -> Result<tinymap::TinySecondaryMap<runtime::EnclaveKey, runtime::Env>, BuilderError> {
    let runtime_parts = static_federation_runtime_parts(parts)?;
    boomerang_federated::execute_federation_over_tcp(runtime_parts, config, tcp)
        .map_err(BuilderError::from)
}

fn static_federation_runtime_parts(
    parts: crate::BuilderRuntimeParts,
) -> Result<boomerang_federated::StaticFederationRuntimeParts, BuilderError> {
    validate_static_runner_plan(&parts)?;

    let topology = federation_topology_from_plan(&parts.federation_plan)?;
    let routes = federated_routes_from_plan(&parts.federation_plan)?
        .into_iter()
        .map(|route| {
            boomerang_federated::FederateClientRoute::new(
                route.endpoint,
                route.source,
                route.target,
            )
        })
        .collect();
    let (federate_enclaves, _federate_by_enclave) = federate_enclave_maps(&parts)?;

    Ok(boomerang_federated::StaticFederationRuntimeParts {
        topology,
        routes,
        federate_enclaves,
        enclaves: parts.enclaves,
        outbound_sink: parts.federated_outbound_sink,
        faults: parts.federated_faults,
        inbound_endpoints: parts.federated_inbound_endpoints,
    })
}

fn validate_static_runner_plan(parts: &crate::BuilderRuntimeParts) -> Result<(), BuilderError> {
    if parts.federation_plan.federates.is_empty()
        || parts.federation_plan.edges.is_empty()
        || parts.federation_plan.endpoints.is_empty()
    {
        return Err(BuilderError::UnsupportedFederationTopology {
            what: "static federation runner requires a non-empty federation plan with at least one cross-federate endpoint".into(),
        });
    }

    let mut zero_delay_graph = petgraph::prelude::DiGraphMap::<BuilderReactorKey, ()>::new();
    for edge in parts.inter_partition_plan.federated_edges() {
        if edge.physical {
            return Err(BuilderError::UnsupportedFederationTopology {
                what: "cross-federate physical connections are reserved for a later milestone"
                    .into(),
            });
        }

        let has_positive_delay = edge
            .delay
            .is_some_and(|delay| delay > runtime::Duration::ZERO);
        if !has_positive_delay {
            zero_delay_graph.add_edge(edge.source_partition, edge.target_partition, ());
        }
    }

    if petgraph::algo::toposort(&zero_delay_graph, None).is_err() {
        return Err(BuilderError::UnsupportedFederationTopology {
            what: "distributed zero-delay cycle is unsupported in the static federation runner"
                .into(),
        });
    }

    Ok(())
}

fn federate_enclave_maps(
    parts: &crate::BuilderRuntimeParts,
) -> Result<
    (
        BTreeMap<boomerang_federated::FederateId, runtime::EnclaveKey>,
        tinymap::TinySecondaryMap<runtime::EnclaveKey, boomerang_federated::FederateId>,
    ),
    BuilderError,
> {
    let mut federate_enclaves = BTreeMap::new();
    let mut federate_by_enclave = tinymap::TinySecondaryMap::new();

    for federate in &parts.federation_plan.federates {
        let enclave_key = *parts
            .aliases
            .enclave_aliases
            .get(federate.reactor)
            .ok_or_else(|| {
                federation_bridge_error(format!(
                    "federate '{}' has no runtime enclave alias",
                    federate.id
                ))
            })?;
        let federate_id = boomerang_federated::FederateId::new(federate.id.clone());

        if let Some(previous) = federate_by_enclave.get(enclave_key) {
            return Err(federation_bridge_error(format!(
                "ambiguous enclave-to-federate mapping: enclave {enclave_key:?} maps to both '{previous}' and '{federate_id}'"
            )));
        }
        if federate_enclaves
            .insert(federate_id.clone(), enclave_key)
            .is_some()
        {
            return Err(federation_bridge_error(format!(
                "duplicate federate id '{federate_id}'"
            )));
        }
        federate_by_enclave.insert(enclave_key, federate_id);
    }

    Ok((federate_enclaves, federate_by_enclave))
}

fn checked_federate_id_set(plan: &FederationPlan) -> Result<BTreeSet<String>, BuilderError> {
    let mut federate_ids = BTreeSet::new();
    for federate in &plan.federates {
        if federate.id.trim().is_empty() {
            return Err(federation_bridge_error(format!(
                "federate '{}' has an empty protocol id",
                federate.reactor_fqn
            )));
        }
        if !federate_ids.insert(federate.id.clone()) {
            return Err(federation_bridge_error(format!(
                "duplicate federate id '{}'",
                federate.id
            )));
        }
    }

    Ok(federate_ids)
}

fn validate_endpoint_id(context: &str, endpoint: &str) -> Result<(), BuilderError> {
    if endpoint.trim().is_empty() {
        return Err(federation_bridge_error(format!(
            "{context} has an empty endpoint id"
        )));
    }

    Ok(())
}

fn validate_edge_federates(
    endpoint: &str,
    source: &str,
    target: &str,
    federate_ids: &BTreeSet<String>,
) -> Result<(), BuilderError> {
    if !federate_ids.contains(source) {
        return Err(federation_bridge_error(format!(
            "endpoint '{endpoint}' references unknown source federate '{source}'"
        )));
    }
    if !federate_ids.contains(target) {
        return Err(federation_bridge_error(format!(
            "endpoint '{endpoint}' references unknown target federate '{target}'"
        )));
    }

    Ok(())
}

fn federation_bridge_error(what: impl Into<String>) -> BuilderError {
    BuilderError::FederationBridgeError { what: what.into() }
}

fn wire_delay_from_runtime_delay(
    delay: Option<runtime::Duration>,
) -> Result<boomerang_federated::WireDelay, BuilderError> {
    delay
        .map(boomerang_federated::WireDelay::try_from)
        .transpose()
        .map_err(BuilderError::from)
        .map(|delay| delay.unwrap_or(boomerang_federated::WireDelay::ZERO))
}
