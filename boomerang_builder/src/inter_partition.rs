//! Transient analysis of connections that cross runtime partitions.

#[cfg(feature = "federated")]
use crate::AssemblyPortKey;
use crate::{runtime, AssemblyReactorKey};
#[cfg(feature = "federated")]
use slotmap::SecondaryMap;

pub(crate) struct PartitionBoundary {
    pub(crate) source_partition: AssemblyReactorKey,
    pub(crate) target_partition: AssemblyReactorKey,
    #[cfg(feature = "federated")]
    pub(crate) source_port: AssemblyPortKey,
    #[cfg(feature = "federated")]
    pub(crate) target_port: AssemblyPortKey,
    pub(crate) delay: Option<runtime::Duration>,
}

#[derive(Default)]
pub(crate) struct PartitionAnalysis {
    #[cfg(feature = "federated")]
    pub(crate) federates: SecondaryMap<AssemblyReactorKey, String>,
    pub(crate) boundaries: Vec<PartitionBoundary>,
}

impl PartitionAnalysis {
    #[cfg(feature = "federated")]
    pub(crate) fn federate_for_partition(&self, partition: AssemblyReactorKey) -> Option<&str> {
        self.federates.get(partition).map(String::as_str)
    }

    pub(crate) fn local_boundaries(&self) -> impl Iterator<Item = &PartitionBoundary> {
        self.boundaries
            .iter()
            .filter(|edge| self.is_local_edge(edge))
    }

    #[cfg(feature = "federated")]
    pub(crate) fn federated_boundaries(
        &self,
    ) -> impl Iterator<Item = (&PartitionBoundary, &str, &str)> {
        self.boundaries.iter().filter_map(|edge| {
            self.federates_for_edge(edge)
                .and_then(|(source, target)| (source != target).then_some((edge, source, target)))
        })
    }

    #[cfg(feature = "federated")]
    fn is_local_edge(&self, edge: &PartitionBoundary) -> bool {
        match self.federates_for_edge(edge) {
            None => true,
            Some((source, target)) => source == target,
        }
    }

    #[cfg(not(feature = "federated"))]
    fn is_local_edge(&self, _edge: &PartitionBoundary) -> bool {
        true
    }

    #[cfg(feature = "federated")]
    fn federates_for_edge(&self, edge: &PartitionBoundary) -> Option<(&str, &str)> {
        let source = self.federate_for_partition(edge.source_partition);
        let target = self.federate_for_partition(edge.target_partition);
        assert_eq!(
            source.is_some(),
            target.is_some(),
            "mixed local/federated boundaries are rejected during analysis"
        );
        source.zip(target)
    }
}
