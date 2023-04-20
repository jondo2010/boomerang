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
//! let reactor = HelloWorldBuilder::build(
//!     "hello_world",
//!     HelloWorld {
//!         success: false
//!     },
//!     None,
//!     &mut env_builder
//! ).unwrap();
//! let env = env_builder.try_into().unwrap();
//! let mut sched = runtime::Scheduler::new(env, true, false);
//! sched.event_loop();
//! ```
//!
//! # Crate features:
//! * **visualization** -
//!   Defaults on. Enables the debug Graphviz functions in [`builder::graphviz`].
//! * **derive** -
//!   Defaults on. Enables the derive macros in [`boomerang_derive`].

#[macro_use]
extern crate derivative;

pub mod builder;

// Re-exports
#[cfg(feature = "federated")]
pub use boomerang_federated as federated;

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

/// Wrapper around [`slotmap::SlotMap`] and [`slotmap::SecondaryMap`] that implements [`Debug`] as a map.
pub struct DebugMap<'a, T>(&'a T);

impl<'a, K, V> std::fmt::Debug for DebugMap<'a, slotmap::SecondaryMap<K, V>>
where
    K: slotmap::Key + std::fmt::Debug,
    V: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

impl<'a, K, V> std::fmt::Debug for DebugMap<'a, slotmap::SlotMap<K, V>>
where
    K: slotmap::Key + std::fmt::Debug,
    V: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}
