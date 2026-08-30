use crate::{
    compiler::{BoundaryId, PortId},
    runtime,
};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Transfer semantics of a structural connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub enum ConnectionSemantics {
    /// Logical connection carrying an optional `after` delay.
    Logical {
        /// Optional logical delay applied to transferred values.
        after: Option<runtime::Duration>,
    },
    /// Physical connection carrying an optional `after` delay.
    Physical {
        /// Optional physical delay applied to transferred values.
        after: Option<runtime::Duration>,
    },
}

/// Directed structural connection between stable port identities.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct Connection {
    /// Stable connection identity.
    pub(super) id: BoundaryId,
    /// Source output-port identity.
    pub(super) source: PortId,
    /// Target input-port identity.
    pub(super) target: PortId,
    /// Logical or physical transfer semantics.
    pub(super) semantics: ConnectionSemantics,
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

    /// Returns the logical or physical transfer semantics.
    pub fn semantics(&self) -> ConnectionSemantics {
        self.semantics
    }
}
