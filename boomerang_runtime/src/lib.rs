#![doc=include_str!( "../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
#![deny(clippy::all)]

pub use ::time::Duration;

pub mod action;
pub mod binding;
pub mod clock;
pub mod context;
mod env;
mod event;
pub mod image;
pub mod initialization;
#[cfg(feature = "hosted-source-input")]
pub mod input;
pub mod keepalive;
mod key_set;
mod observation;
pub mod output;
pub mod port;
mod progress;
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
pub use observation::{
    ObservationHandle, ObservationSnapshot, ObservationState, SchedulerLifecycle, SchedulerPhase,
};
mod boundary;
pub use boundary::{
    BoundaryAdmissionError, BoundarySubmissionError, InboundBoundaryAdapter, OutboundBoundarySink,
    PayloadCodecError, PayloadDecoder, PayloadEncoder, TaggedPayload,
};
pub use kanal::{Receiver, Sender};
pub use key_set::KeySetLimits as ReactionSetLimits;
pub use output::{OutputBound, OutputCompletion, OutputFault, OutputOutcome, OutputWaitError};
#[cfg(feature = "hosted-output")]
pub use output::{OutputObserver, OutputWait};
pub use port::{DynPortRef, DynPortRefMut, *};
pub use reaction::{
    BoxedReactionFn, ConnectionReceiverReactionFn, ConnectionSenderReactionFn, Deadline,
    EnclaveSenderReactionFn, FromRefs, Reaction, ReactionFn, ReactionKey,
};
pub use reactor::*;
pub use reference::{
    execute_owned, execute_owned_federate, EnclaveExecution, ExecuteOwnedError,
    ExecuteOwnedFederateError, FederateBindings, FederateExecution, StateAccessError,
};
pub use reference::{
    execute_owned_federate_slice, execute_owned_federate_slice_with_observations,
    execute_owned_federate_with_backend, execute_owned_federate_with_backend_and_observations,
    execute_owned_federate_with_observations,
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

/// A failure during reaction execution or runtime coordination.
#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    /// Fast-forward execution has no physical release rule for direct effects.
    #[error("external effects require paced execution; fast-forward is unsupported")]
    ExternalEffectFastForward,
    /// A callback returned an outcome inconsistent with its declared effect or failed.
    #[error("reaction {reaction} returned invalid or failed output {output:?}")]
    OutputContract {
        /// Executed reaction identity.
        reaction: ReactionKey,
        /// Actual result, including a possible partial device failure.
        output: OutputOutcome,
    },
    /// Physical-time or microstep conversion exceeded its supported range.
    #[error(transparent)]
    PhysicalClock(
        /// Clock or tag conversion failure.
        #[from]
        crate::clock::PhysicalClockError,
    ),
    /// The requested port does not exist in runtime storage.
    #[error("Port Key not found: {}", 0)]
    PortKeyNotFound(
        /// Missing runtime port key.
        PortKey,
    ),

    /// Live graphs do not admit ordinary synchronous ports through the async channel.
    #[error("async boundary port target is not available in a live graph: {0}")]
    AsyncBoundaryPortUnsupported(
        /// Compiled boundary port rejected by the live graph.
        image::PortIndex,
    ),

    /// Source admission requires compiled execution. A live graph cannot accept source batches.
    #[cfg(feature = "hosted-source-input")]
    #[error("source input is not available in a live graph")]
    SourceInputUnsupported,

    /// Advancing a logical tag by a positive duration exceeded the tag range.
    #[error("logical tag {tag} cannot advance by {period} without overflowing")]
    LogicalTimeOverflow {
        /// Last representable logical tag reached by the scheduler.
        tag: Tag,
        /// Positive recurrence period that cannot be represented at `tag`.
        period: Duration,
    },

    /// A stored value has a different type from the requested value.
    #[error("Mismatched Dynamic Types found {found} but wanted {wanted}")]
    TypeMismatch {
        /// Actual stored type name.
        found: &'static str,
        /// Required type name.
        wanted: &'static str,
    },

    /// Reaction references do not match the required field layout.
    #[error("Destructuring error")]
    DestrError,

    /// A payload encoder failed.
    #[error("Encode error {error}")]
    EncodeError {
        /// Description returned by the encoder.
        error: String,
    },

    /// The scheduler event channel closed during a wait for upstream permission.
    #[error("scheduler event channel closed while waiting for upstream enclave {upstream}")]
    UpstreamEventChannelClosed {
        /// Upstream Enclave whose permission the scheduler requires.
        upstream: EnclaveKey,
    },

    /// Event recording or replay failed.
    #[cfg(feature = "replay")]
    #[error(transparent)]
    ReplayError(
        /// Failure returned by the replay service.
        #[from]
        replay::ReplayError,
    ),
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
