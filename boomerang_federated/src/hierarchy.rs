//! Self-contained runtime ownership for Federates and their Enclaves.

use std::collections::BTreeMap;

use crate::{CompiledTopology, FederateId, FederateRuntimeBridge, FederatedRuntimeConnections};

/// One independently movable compute node in a Federation.
pub struct RuntimeFederate {
    id: FederateId,
    runtime: boomerang_runtime::RuntimeEnclaves,
    bridge: FederateRuntimeBridge,
}

impl RuntimeFederate {
    pub fn id(&self) -> &FederateId {
        &self.id
    }

    pub fn enclaves(&self) -> &boomerang_runtime::RuntimeEnclaves {
        &self.runtime
    }

    pub fn bridge(&self) -> &FederateRuntimeBridge {
        &self.bridge
    }

    pub fn bridge_mut(&mut self) -> &mut FederateRuntimeBridge {
        &mut self.bridge
    }

    pub fn into_parts(
        self,
    ) -> (
        FederateId,
        boomerang_runtime::RuntimeEnclaves,
        FederateRuntimeBridge,
    ) {
        (self.id, self.runtime, self.bridge)
    }
}

/// Independent RTI topology plus the runnable Federates attached to its star.
pub struct RuntimeFederation {
    topology: CompiledTopology,
    federates: BTreeMap<FederateId, RuntimeFederate>,
}

impl RuntimeFederation {
    pub fn topology(&self) -> &CompiledTopology {
        &self.topology
    }

    pub fn federates(&self) -> &BTreeMap<FederateId, RuntimeFederate> {
        &self.federates
    }

    pub fn federates_mut(&mut self) -> &mut BTreeMap<FederateId, RuntimeFederate> {
        &mut self.federates
    }

    pub fn enclave(
        &self,
        key: boomerang_runtime::EnclaveKey,
    ) -> Option<&boomerang_runtime::Enclave> {
        self.federates
            .values()
            .find_map(|federate| federate.enclaves().get(key))
    }

    pub fn enclaves(
        &self,
    ) -> impl Iterator<Item = (boomerang_runtime::EnclaveKey, &boomerang_runtime::Enclave)> {
        self.federates
            .values()
            .flat_map(|federate| federate.enclaves().iter())
    }

    pub fn into_parts(self) -> (CompiledTopology, BTreeMap<FederateId, RuntimeFederate>) {
        (self.topology, self.federates)
    }

    #[doc(hidden)]
    pub fn from_lowered(
        topology: CompiledTopology,
        placement: BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>>,
        mut bridges: FederatedRuntimeConnections,
        runtime: boomerang_runtime::RuntimeEnclaves,
    ) -> Result<Self, RuntimeFederationError> {
        let mut runtimes = runtime.split_by(placement)?;
        let mut federates = BTreeMap::new();
        for id in &topology.topology().federates {
            let runtime = runtimes
                .remove(id)
                .ok_or_else(|| RuntimeFederationError::MissingRuntime(id.clone()))?;
            let bridge = bridges
                .take_federate(id)
                .ok_or_else(|| RuntimeFederationError::MissingBridge(id.clone()))?;
            federates.insert(
                id.clone(),
                RuntimeFederate {
                    id: id.clone(),
                    runtime,
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

#[derive(Debug, thiserror::Error)]
pub enum RuntimeFederationError {
    #[error(transparent)]
    Enclaves(#[from] boomerang_runtime::RuntimeEnclavesError),
    #[error("Federate '{0}' has no owned runtime Enclaves")]
    MissingRuntime(FederateId),
    #[error("Federate '{0}' has no runtime protocol bridge")]
    MissingBridge(FederateId),
    #[error("runtime placement references unknown Federate '{0}'")]
    UnknownFederate(FederateId),
}
