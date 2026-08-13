//! Runtime ownership hierarchy for a lowered Federation.

use std::collections::BTreeMap;

use crate::{FederateId, FederateRuntimeBridge, FederatedRuntimeConnections, RtiGraph};

/// Owned pre-execution runtime bundle for one deployable Federate.
///
/// A runner consumes this value into independently scheduled Enclaves and one protocol client;
/// the RTI itself receives only the final RTI graph and transport endpoints.
pub struct RuntimeFederate {
    /// Protocol identity for this Federate.
    id: FederateId,
    /// Dense runtime Enclaves owned by this Federate.
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    /// Protocol bridge serving this Federate's Enclaves.
    bridge: FederateRuntimeBridge,
}

impl RuntimeFederate {
    /// Return this Federate's protocol identity.
    pub fn id(&self) -> &FederateId {
        &self.id
    }

    /// Return the dense runtime Enclaves owned by this Federate.
    pub fn enclaves(
        &self,
    ) -> &tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave> {
        &self.enclaves
    }

    /// Return mutable access to this Federate's runtime Enclaves.
    pub fn enclaves_mut(
        &mut self,
    ) -> &mut tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave> {
        &mut self.enclaves
    }

    /// Return this Federate's protocol bridge.
    pub fn bridge(&self) -> &FederateRuntimeBridge {
        &self.bridge
    }

    /// Return mutable access to this Federate's protocol bridge.
    pub fn bridge_mut(&mut self) -> &mut FederateRuntimeBridge {
        &mut self.bridge
    }

    /// Consume this Federate into its identity, Enclaves, and protocol bridge.
    pub fn into_parts(
        self,
    ) -> (
        FederateId,
        tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        FederateRuntimeBridge,
    ) {
        (self.id, self.enclaves, self.bridge)
    }
}

/// Immutable RTI graph and the deployable runtime bundle for each Federate.
pub struct RuntimeFederation {
    /// Immutable graph moved into the RTI session at startup.
    graph: RtiGraph,
    /// Runtime Federates keyed by protocol identity.
    federates: BTreeMap<FederateId, RuntimeFederate>,
}

impl RuntimeFederation {
    /// Return the immutable graph used to start the RTI.
    pub fn graph(&self) -> &RtiGraph {
        &self.graph
    }

    /// Return the runtime Federates.
    pub fn federates(&self) -> &BTreeMap<FederateId, RuntimeFederate> {
        &self.federates
    }

    /// Return mutable access to the runtime Federates.
    pub fn federates_mut(&mut self) -> &mut BTreeMap<FederateId, RuntimeFederate> {
        &mut self.federates
    }

    /// Consume this Federation into its graph and Federates.
    pub fn into_parts(self) -> (RtiGraph, BTreeMap<FederateId, RuntimeFederate>) {
        (self.graph, self.federates)
    }

    /// Construct the runtime hierarchy from validated lowering artifacts.
    #[doc(hidden)]
    pub fn from_lowered(
        graph: RtiGraph,
        mut runtimes: BTreeMap<
            FederateId,
            tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        >,
        mut bridges: FederatedRuntimeConnections,
    ) -> Result<Self, RuntimeFederationError> {
        let mut federates = BTreeMap::new();

        for id in graph.federate_ids().cloned().collect::<Vec<_>>() {
            let enclaves = runtimes
                .remove(&id)
                .ok_or_else(|| RuntimeFederationError::MissingRuntime(id.clone()))?;
            if enclaves.is_empty() {
                return Err(RuntimeFederationError::EmptyRuntime(id.clone()));
            }

            let bridge = bridges
                .take_federate(&id)
                .ok_or_else(|| RuntimeFederationError::MissingBridge(id.clone()))?;
            federates.insert(
                id.clone(),
                RuntimeFederate {
                    id: id.clone(),
                    enclaves,
                    bridge,
                },
            );
        }

        if let Some(id) = runtimes.into_keys().next() {
            return Err(RuntimeFederationError::UnknownFederate(id));
        }
        if let Some(id) = bridges.first_federate_id().cloned() {
            return Err(RuntimeFederationError::UnknownFederate(id));
        }

        Ok(Self { graph, federates })
    }
}

