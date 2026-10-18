//! Reactor state construction before logical startup.
//!
//! Generated adapters bind declared resources to user initializers after validation.
//! Input actions, source admission, and clock control are capabilities supplied by
//! their owning subsystems. Initialization does not own their execution protocols.
//!
//! [`StateInitContext`] uses compiled action rows and prepared owned storage.
//! The live builder uses a separate adapter with live action keys. Both validate
//! all declarations before calling user code and prepare resources before handing
//! capabilities to an initializer.
//!
//! Initializers must return without waiting for logical execution. Keep workers in
//! cancelling guards, including workers created before an initializer fails.
//! Runtime cleanup closes resources before dropping successfully returned states.

mod compiled;
pub use compiled::{ActionCapability, StateCapabilities, StateInitContext};

#[doc(hidden)]
pub mod live;

/// User initializer errors preserve their concrete source chain.
pub type StateInitializationError = Box<dyn std::error::Error + Send + Sync>;

/// Invalid or unavailable typed initialization capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InitializationError {
    /// Reaction tables or their storage references do not match the live environment.
    #[error("invalid live reactor storage layout")]
    InvalidStorageLayout,
    /// A declared action, payload, source, or clock does not match the execution.
    #[error("invalid or unavailable initialization capabilities")]
    InvalidCapabilities,
}
