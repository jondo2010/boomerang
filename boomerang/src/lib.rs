#![allow(dead_code)]
#![feature(map_first_last)]
#![feature(associated_type_defaults)]

#[macro_use]
extern crate derivative;

pub mod builder;

pub use boomerang_runtime as runtime;

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

    #[error(transparent)]
    Runtime(#[from] boomerang_runtime::RuntimeError),
}
