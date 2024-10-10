#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

pub mod action;
mod context;
pub mod data;
mod env;
mod event;
pub mod keepalive;
mod key_set;
pub mod port;
pub mod reaction;
mod reactor;
mod refs;
mod registry;
mod sched;
pub mod store;
mod time;

// Re-exports
pub use action::{
    Action, ActionKey, ActionRef, ActionRefValue, LogicalAction, PhysicalAction, PhysicalActionRef,
};
pub use context::*;
pub use data::ReactorData;
pub use env::{BankInfo, Env, Level, LevelReactionKey, ReactionGraph};
pub use key_set::KeySetLimits as ReactionSetLimits;
pub use port::*;
pub use reaction::{
    BoxedReactionFn, Deadline, FromRefs, Reaction, ReactionFn, ReactionKey, ReactionSet,
    ReactionWrapper, Trigger,
};
pub use reactor::*;
pub use refs::{Refs, RefsMut};
pub use sched::*;
pub use time::*;

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
