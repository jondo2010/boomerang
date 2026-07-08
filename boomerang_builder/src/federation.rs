//! Builder-visible metadata for static federated enclave topologies.

use crate::{runtime, BuilderPortKey, BuilderReactorKey};

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
}
