#![doc=include_str!("../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

mod action;
mod connection;
mod env;
#[cfg(feature = "federated")]
mod federation;
mod fqn;
mod inter_partition;
mod port;
mod reaction;
mod reactor;
#[cfg(test)]
pub mod tests;

mod macro_support;
pub use macro_support::{Reactor, ReactorPorts};

pub mod plantuml;

pub use action::*;
pub use env::*;
#[cfg(feature = "federated")]
pub use federation::*;
pub use fqn::*;
pub use inter_partition::*;
pub use port::{
    BuilderPortKey, Contained, Input, Local, Output, PortBank, PortBuilder, PortTag, PortType,
    TypedPortKey,
};
pub use reaction::*;
pub use reactor::*;

pub use boomerang_runtime::TransitionKind;
use boomerang_runtime::{self as runtime};

#[derive(thiserror::Error, Debug)]
pub enum BuilderError {
    #[error("Duplicate Port Definition: {}.{}", reactor_name, port_name)]
    DuplicatePortDefinition {
        reactor_name: String,
        port_name: String,
    },

    #[error("Duplicate Action Definition: {}.{}", reactor_name, action_name)]
    DuplicateActionDefinition {
        reactor_name: String,
        action_name: String,
    },

    #[error("Duplicate Mode Definition: {}.{}", reactor_name, mode_name)]
    DuplicateModeDefinition {
        reactor_name: String,
        mode_name: String,
    },

    #[error("Multiple initial modes defined for reactor {reactor_name}")]
    MultipleInitialModes { reactor_name: String },

    #[error("ActionKey not found: {}", 0)]
    ActionKeyNotFound(BuilderActionKey),

    #[error("ReactorKey not found: {}", 0)]
    ReactorKeyNotFound(BuilderReactorKey),

    #[error("PortKey not found: {}", 0)]
    PortKeyNotFound(BuilderPortKey),

    #[error("ReactionKey not found: {}", 0)]
    ReactionKeyNotFound(BuilderReactionKey),

    #[error("A Port named '{0}' was not found.")]
    NamedPortNotFound(String),

    #[error("A Reaction named '{0}' was not found.")]
    NamedReactionNotFound(String),

    #[error("An Action named '{0}' was not found.")]
    NamedActionNotFound(String),

    #[error("A Reactor named '{0}' was not found.")]
    NamedReactorNotFound(String),

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found: {what:?}.")]
    ReactionGraphCycle { what: Vec<BuilderReactionKey> },

    #[error("A cycle in the Reactor graph was found.")]
    ReactorGraphCycle { what: BuilderReactorKey },

    #[error("Error binding ports ({source_key:?}->{target_key:?}): {what}")]
    PortConnectionError {
        source_key: BuilderPortKey,
        target_key: BuilderPortKey,
        what: String,
    },

    #[error("Port connection length mismatch: {from} -> {to}")]
    PortConnectionLengthMismatch { from: usize, to: usize },

    #[error("Unsupported federation topology: {what}")]
    UnsupportedFederationTopology { what: String },

    #[error("Error building Reaction: {0}")]
    ReactionBuilderError(String),

    #[error("Invalid fully-qualified name: {0}")]
    InvalidFqn(String),

    #[error("Internal Error: {0}")]
    InternalError(String),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error("ReplayKey already exists: {}", 0)]
    ReplayKeyAlreadyExists(BuilderActionKey),
}

impl From<std::convert::Infallible> for BuilderError {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}
