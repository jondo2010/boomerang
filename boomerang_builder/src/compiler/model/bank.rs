#[cfg(feature = "host-interchange")]
use serde::{Deserialize, Serialize};

/// Validated structural metadata for one member of a bank declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "host-interchange", derive(Deserialize, Serialize))]
#[cfg_attr(feature = "host-interchange", serde(deny_unknown_fields))]
pub struct BankMember {
    /// Zero-based member index within the bank.
    index: u32,
    /// Total number of members declared by the bank.
    total: u32,
}

impl BankMember {
    /// Creates bank metadata when the total is non-zero and contains the index.
    pub fn new(index: u32, total: u32) -> Result<Self, InvalidBankMember> {
        if total == 0 {
            return Err(InvalidBankMember::Empty);
        }
        if index >= total {
            return Err(InvalidBankMember::IndexOutOfBounds { index, total });
        }
        Ok(Self { index, total })
    }

    /// Returns the zero-based member index.
    pub fn index(self) -> u32 {
        self.index
    }

    /// Returns the total number of members in the bank.
    pub fn total(self) -> u32 {
        self.total
    }
}

/// Reports invalid structural bank metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InvalidBankMember {
    /// The bank declares no members.
    #[error("bank member total must be non-zero")]
    Empty,
    /// The member index is outside the declared bank.
    #[error("bank member index {index} is outside total {total}")]
    IndexOutOfBounds {
        /// Invalid zero-based member index.
        index: u32,
        /// Declared total number of members.
        total: u32,
    },
}
