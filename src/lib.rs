//! Boomerang is a framework for building and executing stateful, deterministic Reactors.
//!
//! ## Example
//!
//! Build and run a Reactor with reactions that respond to startup and shutdown actions:
//!
//! ```rust
//! use boomerang::{builder::*, runtime, Reactor};
//!
//! #[derive(Reactor)]
//! #[reactor(state = "HelloWorld")]
//! struct HelloWorldBuilder {
//!     #[reactor(reaction(function = "HelloWorld::reaction_startup"))]
//!     reaction_startup: BuilderReactionKey,
//!     #[reactor(reaction(function = "HelloWorld::reaction_shutdown"))]
//!     reaction_shutdown: BuilderReactionKey,
//! }
//!
//! #[derive(Clone)]
//! struct HelloWorld {
//!     success: bool,
//! }
//!
//! impl HelloWorld {
//!     #[boomerang::reaction(reactor = "HelloWorldBuilder", triggers(startup))]
//!     fn reaction_startup(&mut self, _ctx: &runtime::Context) {
//!         println!("Hello World.");
//!         self.success = true;
//!     }
//!
//!     #[boomerang::reaction(reactor = "HelloWorldBuilder", triggers(shutdown))]
//!     fn reaction_shutdown(&mut self, _ctx: &runtime::Context) {
//!         println!("Shutdown invoked.");
//!         assert!(self.success, "ERROR: startup reaction not executed.");
//!     }
//! }
//!
//! let mut env_builder = EnvBuilder::new();
//! let (reactor_key, reactor) = HelloWorldBuilder::build(
//!     "hello_world",
//!     HelloWorld {
//!         success: false
//!     },
//!     None,
//!     &mut env_builder
//! ).unwrap();
//! let (env, aliases) = env_builder.build_runtime(reactor_key).unwrap();
//! let mut sched = runtime::Scheduler::new(env, runtime::Config::default());
//! sched.event_loop();
//! ```
//!
//! ## Feature flags
#![doc = document_features::document_features!()]

#[cfg(feature = "runner")]
pub mod runner;

// Re-exports
pub use boomerang_builder as builder;
pub use boomerang_core as core;
#[cfg(feature = "federated")]
pub use boomerang_federated as federated;
pub use boomerang_runtime as runtime;

#[cfg(feature = "derive")]
#[allow(unused_imports)]
#[macro_use]
extern crate boomerang_derive;

#[cfg(feature = "derive")]
#[doc(hidden)]
pub use boomerang_derive::*;