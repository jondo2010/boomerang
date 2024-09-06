#![doc=include_str!( "../../README.md")]
//!
//! ## Example
//!
//! Build and run a Reactor with reactions that respond to startup and shutdown actions:
//!
//! ```rust
//! use boomerang::{builder::prelude::*, runtime, Reactor, Reaction};
//!
//! struct State {
//!     success: bool,
//! }
//!
//! #[derive(Reactor, Clone)]
//! #[reactor(state = State)]
//! struct HelloWorld {
//!     reaction_startup: TypedReactionKey<ReactionStartup>,
//!     reaction_shutdown: TypedReactionKey<ReactionShutdown>,
//! }
//!
//! #[derive(Reaction)]
//! #[reaction(triggers(startup))]
//! struct ReactionStartup;
//!
//! impl Trigger for ReactionStartup {
//!     type Reactor = HelloWorld;
//!     fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut State) {
//!         println!("Hello World.");
//!         state.success = true;
//!     }
//! }
//!
//! #[derive(Reaction)]
//! #[reaction(triggers(shutdown))]
//! struct ReactionShutdown;
//!
//! impl Trigger for ReactionShutdown {
//!     type Reactor = HelloWorld;
//!     fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut State) {
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
//!     &mut env_builder
//! ).unwrap();
//! let (mut env, triggers, _) = env_builder.into_runtime_parts().unwrap();
//! let mut sched = runtime::Scheduler::new(&mut env, triggers, true, false);
//! sched.event_loop();
//! ```
//!
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(unsafe_code)]
#![deny(clippy::all)]

pub mod builder;

// Re-exports
pub use boomerang_runtime as runtime;

#[cfg(feature = "derive")]
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
