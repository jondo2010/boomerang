#![feature(new_uninit)]
#![feature(type_alias_impl_trait)]
#![feature(get_many_mut)]

mod action;
mod context;
mod env;
mod key_set;
mod port;
mod reaction;
mod reactor;
mod sched;
mod time;
pub mod util;

// Re-exports
pub use action::*;
pub use context::*;
pub use env::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;
pub use sched::*;
pub use time::*;

pub use std::time::{Duration, Instant};

#[macro_use]
extern crate derivative;

pub trait PortData: std::fmt::Debug + Send + Sync + 'static {}
impl<T> PortData for T where T: std::fmt::Debug + Send + Sync + 'static {}

/// Used to get access to the inner type from Port, Action, etc.
pub trait InnerType {
    type Inner: PortData;
}

#[derive(thiserror::Error, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(PortKey),

    #[error("Mismatched Dynamic Types found {} but wanted {}", found, wanted)]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },
}
