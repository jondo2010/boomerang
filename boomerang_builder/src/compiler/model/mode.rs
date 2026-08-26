use crate::compiler::{ModeId, ReactorId};

/// Mode in a reactor-local hierarchy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mode {
    /// Stable mode identity.
    pub(super) id: ModeId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Optional parent mode identity.
    pub(super) parent: Option<ModeId>,
    /// Whether this is the initial sibling mode.
    pub(super) initial: bool,
}

impl Mode {
    /// Returns the stable mode identity.
    pub fn id(&self) -> &ModeId {
        &self.id
    }

    /// Returns the owning reactor identity.
    pub fn reactor(&self) -> &ReactorId {
        &self.reactor
    }

    /// Returns the optional parent mode identity.
    pub fn parent(&self) -> Option<&ModeId> {
        self.parent.as_ref()
    }

    /// Reports whether this is the initial sibling mode.
    pub fn is_initial(&self) -> bool {
        self.initial
    }
}
