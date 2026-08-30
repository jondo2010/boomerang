use crate::{
    compiler::{ActionId, ModeId, ReactorId},
    runtime,
};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Structural action category and its category-specific timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub enum ActionKind {
    /// Logical action scheduled in logical time.
    Logical {
        /// Optional minimum logical delay before scheduling.
        minimum_delay: Option<runtime::Duration>,
    },
    /// Physical action sourced outside logical time.
    Physical {
        /// Optional minimum physical delay before scheduling.
        minimum_delay: Option<runtime::Duration>,
    },
    /// Runtime timer action.
    Timer {
        /// Optional timer offset from startup.
        offset: Option<runtime::Duration>,
        /// Optional timer period.
        period: Option<runtime::Duration>,
    },
    /// Built-in startup action.
    Startup,
    /// Built-in shutdown action.
    Shutdown,
}

/// Structural action declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct Action {
    /// Stable action identity.
    pub(super) id: ActionId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Action category.
    pub(super) kind: ActionKind,
    /// Zero-based contiguous ordinal within the owning reactor's action declarations.
    pub(super) declaration_position: u32,
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

    /// Returns the zero-based declaration ordinal within the owning reactor's actions.
    pub fn declaration_position(&self) -> u32 {
        self.declaration_position
    }

    /// Returns the optional modal scope.
    pub fn mode(&self) -> Option<&ModeId> {
        self.mode.as_ref()
    }
}
