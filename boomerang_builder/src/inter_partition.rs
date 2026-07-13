//! Assembly-visible metadata for connections that cross runtime partitions.

use crate::{runtime, AssemblyPortKey, AssemblyReactorKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionRootKind {
    LocalEnclave,
    Federated { federate: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionRoot {
    pub reactor: AssemblyReactorKey,
    pub reactor_fqn: String,
    pub kind: PartitionRootKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryKind {
    LocalEnclave,
    Federated {
        source_federate: String,
        target_federate: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterPartitionEdge {
    pub kind: BoundaryKind,
    pub source_partition: AssemblyReactorKey,
    pub target_partition: AssemblyReactorKey,
    pub source_port: AssemblyPortKey,
    pub target_port: AssemblyPortKey,
    pub delay: Option<runtime::Duration>,
    pub physical: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InterPartitionPlan {
    pub partition_roots: Vec<PartitionRoot>,
    pub edges: Vec<InterPartitionEdge>,
}

impl InterPartitionPlan {
    pub fn local_enclave_edges(&self) -> impl Iterator<Item = &InterPartitionEdge> {
        self.edges
            .iter()
            .filter(|edge| matches!(edge.kind, BoundaryKind::LocalEnclave))
    }

    pub fn federated_edges(&self) -> impl Iterator<Item = &InterPartitionEdge> {
        self.edges
            .iter()
            .filter(|edge| matches!(edge.kind, BoundaryKind::Federated { .. }))
    }
}
