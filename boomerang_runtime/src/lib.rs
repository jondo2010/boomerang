#![feature(map_first_last)]
#![feature(type_name_of_val)]

mod action;
mod env;
mod port;
mod reaction;
mod reactor;
mod scheduler;
mod time;

pub use action::*;
pub use env::*;
pub use port::*;
pub use reaction::*;
pub use reactor::*;
pub use scheduler::*;
pub use time::*;

pub use std::time::{Duration, Instant};

#[macro_use]
extern crate derivative;

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
