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
#[cfg(feature = "federated")]
mod static_federation;

#[cfg(feature = "federated")]
pub use static_federation::{execute_federation_in_memory, execute_federation_over_tcp};

// Re-exports
pub use boomerang_builder as builder;
#[cfg(feature = "federated")]
pub use boomerang_federated as federated;
pub use boomerang_runtime as runtime;

pub mod prelude {
    //! Re-exported common types and traits for Boomerang

    pub use super::builder::{
        Assembly, AssemblyError, AssemblyFqn, AssemblyModeKey, AssemblyReactorKey, Contained,
        Input, Local, Logical, ModeEffectSpec, ModeKind, Output, Physical, PortBank, Reactor,
        ReactorPlacement, RuntimeAssembly, RuntimeExecution, RuntimeExecutionError, TimerActionKey,
        TimerSpec, TransitionKind, TypedActionKey, TypedPortKey,
    };

    #[cfg(feature = "federated")]
    pub use super::builder::FederateSpec;

    #[cfg(feature = "federated")]
    pub use super::{execute_federation_in_memory, execute_federation_over_tcp};

    #[cfg(feature = "federated")]
    pub use super::federated::{
        EndpointId, FederateId, RuntimeBridgeError, RuntimeFederate, RuntimeFederation,
        TcpStaticFederationConfig, WireDelay, WireTag,
    };

    pub use super::runtime::{self, action::ActionCommon, CommonContext, Duration, FromRefs, Tag};

    pub use boomerang_macros::{reaction, reactor, reactor_ports, timer};

    pub use crate::flatten_transposed::FlattenTransposedExt;
}

/// Top-level error type for Boomerang
#[derive(thiserror::Error, Debug)]
pub enum BoomerangError {
    #[error(transparent)]
    Assembly(#[from] builder::AssemblyError),

    #[error(transparent)]
    Runtime(#[from] runtime::RuntimeError),

    #[cfg(feature = "federated")]
    #[error(transparent)]
    StaticFederation(#[from] federated::StaticFederationRunnerError),

    #[cfg(feature = "federated")]
    #[error("static federation execution requires a lowered federation")]
    MissingStaticFederation,
}
