//! Lowering support for static federated enclave topologies.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{runtime, AssemblyError, AssemblyPortKey, AssemblyReactorKey, PartitionAnalysis};

pub(crate) type FederatedBoundaryIndex =
    HashMap<(AssemblyPortKey, AssemblyPortKey), FederatedBoundary>;

/// Protocol identity needed while lowering one federated boundary connection.
#[derive(Debug, Clone)]
pub(crate) struct FederatedBoundary {
    pub(crate) endpoint: boomerang_federated::EndpointId,
    pub(crate) target_federate: boomerang_federated::FederateId,
    pub(crate) target_partition: AssemblyReactorKey,
}

/// Canonical one-pass projection from assembly boundaries to federation artifacts.
pub(crate) struct FederationLowering {
    pub(crate) topology: boomerang_federated::FederatedTopology,
    pub(crate) federate_reactors: BTreeMap<boomerang_federated::FederateId, AssemblyReactorKey>,
    pub(crate) boundaries: FederatedBoundaryIndex,
}

impl FederationLowering {
    pub(crate) fn from_partition_analysis(
        analysis: &PartitionAnalysis,
        mut port_fqn: impl FnMut(AssemblyPortKey) -> Result<String, AssemblyError>,
    ) -> Result<Self, AssemblyError> {
        let mut federates = Vec::new();
        let mut federate_reactors = BTreeMap::new();
        for (reactor, federate) in &analysis.federates {
            if federate.trim().is_empty() {
                return Err(federation_bridge_error(format!(
                    "federate partition {:?} has an empty protocol id",
                    reactor
                )));
            }
            let federate_id = boomerang_federated::FederateId::new(federate.clone());
            if federate_reactors
                .insert(federate_id.clone(), reactor)
                .is_some()
            {
                return Err(federation_bridge_error(format!(
                    "duplicate federate id '{federate_id}'"
                )));
            }
            federates.push(federate_id);
        }

        let mut seen_endpoints = BTreeSet::new();
        let mut boundaries = HashMap::new();
        let mut topology_edges = Vec::new();
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
            if !seen_endpoints.insert(endpoint.clone()) {
                return Err(federation_bridge_error(format!(
                    "duplicate federated boundary endpoint '{endpoint}'"
                )));
            }

            topology_edges.push(boomerang_federated::TopologyEdge::new(
                source,
                target.clone(),
                endpoint.clone(),
                wire_delay_from_runtime_delay(edge.delay)?,
            ));
            if boundaries
                .insert(
                    (edge.source_port, edge.target_port),
                    FederatedBoundary {
                        endpoint,
                        target_federate: target,
                        target_partition: edge.target_partition,
                    },
                )
                .is_some()
            {
                return Err(federation_bridge_error(
                    "duplicate federated boundary for the same source and target ports",
                ));
            }
        }

        Ok(Self {
            topology: boomerang_federated::FederatedTopology::with_edges(federates, topology_edges),
            federate_reactors,
            boundaries,
        })
    }
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
