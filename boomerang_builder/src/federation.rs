//! Assembly-visible metadata for static federated enclave topologies.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    runtime, AssemblyError, AssemblyPortKey, AssemblyReactorKey, BoundaryKind, InterPartitionPlan,
    PartitionRootKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederateBuildInfo {
    /// Stable protocol identity assigned to the federate.
    pub id: String,
    /// Assembly reactor that forms the federate's partition root.
    pub reactor: AssemblyReactorKey,
    /// Fully qualified assembly name of the federate reactor.
    pub reactor_fqn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedEndpoint {
    /// Stable protocol identity of the cross-federate endpoint.
    pub id: boomerang_federated::EndpointId,
    /// Protocol identity of the sending federate.
    pub source_federate: String,
    /// Protocol identity of the receiving federate.
    pub target_federate: String,
    /// Assembly port that emits values for this endpoint.
    pub source_port: AssemblyPortKey,
    /// Assembly port that receives values from this endpoint.
    pub target_port: AssemblyPortKey,
    /// Fully qualified assembly name of the source port.
    pub source_port_fqn: String,
    /// Fully qualified assembly name of the target port.
    pub target_port_fqn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedEdge {
    /// Stable protocol identity shared with the corresponding endpoint.
    pub endpoint: boomerang_federated::EndpointId,
    /// Protocol identity of the sending federate.
    pub source_federate: String,
    /// Protocol identity of the receiving federate.
    pub target_federate: String,
    /// Assembly reactor that forms the source federate partition root.
    pub source_federate_reactor: AssemblyReactorKey,
    /// Assembly reactor that forms the target federate partition root.
    pub target_federate_reactor: AssemblyReactorKey,
    /// Assembly port at the source of the cross-federate connection.
    pub source_port: AssemblyPortKey,
    /// Assembly port at the target of the cross-federate connection.
    pub target_port: AssemblyPortKey,
    /// Logical delay applied while crossing the federation boundary.
    pub delay: Option<runtime::Duration>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FederationPlan {
    /// Federates participating in the statically lowered application.
    pub federates: Vec<FederateBuildInfo>,
    /// Directed logical dependencies between federates.
    pub edges: Vec<FederatedEdge>,
    /// Typed payload routes associated with the federated edges.
    pub endpoints: Vec<FederatedEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedRoute {
    /// Stable protocol identity of the routed endpoint.
    pub endpoint: boomerang_federated::EndpointId,
    /// Protocol identity of the sending federate.
    pub source: boomerang_federated::FederateId,
    /// Protocol identity of the receiving federate.
    pub target: boomerang_federated::FederateId,
}

impl FederationPlan {
    pub fn is_empty(&self) -> bool {
        self.federates.is_empty() && self.edges.is_empty() && self.endpoints.is_empty()
    }

    pub fn from_inter_partition_plan<E>(
        plan: &InterPartitionPlan,
        mut port_fqn: impl FnMut(AssemblyPortKey) -> Result<String, E>,
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
            let endpoint = boomerang_federated::EndpointId::new(format!(
                "{}->{}",
                source_port_fqn, target_port_fqn
            ));

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
) -> Result<boomerang_federated::FederatedTopology, AssemblyError> {
    let federate_ids = checked_federate_id_set(plan)?;
    let mut seen_endpoints = BTreeSet::<boomerang_federated::EndpointId>::new();
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
                edge.endpoint.clone(),
                wire_delay_from_runtime_delay(edge.delay)?,
            ))
        })
        .collect::<Result<Vec<_>, AssemblyError>>()?;

    Ok(boomerang_federated::FederatedTopology::with_edges(
        plan.federates
            .iter()
            .map(|federate| boomerang_federated::FederateId::new(federate.id.clone())),
        edges,
    ))
}

pub fn federated_routes_from_plan(
    plan: &FederationPlan,
) -> Result<Vec<FederatedRoute>, AssemblyError> {
    let federate_ids = checked_federate_id_set(plan)?;
    let mut edge_by_endpoint = BTreeMap::<boomerang_federated::EndpointId, (String, String)>::new();

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

    let mut endpoint_ids = BTreeSet::<boomerang_federated::EndpointId>::new();
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
            endpoint: endpoint.id.clone(),
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

fn checked_federate_id_set(plan: &FederationPlan) -> Result<BTreeSet<String>, AssemblyError> {
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

fn validate_endpoint_id(context: &str, endpoint: &str) -> Result<(), AssemblyError> {
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
) -> Result<(), AssemblyError> {
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

fn federation_bridge_error(what: impl Into<String>) -> AssemblyError {
    AssemblyError::FederationBridgeError { what: what.into() }
}

fn wire_delay_from_runtime_delay(
    delay: Option<runtime::Duration>,
) -> Result<boomerang_federated::WireDelay, AssemblyError> {
    delay
        .map(boomerang_federated::WireDelay::try_from)
        .transpose()
        .map_err(AssemblyError::from)
        .map(|delay| delay.unwrap_or(boomerang_federated::WireDelay::ZERO))
}
