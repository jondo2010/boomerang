use crate::compiler::{ComponentInstanceId, ModeId, PlacementGroupId, ReactorId, StableEnclaveId};

/// Structural reactor declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reactor {
    /// Stable reactor identity.
    pub(super) id: ReactorId,
    /// Owning component identity.
    pub(super) component: ComponentInstanceId,
    /// Optional parent reactor identity.
    pub(super) parent: Option<ReactorId>,
    /// Enclave containing this reactor.
    pub(super) enclave: StableEnclaveId,
    /// Optional placement-group identity.
    pub(super) placement_group: Option<PlacementGroupId>,
    /// Optional enclosing mode owned by the structural parent reactor.
    pub(super) scope_mode: Option<ModeId>,
}

impl Reactor {
    /// Returns the stable reactor identity.
    pub fn id(&self) -> &ReactorId {
        &self.id
    }

    /// Returns the owning component identity.
    pub fn component(&self) -> &ComponentInstanceId {
        &self.component
    }

    /// Returns the optional structural parent identity.
    pub fn parent(&self) -> Option<&ReactorId> {
        self.parent.as_ref()
    }

    /// Returns the containing Enclave identity.
    pub fn enclave(&self) -> &StableEnclaveId {
        &self.enclave
    }

    /// Returns the optional placement-group identity.
    pub fn placement_group(&self) -> Option<&PlacementGroupId> {
        self.placement_group.as_ref()
    }

    /// Returns the optional enclosing parent-mode identity.
    pub fn scope_mode(&self) -> Option<&ModeId> {
        self.scope_mode.as_ref()
    }
}
