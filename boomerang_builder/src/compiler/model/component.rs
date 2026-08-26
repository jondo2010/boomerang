use crate::compiler::{ComponentInstanceId, ContractId};

/// Logical component instance and its required contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInstance {
    /// Stable component identity.
    pub(super) id: ComponentInstanceId,
    /// Required external contract.
    pub(super) contract: ContractId,
}

impl ComponentInstance {
    /// Constructs a validated component declaration.
    pub fn new(
        id: impl AsRef<str>,
        contract: impl Into<Box<str>>,
    ) -> Result<Self, crate::compiler::InvalidStableId> {
        Ok(Self {
            id: ComponentInstanceId::new(id)?,
            contract: ContractId::new(contract)?,
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
}
