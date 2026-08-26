use crate::compiler::{ActionId, ModeId, ReactorId};

/// Structural action category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    /// Logical action scheduled in logical time.
    Logical,
    /// Physical action sourced outside logical time.
    Physical,
    /// Runtime timer action.
    Timer,
    /// Built-in startup action.
    Startup,
    /// Built-in shutdown action.
    Shutdown,
}

/// Structural action declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Action {
    /// Stable action identity.
    pub(super) id: ActionId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Action category.
    pub(super) kind: ActionKind,
    /// Optional modal scope.
    pub(super) mode: Option<ModeId>,
}

impl Action {
    /// Returns the stable action identity.
    pub fn id(&self) -> &ActionId {
        &self.id
    }

    /// Returns the owning reactor identity.
    pub fn reactor(&self) -> &ReactorId {
        &self.reactor
    }

    /// Returns the action category.
    pub fn kind(&self) -> ActionKind {
        self.kind
    }

    /// Returns the optional modal scope.
    pub fn mode(&self) -> Option<&ModeId> {
        self.mode.as_ref()
    }
}
