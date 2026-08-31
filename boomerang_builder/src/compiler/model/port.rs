use super::BankMember;
use crate::compiler::{ModeId, PortId, ReactorId};
#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Structural direction of a port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
pub enum PortDirection {
    /// Input received by its owning reactor.
    Input,
    /// Output produced by its owning reactor.
    Output,
}

/// Structural port declaration using stable identities.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct Port {
    /// Stable port identity.
    pub(super) id: PortId,
    /// Owning reactor identity.
    pub(super) reactor: ReactorId,
    /// Port direction.
    pub(super) direction: PortDirection,
    /// Optional structural bank membership.
    pub(super) bank: Option<BankMember>,
    /// Zero-based contiguous ordinal within the owning reactor's port declarations.
    pub(super) declaration_position: u32,
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

    /// Returns the optional structural bank membership.
    pub fn bank(&self) -> Option<BankMember> {
        self.bank
    }

    /// Returns the zero-based declaration ordinal within the owning reactor's ports.
    pub fn declaration_position(&self) -> u32 {
        self.declaration_position
    }

    /// Returns the optional modal scope.
    pub fn mode(&self) -> Option<&ModeId> {
        self.mode.as_ref()
    }
}
