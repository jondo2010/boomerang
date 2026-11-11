//! Reactor state construction before logical startup.
//!
//! Generated adapters bind declared resources to user initializers after validation.
//! Input actions, source admission, clock control, and output observation are supplied by
//! their owning subsystems. Initialization does not own their execution protocols.
//!
//! [`StateInitContext`] uses compiled action rows and prepared owned storage.
//! The live builder uses a separate adapter with live action keys. Both validate
//! all declarations before calling user code and prepare resources before handing
//! capabilities to an initializer. Output observation binds only the actual state owner.
//! It does not include child effects or require source input or manual clock control.
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
    /// Output export was declared without hosted output support.
    #[error("output observer initialization requires the hosted-output feature")]
    OutputFeatureDisabled,
    /// State storage does not have one authoritative reactor owner.
    #[error("output observer initialization has no unique reactor owner")]
    InvalidOutputOwner,
    /// Output export cannot use the selected execution configuration.
    #[cfg(feature = "hosted-output")]
    #[error("output observer initialization is unavailable: {0}")]
    OutputUnavailable(crate::OutputWaitError),
}
