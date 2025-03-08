#![doc=include_str!("../README.md")]
//!
//! ## Example
//!
//! Build and run a Reactor with reactions that respond to startup and shutdown actions:
//!
//! ```rust
//! use boomerang::prelude::*;
//!
//! struct State {
//!     success: bool,
//! }
//!
//! #[derive(Reactor)]
//! #[reactor(
//!     state = "State",
//!     reaction = "ReactionStartup",
//!     reaction = "ReactionShutdown"
//! )]
//! struct HelloWorld;
//!
//! #[derive(Reaction)]
//! #[reaction(
//!     reactor = "HelloWorld",
//!     triggers(startup)
//! )]
//! struct ReactionStartup;
//!
//! impl runtime::Trigger<State> for ReactionStartup {
//!     fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
//!         println!("Hello World.");
//!         state.success = true;
//!     }
//! }
//!
//! #[derive(Reaction)]
//! #[reaction(
//!     reactor = "HelloWorld",
//!     triggers(shutdown)
//! )]
//! struct ReactionShutdown;
//!
//! impl runtime::Trigger<State> for ReactionShutdown {
//!     fn trigger(self, _ctx: &mut runtime::Context, state: &mut State) {
//!         println!("Shutdown invoked.");
//!         assert!(state.success, "ERROR: startup reaction not executed.");
//!     }
//! }
//!
//! let config = runtime::Config::default().with_fast_forward(true);
//! let (_, envs) = boomerang_util::runner::build_and_test_reactor::<HelloWorld>(
//!     "hello_world",
//!     State { success: false },
//!     config,
//! )
//! .unwrap();
//!
//! assert!(envs[0]
//!     .find_reactor_by_name("hello_world")
//!     .and_then(|reactor| reactor.get_state::<State>())
//!     .unwrap().success,
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
        BuilderError, BuilderFqn, BuilderRuntimeParts, EnvBuilder, Input, Logical, Output,
        Physical, Reactor, TimerActionKey, TypedActionKey, TypedPortKey,
    };

    pub use super::runtime::{self, CommonContext, Duration, FromRefs, Tag};

    pub use boomerang_derive::{Reaction, Reactor};
}

#[cfg(feature = "derive")]
#[doc(hidden)]
pub use boomerang_derive::*;

/// Top-level error type for Boomerang
#[derive(thiserror::Error, Debug)]
pub enum BoomerangError {
    #[error(transparent)]
    Builder(#[from] builder::BuilderError),

    #[error(transparent)]
    Runtime(#[from] runtime::RuntimeError),
}
