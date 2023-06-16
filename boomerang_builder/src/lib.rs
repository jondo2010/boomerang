#[macro_use]
extern crate derivative;

mod action;
mod env;
#[cfg(feature = "visualization")]
pub mod graphviz;
mod port;
mod reaction;
mod reactor;
mod util;

// re-exports
pub use action::*;
pub use env::*;
pub use port::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;

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

    #[error("A Port named '{}' was not found.", 0)]
    NamedPortNotFound(String),

    #[error("An Action named '{}' was not found.", 0)]
    NamedActionNotFound(String),

    #[error("Inconsistent Builder State: {}", what)]
    InconsistentBuilderState {
        what: String,
        // sub_error: String, //Option<BuilderError>,
    },

    #[error("A cycle in the Reaction graph was found.")]
    ReactionGraphCycle { what: BuilderReactionKey },

    #[error("A cycle in the Reactor graph was found.")]
    ReactorGraphCycle { what: BuilderReactorKey },

    #[error("Error binding ports ({:?}->{:?}): {}", port_a_key, port_b_key, what)]
    PortBindError {
        port_a_key: BuilderPortKey,
        port_b_key: BuilderPortKey,
        what: String,
    },

    #[error("Expected a top-level reactor, but this reactor has a parent: {parent:?}")]
    NotTopLevelReactor { parent: BuilderReactorKey },

    #[error("Expected a child reactor, but this reactor has no parent: {reactor:?}")]
    NotChildReactor { reactor: BuilderReactorKey },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
