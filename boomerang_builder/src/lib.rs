#![doc=include_str!("../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

mod action;
mod env;
mod fqn;
mod port;
mod reaction;
mod reactor;
#[cfg(test)]
pub mod tests;

#[cfg(feature = "graphviz")]
pub mod graphviz;

pub use action::*;
pub use env::*;
pub use fqn::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;

use boomerang_runtime as runtime;

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

    #[error("Error binding ports ({:?}->{:?}): {}", port_a_key, port_b_key, what)]
    PortBindError {
        port_a_key: BuilderPortKey,
        port_b_key: BuilderPortKey,
        what: String,
    },

    #[error("Error building Reaction: {0}")]
    ReactionBuilderError(String),

    #[error("Invalid fully-qualified name: {0}")]
    InvalidFqn(String),

    #[error("Internal Error: {0}")]
    InternalError(String),

    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

impl From<std::convert::Infallible> for BuilderError {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

/// A macro to create a new reaction closure.
#[macro_export]
macro_rules! reaction_closure {
    // empty closure case
    () => {
        Box::new(
            |_ctx: &mut runtime::Context,
             _state: &mut dyn runtime::ReactorState,
             _ref_ports: runtime::Refs<dyn runtime::BasePort>,
             _mut_ports: runtime::RefsMut<dyn runtime::BasePort>,
             _actions: runtime::RefsMut<runtime::Action>| {},
        )
    };
    // closure with body
    ( $ctx:ident, $state:ident, $ref_ports:ident, $mut_ports:ident, $actions:ident => $body:block ) => {
        Box::new(
            move |$ctx: &mut runtime::Context,
                  $state: &mut dyn runtime::ReactorState,
                  $ref_ports: runtime::Refs<dyn runtime::BasePort>,
                  $mut_ports: runtime::RefsMut<dyn runtime::BasePort>,
                  $actions: runtime::RefsMut<runtime::Action>| { $body },
        )
    };
}

#[test]
fn test_reaction_closure() {
    let _closure = reaction_closure!(ctx, _state, _ref_ports, _mut_ports, _actions => {
        ctx.get_elapsed_logical_time();
    });
}
