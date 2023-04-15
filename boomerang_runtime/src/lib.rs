mod action;
mod context;
mod env;
mod key_set;
mod port;
mod reaction;
mod reactor;
mod sched;
pub mod util;

// Re-exports
pub use action::*;
pub use context::*;
pub use env::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;
pub use sched::*;

pub use boomerang_core::{
    keys,
    time::{Tag, Timestamp},
};

#[macro_use]
extern crate derivative;

pub trait PortData: std::fmt::Debug + Send + Sync + 'static {}
impl<T> PortData for T where T: std::fmt::Debug + Send + Sync + 'static {}

/// Used to get access to the inner type from Port, Action, etc.
pub trait InnerType {
    type Inner: PortData;
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(keys::PortKey),

    #[error("Mismatched Dynamic Types found {} but wanted {}", found, wanted)]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },

    #[cfg(feature = "federated")]
    #[error(transparent)]
    Federate(#[from] boomerang_federated::client::ClientError),
}
