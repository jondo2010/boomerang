#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

pub mod action;
mod context;
mod env;
mod event;
pub mod keepalive;
mod key_set;
pub mod port;
pub mod reaction;
mod reactor;
mod refs;
mod sched;
pub mod store;
mod time;

// Re-exports
pub use action::{Action, ActionCommon, ActionKey, ActionRef, AsyncActionRef, BaseAction};
pub use context::*;
pub use env::{BankInfo, Env, Level, LevelReactionKey, ReactionGraph};
pub use key_set::KeySetLimits as ReactionSetLimits;
pub use port::*;
pub use reaction::{
    BoxedReactionFn, Deadline, FromRefs, Reaction, ReactionAdapter, ReactionFn, ReactionKey,
    ReactionSet, Trigger,
};
pub use reactor::*;
pub use refs::{Refs, RefsMut};
pub use sched::*;
pub use time::*;

/// Types implementing this trait can be used as data in ports, actions, and reactors.
pub trait ReactorData: Send + Sync + 'static {}

impl<T> ReactorData for T where T: Send + Sync + 'static {}

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(PortKey),

    #[error("Mismatched Dynamic Types found {found} but wanted {wanted}")]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },

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
