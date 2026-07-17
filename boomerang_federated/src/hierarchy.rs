//! Runtime ownership hierarchy for a lowered Federation.

use std::collections::BTreeMap;

use crate::{CompiledTopology, FederateId, FederateRuntimeBridge, FederatedRuntimeConnections};

/// One deployable Federate's identity, Enclave placement, and protocol bridge.
pub struct RuntimeFederate {
    /// Protocol identity for this Federate.
    id: FederateId,
    /// Keys of the runtime Enclaves assigned to this Federate.
    enclave_keys: Vec<boomerang_runtime::EnclaveKey>,
    /// Protocol bridge serving this Federate's Enclaves.
    bridge: FederateRuntimeBridge,
}

impl RuntimeFederate {
    /// Return this Federate's protocol identity.
    pub fn id(&self) -> &FederateId {
        &self.id
    }

    /// Return the keys of the Enclaves assigned to this Federate.
    pub fn enclave_keys(&self) -> &[boomerang_runtime::EnclaveKey] {
        &self.enclave_keys
    }

    /// Return this Federate's protocol bridge.
    pub fn bridge(&self) -> &FederateRuntimeBridge {
        &self.bridge
    }

    /// Return mutable access to this Federate's protocol bridge.
    pub fn bridge_mut(&mut self) -> &mut FederateRuntimeBridge {
        &mut self.bridge
    }

    /// Consume this Federate into its identity, Enclave keys, and protocol bridge.
    pub fn into_parts(
        self,
    ) -> (
        FederateId,
        Vec<boomerang_runtime::EnclaveKey>,
        FederateRuntimeBridge,
    ) {
        (self.id, self.enclave_keys, self.bridge)
    }
}

/// Dense runtime Enclave owner plus the RTI topology and Federate metadata.
pub struct RuntimeFederation {
    /// Immutable topology used to start the RTI.
    topology: CompiledTopology,
    /// Dense owner of every runtime Enclave in this Federation.
    enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    /// Federate placement and protocol bridges keyed by protocol identity.
    federates: BTreeMap<FederateId, RuntimeFederate>,
}

impl RuntimeFederation {
    /// Return the immutable topology used to start the RTI.
    pub fn topology(&self) -> &CompiledTopology {
        &self.topology
    }

    /// Return the dense map that owns every runtime Enclave.
    pub fn enclaves(
        &self,
    ) -> &tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave> {
        &self.enclaves
    }

    /// Return the Federate metadata and protocol bridges.
    pub fn federates(&self) -> &BTreeMap<FederateId, RuntimeFederate> {
        &self.federates
    }

    /// Return mutable access to the Federate metadata and protocol bridges.
    pub fn federates_mut(&mut self) -> &mut BTreeMap<FederateId, RuntimeFederate> {
        &mut self.federates
    }

    /// Return one runtime Enclave by its globally allocated key.
    pub fn enclave(
        &self,
        key: boomerang_runtime::EnclaveKey,
    ) -> Option<&boomerang_runtime::Enclave> {
        self.enclaves.get(key)
    }

    /// Consume this Federation into its topology, dense Enclave map, and Federates.
    pub fn into_parts(
        self,
    ) -> (
        CompiledTopology,
        tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
        BTreeMap<FederateId, RuntimeFederate>,
    ) {
        (self.topology, self.enclaves, self.federates)
    }

    /// Construct the runtime hierarchy from validated lowering artifacts.
    #[doc(hidden)]
    pub fn from_lowered(
        topology: CompiledTopology,
        mut placement: BTreeMap<FederateId, Vec<boomerang_runtime::EnclaveKey>>,
        mut bridges: FederatedRuntimeConnections,
        enclaves: tinymap::TinyMap<boomerang_runtime::EnclaveKey, boomerang_runtime::Enclave>,
    ) -> Result<Self, RuntimeFederationError> {
        let mut owners =
            tinymap::TinySecondaryMap::<boomerang_runtime::EnclaveKey, FederateId>::new();
        let mut federates = BTreeMap::new();

        for id in &topology.topology().federates {
            let enclave_keys = placement
                .remove(id)
                .ok_or_else(|| RuntimeFederationError::MissingRuntime(id.clone()))?;
            for &key in &enclave_keys {
                if enclaves.get(key).is_none() {
                    return Err(RuntimeFederationError::UnknownEnclave(key));
                }
                if owners.insert(key, id.clone()).is_some() {
                    return Err(RuntimeFederationError::DuplicateEnclaveOwner(key));
                }
            }

            let bridge = bridges
                .take_federate(id)
                .ok_or_else(|| RuntimeFederationError::MissingBridge(id.clone()))?;
            federates.insert(
                id.clone(),
                RuntimeFederate {
                    id: id.clone(),
                    enclave_keys,
                    bridge,
                },
            );
        }

        if let Some(id) = placement.into_keys().next() {
            return Err(RuntimeFederationError::UnknownFederate(id));
        }
        for (key, enclave) in enclaves.iter() {
            if owners.get(key).is_none() && !enclave.env.reactions.is_empty() {
                return Err(RuntimeFederationError::MissingEnclaveOwner(key));
            }
        }

        Ok(Self {
            topology,
            enclaves,
            federates,
        })
    }
}

/// Error produced while assembling the final runtime Federation hierarchy.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeFederationError {
    /// An Enclave was assigned to more than one Federate.
    #[error("Enclave {0:?} is assigned to more than one Federate")]
    DuplicateEnclaveOwner(boomerang_runtime::EnclaveKey),
    /// A non-empty Enclave was not assigned to a Federate.
    #[error("Enclave {0:?} has no owning Federate")]
    MissingEnclaveOwner(boomerang_runtime::EnclaveKey),
    /// Placement referenced an Enclave outside the owning dense map.
    #[error("Federate placement references unknown Enclave {0:?}")]
    UnknownEnclave(boomerang_runtime::EnclaveKey),
    /// A topology Federate had no runtime placement entry.
    #[error("Federate '{0}' has no owned runtime Enclaves")]
    MissingRuntime(FederateId),
    /// A topology Federate had no protocol bridge.
    #[error("Federate '{0}' has no runtime protocol bridge")]
    MissingBridge(FederateId),
    /// Runtime placement referenced a Federate absent from the topology.
    #[error("runtime placement references unknown Federate '{0}'")]
    UnknownFederate(FederateId),
}
