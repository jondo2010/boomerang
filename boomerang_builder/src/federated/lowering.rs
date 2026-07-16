//! Projection from assembly partition boundaries to protocol topology artifacts.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{runtime, AssemblyError, AssemblyPortKey, AssemblyReactorKey, PartitionAnalysis};

pub(crate) type FederatedBoundaryIndex =
    HashMap<(AssemblyPortKey, AssemblyPortKey), FederatedBoundary>;

#[derive(Debug, Clone)]
pub(crate) struct FederatedBoundary {
    pub(crate) endpoint: boomerang_federated::EndpointId,
    pub(crate) target_federate: boomerang_federated::FederateId,
    pub(crate) target_partition: AssemblyReactorKey,
}

pub(crate) struct FederationLowering {
    pub(crate) topology: boomerang_federated::FederatedTopology,
    pub(crate) federates: BTreeMap<boomerang_federated::FederateId, FederatePlan>,
    pub(crate) boundaries: FederatedBoundaryIndex,
}

pub(crate) struct FederatePlan {
    pub(crate) enclave_roots: Vec<AssemblyReactorKey>,
}

impl FederationLowering {
    pub(crate) fn from_partition_analysis(
        analysis: &PartitionAnalysis,
        mut port_fqn: impl FnMut(AssemblyPortKey) -> Result<String, AssemblyError>,
    ) -> Result<Self, AssemblyError> {
        let mut federates = BTreeMap::<boomerang_federated::FederateId, FederatePlan>::new();
        let mut federate_order = Vec::new();
        for (reactor, federate) in &analysis.federates {
            if federate.trim().is_empty() {
                return Err(federation_bridge_error(format!(
                    "federate partition {reactor:?} has an empty protocol id"
                )));
            }
            let federate_id = boomerang_federated::FederateId::new(federate.clone());
            if !federates.contains_key(&federate_id) {
                federate_order.push(federate_id.clone());
            }
            federates
                .entry(federate_id)
                .or_insert_with(|| FederatePlan {
                    enclave_roots: Vec::new(),
                })
                .enclave_roots
                .push(reactor);
        }

        let mut seen_endpoints = BTreeSet::new();
        let mut boundaries = HashMap::new();
        let mut topology_edges = Vec::new();
        for (edge, source_federate, target_federate) in analysis.federated_boundaries() {
            let source = boomerang_federated::FederateId::new(source_federate);
            let target = boomerang_federated::FederateId::new(target_federate);
            if !federates.contains_key(&source) {
                return Err(federation_bridge_error(format!(
                    "federated boundary references unknown source federate '{source}'"
                )));
            }
            if !federates.contains_key(&target) {
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
            topology: boomerang_federated::FederatedTopology::with_edges(
                federate_order,
                topology_edges,
            ),
            federates,
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