/// Error produced while assembling the final runtime Federation hierarchy.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeFederationError {
    /// A lowered Federate had no runtime Enclave map.
    #[error("Federate '{0}' has no owned runtime Enclaves")]
    MissingRuntime(FederateId),
    /// A lowered Federate had an empty runtime Enclave map.
    #[error("Federate '{0}' has an empty runtime Enclave map")]
    EmptyRuntime(FederateId),
    /// A lowered Federate had no protocol bridge.
    #[error("Federate '{0}' has no runtime protocol bridge")]
    MissingBridge(FederateId),
    /// Lowered runtime state referenced a Federate absent from the RTI graph.
    #[error("lowered runtime state references unknown Federate '{0}'")]
    UnknownFederate(FederateId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(id: &str) -> FederateId {
        FederateId::new(id)
    }

    fn graph(ids: &[&str]) -> RtiGraph {
        crate::rti::test_graph(
            ids.iter().map(|id| crate::rti::RtiFederateParts {
                id: fed(id),
                transitive_incoming: Vec::new(),
                affected_downstream: Vec::new(),
            }),
            [],
        )
    }

    fn runtimes(
        ids: &[&str],
    ) -> BTreeMap<
        FederateId,
        tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    > {
        ids.iter()
            .map(|id| {
                let mut enclaves = tinymap::TinyMap::new();
                enclaves.insert(boomerang_runtime::Enclave::default());
                (fed(id), enclaves)
            })
            .collect()
    }

    fn bridges(ids: &[&str]) -> FederatedRuntimeConnections {
        FederatedRuntimeConnections::new(
            ids.iter().map(|id| fed(id)),
            std::iter::empty::<crate::FederateClientRoute>(),
        )
        .unwrap()
    }

    #[test]
    fn rejects_graph_federate_without_runtime() {
        let missing = fed("b");
        let error = RuntimeFederation::from_lowered(
            graph(&["a", "b"]),
            runtimes(&["a"]),
            bridges(&["a", "b"]),
        )
        .err()
        .expect("every RTI graph federate must own a runtime");

        assert!(matches!(error, RuntimeFederationError::MissingRuntime(id) if id == missing));
    }

    #[test]
    fn rejects_runtime_for_federate_absent_from_graph() {
        let extra = fed("z");
        let error = RuntimeFederation::from_lowered(
            graph(&["a"]),
            runtimes(&["a", "z"]),
            bridges(&["a", "z"]),
        )
        .err()
        .expect("runtime state must not introduce a Federate outside the RTI graph");

        assert!(matches!(error, RuntimeFederationError::UnknownFederate(ref id) if id == &extra));
        assert!(error
            .to_string()
            .contains("lowered runtime state references unknown Federate 'z'"));
    }

    #[test]
    fn rejects_graph_federate_without_bridge() {
        let missing = fed("b");
        let error = RuntimeFederation::from_lowered(
            graph(&["a", "b"]),
            runtimes(&["a", "b"]),
            bridges(&["a"]),
        )
        .err()
        .expect("every RTI graph federate must own a protocol bridge");

        assert!(matches!(error, RuntimeFederationError::MissingBridge(id) if id == missing));
    }

    #[test]
    fn rejects_bridge_for_federate_absent_from_graph() {
        let extra = fed("z");
        let error =
            RuntimeFederation::from_lowered(graph(&["a"]), runtimes(&["a"]), bridges(&["a", "z"]))
                .err()
                .expect("bridge state must not introduce a Federate outside the RTI graph");

        assert!(matches!(error, RuntimeFederationError::UnknownFederate(ref id) if id == &extra));
        assert!(error
            .to_string()
            .contains("lowered runtime state references unknown Federate 'z'"));
    }
}
