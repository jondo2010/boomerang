use crate::compiler::{BoundaryId, PortId};

/// Directed structural connection between stable port identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Connection {
    /// Stable connection identity.
    pub(super) id: BoundaryId,
    /// Source output-port identity.
    pub(super) source: PortId,
    /// Target input-port identity.
    pub(super) target: PortId,
}

impl Connection {
    /// Returns the stable connection identity.
    pub fn id(&self) -> &BoundaryId {
        &self.id
    }

    /// Returns the source port identity.
    pub fn source(&self) -> &PortId {
        &self.source
    }

    /// Returns the target port identity.
    pub fn target(&self) -> &PortId {
        &self.target
    }
}
