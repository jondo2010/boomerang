#[macro_use]
extern crate derivative;

pub mod builder;

pub use boomerang_runtime as runtime;

#[cfg(feature = "boomerang_derive")]
#[allow(unused_imports)]
#[macro_use]
extern crate boomerang_derive;

#[cfg(feature = "boomerang_derive")]
#[doc(hidden)]
pub use boomerang_derive::*;

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
