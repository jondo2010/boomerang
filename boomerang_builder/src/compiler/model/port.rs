use crate::compiler::{ModeId, PortId, ReactorId};

/// Structural direction of a port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortDirection {
    /// Input received by its owning reactor.
    Input,
    /// Output produced by its owning reactor.
    Output,
}

/// Structural port declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    /// Stable port identity.
    pub(super) id: PortId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Port direction.
    pub(super) direction: PortDirection,
    /// Optional modal scope.
    pub(super) mode: Option<ModeId>,
}

impl Port {
    /// Returns the stable port identity.
    pub fn id(&self) -> &PortId {
        &self.id
    }

    /// Returns the owning reactor identity.
    pub fn reactor(&self) -> &ReactorId {
        &self.reactor
    }

    /// Returns the port direction.
    pub fn direction(&self) -> PortDirection {
        self.direction
    }

    /// Returns the optional modal scope.
    pub fn mode(&self) -> Option<&ModeId> {
        self.mode.as_ref()
    }
}
