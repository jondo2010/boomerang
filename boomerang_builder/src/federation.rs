//! Builder-visible metadata for static federated enclave topologies.

use crate::{
    runtime, BoundaryKind, BuilderPortKey, BuilderReactorKey, InterPartitionPlan, PartitionRootKind,
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
