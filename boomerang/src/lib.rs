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
//! let mut env_builder = EnvBuilder::new();
//! let reactor = HelloWorld::build(
//!     "hello_world",
//!     State {
//!         success: false
//!     },
//!     None,
//!     None,
//!     &mut env_builder
//! ).unwrap();
//! let (mut env, triggers, _) = env_builder.into_runtime_parts().unwrap();
//! let config = runtime::Config::default().with_fast_forward(true);
//! let mut sched = runtime::Scheduler::new(env, triggers, config);
//! sched.event_loop();
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
        BuilderError, BuilderFqn, EnvBuilder, Input, Logical, Output, Physical, Reactor,
        TimerActionKey, TypedActionKey, TypedPortKey,
    };

    pub use super::runtime::{self, ContextCommon, FromRefs};

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
