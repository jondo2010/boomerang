#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

pub use ::time::Duration;

pub mod action;
pub mod binding;
mod context;
mod env;
mod event;
#[cfg(feature = "federated")]
mod federated;
pub mod image;
pub mod keepalive;
mod key_set;
pub mod port;
pub mod reaction;
mod reactor;
mod reference;
pub mod refs;
mod refs_extract;
#[cfg(feature = "replay")]
pub mod replay;
mod sched;
pub mod storage;
pub mod store;
mod time;

pub use action::{
    Action, ActionCommon, ActionKey, ActionRef, AsyncActionRef, BaseAction, DynActionRef,
    DynActionRefMut,
};
pub use context::*;
use downcast_rs::Downcast;
pub use env::{
    crosslink_enclaves, BankInfo, Enclave, EnclaveKey, Env, Level, LevelReactionKey,
    LifecycleReaction, ModalScheduleIndex, Mode, ModeFilter, ModeKey, ReactionGraph, ScopeInfo,
    ScopeKey, TransitionKind,
};
pub use event::{AsyncEvent, AsyncEventTarget};
#[cfg(feature = "federated")]
pub use federated::{
    FederatedEndpointError, FederatedFaultState, FederatedInboundEndpoint,
    FederatedOutboundCommand, FederatedOutboundMessage, FederatedOutboundSink,
    FederatedPayloadDecoder, FederatedPayloadEncoder,
};
pub use kanal::{Receiver, Sender};
pub use key_set::KeySetLimits as ReactionSetLimits;
pub use port::{DynPortRef, DynPortRefMut, *};
#[cfg(feature = "federated")]
pub use reaction::FederatedSenderReactionFn;
pub use reaction::{
    BoxedReactionFn, ConnectionReceiverReactionFn, ConnectionSenderReactionFn, Deadline,
    EnclaveSenderReactionFn, FromRefs, Reaction, ReactionFn, ReactionKey,
};
pub use reactor::*;
pub use reference::{
    execute_owned, execute_owned_federate, EnclaveExecution, ExecuteOwnedError,
    ExecuteOwnedFederateError, FederateBindings, FederateExecution, StateAccessError,
};
pub use refs::{Refs, RefsMut};
pub use refs_extract::{ReactionRefs, ReactionRefsError, ReactionRefsExtract};
pub use sched::*;
pub use storage::owned::{EnclaveBindings, OwnedStorage, OwnedStorageError, ReactionBindingError};
pub use time::*;

/// Types implementing this trait can be used as data in ports, actions, and reactors.
pub trait ReactorData: Downcast + Send + Sync + 'static {}

impl<T> ReactorData for T where T: Send + Sync + 'static {}

downcast_rs::impl_downcast!(ReactorData);

/// Zero-sized witness for the payload type exported by a direct port or action binding.
#[derive(Clone, Copy, Debug, Default)]
pub struct PayloadType<T: ReactorData>(std::marker::PhantomData<fn() -> T>);

impl<T: ReactorData> PayloadType<T> {
    /// Creates a payload type witness without constructing a payload value.
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(PortKey),

    /// Live graphs do not admit ordinary synchronous ports through the async channel.
    #[error("async boundary port target is not available in a live graph: {0}")]
    AsyncBoundaryPortUnsupported(image::PortIndex),

    /// Advancing a logical tag by a positive duration exceeded the tag range.
    #[error("logical tag {tag} cannot advance by {period} without overflowing")]
    LogicalTimeOverflow {
        /// Last representable logical tag reached by the scheduler.
        tag: Tag,
        /// Positive recurrence period that cannot be represented at `tag`.
        period: Duration,
    },

    #[error("Mismatched Dynamic Types found {found} but wanted {wanted}")]
    TypeMismatch {
        found: &'static str,
        wanted: &'static str,
    },

    #[error("Destructuring error")]
    DestrError,

    #[error("Encode error {error}")]
    EncodeError { error: String },

    #[error(transparent)]
    LogicalTimeBarrier(#[from] LogicalTimeBarrierError),

    #[cfg(feature = "replay")]
    #[error(transparent)]
    ReplayError(#[from] replay::ReplayError),

    #[cfg(feature = "federated")]
    #[error(transparent)]
    FederatedBarrier(#[from] FederatedBarrierError),
}

pub mod fmt_utils {
    //! Utility functions for formatting until [debug_closure_helpers](https://github.com/rust-lang/rust/issues/117729) lands in stable.
    pub fn from_fn<F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result>(f: F) -> FromFn<F> {
        FromFn(f)
    }

    pub struct FromFn<F>(F)
    where
        F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result;

    impl<F> std::fmt::Debug for FromFn<F>
    where
        F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result,
    {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            (self.0)(f)
        }
    }
}
