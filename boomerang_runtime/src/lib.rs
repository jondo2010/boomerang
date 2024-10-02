#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

mod action;
mod context;
mod env;
mod event;
pub mod keepalive;
mod key_set;
mod partition;
mod port;
mod reaction;
mod reactor;
mod sched;
mod store;
mod time;

// Re-exports
pub use action::*;
pub use context::*;
pub use env::*;
pub use key_set::KeySetLimits as ReactionSetLimits;
pub use partition::{partition, partition_mut, Partition, PartitionMut};
pub use port::*;
pub use reaction::*;
pub use reactor::*;
pub use sched::*;
pub use time::*;

pub trait PortData: std::fmt::Debug + Send + Sync + 'static {}
impl<T> PortData for T where T: std::fmt::Debug + Send + Sync + 'static {}

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(PortKey),

    #[error("Mismatched Dynamic Types found {found} but wanted {wanted}")]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },

    #[cfg(feature = "serde")]
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_arrow::Error),

    #[error("Destructuring error")]
    DestrError,
}

pub mod fmt_utils {
    //! Utility functions for formatting until [debug_closure_helpers](https://github.com/rust-lang/rust/issues/117729) lands in stable.
    pub fn from_fn<F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result>(f: F) -> FromFn<F> {
        FromFn(f)
    }

    pub struct FromFn<F>(F)
    where
        F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result;

    impl<F> std::fmt::Debug for FromFn<F>
    where
        F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result,
    {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            (self.0)(f)
        }
    }
}
