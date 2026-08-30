use crate::compiler::{ComponentInstanceId, ContractId};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Logical component instance and its required contract.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct ComponentInstance {
    /// Stable component identity.
    pub(super) id: ComponentInstanceId,
    /// Required external contract.
    pub(super) contract: ContractId,
    /// Required external contract version.
    pub(super) contract_version: u64,
}

impl ComponentInstance {
    /// Constructs a component from already validated stable identities.
    pub(crate) fn from_ids(
        id: ComponentInstanceId,
        contract: ContractId,
        contract_version: u64,
    ) -> Self {
        Self {
            id,
            contract,
            contract_version,
        }
    }

    /// Constructs a validated component declaration.
    pub fn new(
        id: impl AsRef<str>,
        contract: impl Into<Box<str>>,
        contract_version: u64,
    ) -> Result<Self, crate::compiler::InvalidStableId> {
        Ok(Self {
            id: ComponentInstanceId::new(id)?,
            contract: ContractId::new(contract)?,
            contract_version,
        })
    }

    /// Returns the stable component identity.
    pub fn id(&self) -> &ComponentInstanceId {
        &self.id
    }

    /// Returns the required contract identity.
    pub fn contract(&self) -> &ContractId {
        &self.contract
    }

    /// Returns the required contract version.
    pub fn contract_version(&self) -> u64 {
        self.contract_version
    }
}
