//! Runtime ownership hierarchy for a lowered Federation.

use std::collections::BTreeMap;

use crate::{CompiledTopology, FederateId, FederateRuntimeBridge, FederatedRuntimeConnections};

/// One deployable Federate's identity, owned Enclaves, and protocol bridge.
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

/// RTI topology and deployable runtime Federates.
pub struct RuntimeFederation {
    /// Immutable topology used to start the RTI.
    topology: CompiledTopology,
    /// Runtime Federates keyed by protocol identity.
    federates: BTreeMap<FederateId, RuntimeFederate>,
}

impl RuntimeFederation {
    /// Return the immutable topology used to start the RTI.
    pub fn topology(&self) -> &CompiledTopology {
        &self.topology
    }

    /// Return the runtime Federates.
    pub fn federates(&self) -> &BTreeMap<FederateId, RuntimeFederate> {
        &self.federates
    }

    /// Return mutable access to the runtime Federates.
    pub fn federates_mut(&mut self) -> &mut BTreeMap<FederateId, RuntimeFederate> {
        &mut self.federates
    }

    /// Consume this Federation into its topology and Federates.
    pub fn into_parts(self) -> (CompiledTopology, BTreeMap<FederateId, RuntimeFederate>) {
        (self.topology, self.federates)
    }

    /// Construct the runtime hierarchy from validated lowering artifacts.
    #[doc(hidden)]
    pub fn from_lowered(
        topology: CompiledTopology,
        mut runtimes: BTreeMap<
            FederateId,
            tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        >,
        mut bridges: FederatedRuntimeConnections,
    ) -> Result<Self, RuntimeFederationError> {
        let mut federates = BTreeMap::new();

        for id in &topology.topology().federates {
            let enclaves = runtimes
                .remove(id)
                .ok_or_else(|| RuntimeFederationError::MissingRuntime(id.clone()))?;
            if enclaves.is_empty() {
                return Err(RuntimeFederationError::EmptyRuntime(id.clone()));
            }

            let bridge = bridges
                .take_federate(id)
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

        Ok(Self {
            topology,
            federates,
        })
    }
}

/// Error produced while assembling the final runtime Federation hierarchy.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeFederationError {
    /// A topology Federate had no runtime Enclave map.
    #[error("Federate '{0}' has no owned runtime Enclaves")]
    MissingRuntime(FederateId),
    /// A topology Federate had an empty runtime Enclave map.
    #[error("Federate '{0}' has an empty runtime Enclave map")]
    EmptyRuntime(FederateId),
    /// A topology Federate had no protocol bridge.
    #[error("Federate '{0}' has no runtime protocol bridge")]
    MissingBridge(FederateId),
    /// Runtime placement referenced a Federate absent from the topology.
    #[error("runtime placement references unknown Federate '{0}'")]
    UnknownFederate(FederateId),
}
