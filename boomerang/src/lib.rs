#![doc=include_str!("../README.md")]
//!
//! ## Example
//!
//! Build and run a Reactor with a startup reaction:
//!
//! ```rust
//! use boomerang::prelude::*;
//!
//! struct State {
//!     success: bool,
//! }
//!
//! #[reactor(state = State)]
//! fn HelloWorld() -> impl Reactor {
//!     reaction! {
//!         Startup (startup) {
//!             println!("Hello World.");
//!             state.success = true;
//!             ctx.schedule_shutdown(None);
//!         }
//!     }
//! }
//!
//! let config = runtime::Config::default().with_fast_forward(true);
//! let (_, envs) = boomerang_util::runner::build_and_test_reactor(
//!     HelloWorld(),
//!     "hello_world",
//!     State { success: false },
//!     config,
//! )
//! .unwrap();
//!
//! assert!(envs[0]
//!     .find_reactor_by_name("hello_world")
//!     .and_then(|reactor| reactor.get_state::<State>())
//!     .unwrap()
//!     .success,
//! );
//! ```
//!
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod flatten_transposed;

// Re-exports
pub use boomerang_builder as builder;
pub use boomerang_runtime as runtime;

pub mod prelude {
    //! Re-exported common types and traits for Boomerang

    pub use super::builder::{
        BuilderError, BuilderFqn, BuilderRuntimeParts, Contained, EnvBuilder, Input, Local,
        Logical, Output, Physical, Reactor, TimerActionKey, TimerSpec, TypedActionKey,
        TypedPortKey,
    };

    pub use super::runtime::{self, action::ActionCommon, CommonContext, Duration, FromRefs, Tag};

    pub use boomerang_macros::{reaction, reactor, reactor_ports, timer};

    pub use crate::flatten_transposed::FlattenTransposedExt;
}


/// Top-level error type for Boomerang
#[derive(thiserror::Error, Debug)]
pub enum BoomerangError {
    #[error(transparent)]
    Builder(#[from] builder::BuilderError),

    #[error(transparent)]
    Runtime(#[from] runtime::RuntimeError),
}
