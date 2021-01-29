#![allow(dead_code)]
#![feature(map_first_last)]

#[macro_use]
extern crate derivative;

pub mod builder;
pub mod runtime;

#[derive(thiserror::Error, Debug)]
pub enum BoomerangError {
    /// An internal builder error
    #[error("Internal Builder Error")]
    BuilderInternal,
    /// An arbitrary error message.
    #[error("{0}")]
    Custom(String),

    #[error(transparent)]
    Builder(#[from] builder::BuilderError),
}
