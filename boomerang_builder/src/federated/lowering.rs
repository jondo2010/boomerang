//! Projection from assembly partition boundaries to protocol topology artifacts.

use std::collections::{BTreeMap, HashMap};

use crate::{runtime, AssemblyError, AssemblyPortKey, AssemblyReactorKey, PartitionAnalysis};

use super::{analyze_federation_graph, FederationEndpoint};

pub(crate) type FederatedBoundaryIndex =
    HashMap<(AssemblyPortKey, AssemblyPortKey), FederatedBoundary>;

#[derive(Debug, Clone)]
/// Lowered metadata for one cross-federate connection.
pub(crate) struct FederatedBoundary {
    /// Stable protocol endpoint for the connection.
    pub(crate) endpoint: boomerang_federated::EndpointId,
    /// Federate receiving the connection payload.
    pub(crate) target_federate: boomerang_federated::FederateId,
    /// Assembly partition receiving the connection payload.
    pub(crate) target_partition: AssemblyReactorKey,
}

/// Transient artifacts produced by lowering assembly federation boundaries.
pub(crate) struct FederationLoweringArtifacts {
    /// Final immutable graph derived from federated partition boundaries.
    pub(crate) rti_graph: boomerang_federated::RtiGraph,
    /// Prebuilt local client mailboxes and routes from the same endpoint records.
    pub(crate) connections: boomerang_federated::FederatedRuntimeConnections,
    /// Assembly enclave roots grouped by their owning protocol federate.
    pub(crate) federate_reactors:
        BTreeMap<boomerang_federated::FederateId, Vec<AssemblyReactorKey>>,
    /// Connection metadata consumed while lowering connection specifications.
    pub(crate) boundaries: FederatedBoundaryIndex,
}

/// Lower partition analysis into protocol topology and connection artifacts.
pub(crate) fn lower_federation(
    analysis: &PartitionAnalysis,
    mut port_fqn: impl FnMut(AssemblyPortKey) -> Result<String, AssemblyError>,
) -> Result<FederationLoweringArtifacts, AssemblyError> {
    let mut federate_reactors = BTreeMap::new();
    for (reactor, federate) in &analysis.federates {
        if federate.trim().is_empty() {
            return Err(federation_bridge_error(format!(
                "federate partition {reactor:?} has an empty protocol id"
            )));
        }
        let federate_id = boomerang_federated::FederateId::new(federate.clone());
        federate_reactors
            .entry(federate_id)
            .or_insert_with(Vec::new)
            .push(reactor);
    }

    let mut boundaries = HashMap::new();
    let mut has_duplicate_boundary = false;
    let mut endpoints = Vec::new();
    for (edge, source_federate, target_federate) in analysis.federated_boundaries() {
        let source = boomerang_federated::FederateId::new(source_federate);
        let target = boomerang_federated::FederateId::new(target_federate);
        if !federate_reactors.contains_key(&source) {
            return Err(federation_bridge_error(format!(
                "federated boundary references unknown source federate '{source}'"
            )));
        }
        if !federate_reactors.contains_key(&target) {
            return Err(federation_bridge_error(format!(
                "federated boundary references unknown target federate '{target}'"
            )));
        }

        let endpoint = boomerang_federated::EndpointId::new(format!(
            "{}->{}",
            port_fqn(edge.source_port)?,
            port_fqn(edge.target_port)?,
        ));
        if endpoint.as_str().trim().is_empty() {
            return Err(federation_bridge_error(
                "federated boundary has an empty endpoint id",
            ));
        }
        endpoints.push(FederationEndpoint {
            source,
            target: target.clone(),
            endpoint: endpoint.clone(),
            delay: wire_delay_from_runtime_delay(edge.delay)?,
        });
        has_duplicate_boundary |= boundaries
            .insert(
                (edge.source_port, edge.target_port),
                FederatedBoundary {
                    endpoint,
                    target_federate: target,
                    target_partition: edge.target_partition,
                },
            )
            .is_some();
    }

    let graph = analyze_federation_graph(federate_reactors.keys().cloned(), endpoints)?;
    if has_duplicate_boundary {
        return Err(federation_bridge_error(
            "duplicate federated boundary for the same source and target ports",
        ));
    }

    let connections = boomerang_federated::FederatedRuntimeConnections::new(
        graph.federates.iter().cloned(),
        graph.endpoints.iter().map(|edge| {
            boomerang_federated::FederateClientRoute::new(
                edge.endpoint.clone(),
                edge.source.clone(),
                edge.target.clone(),
            )
        }),
    )
    .map_err(|error| federation_bridge_error(error.to_string()))?;
    let rti_graph = graph.to_rti_graph();

    Ok(FederationLoweringArtifacts {
        rti_graph,
        connections,
        federate_reactors,
        boundaries,
    })
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
