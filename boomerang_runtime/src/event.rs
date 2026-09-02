use std::fmt::{Debug, Display};

use crate::{image::PortIndex, ActionKey, Duration, EnclaveKey, ReactorData, Tag};

/// Scheduler-owned destination for an asynchronously admitted value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncEventTarget {
    /// A normal asynchronous action destination.
    Action(ActionKey),
    /// A validated scheduler boundary port destination.
    BoundaryPort(PortIndex),
}

/// `AsyncEvent` is used to inject events into the scheduler from outside of the normal event loop.
pub enum AsyncEvent {
    /// A release event is used by upstream enclaves to signal that they have completed processing the tag.
    TagRelease {
        /// The key of the enclave that is releasing the `Tag``.
        enclave: EnclaveKey,
        /// The tag that is being released.
        tag: Tag,
    },
    /// An empty event is used by upstream enclaves to signal that they are ready to process the next event.
    TagReleaseProvisional {
        /// The key of the enclave that is waiting
        enclave: EnclaveKey,
        /// The tag that is being waited on.
        tag: Tag,
    },
    /// A Logical event has its `tag` set to the current logical time (+ an optional delay).
    Logical {
        /// The tag at which the Action should be executed
        tag: Tag,
        /// Scheduler destination for the admitted value.
        target: AsyncEventTarget,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// A Physical event has its `tag` set to the current physical time (+ an optional delay).
    Physical {
        /// The instant at which the Action should be executed
        time: std::time::Instant,
        /// Scheduler destination for the admitted value.
        target: AsyncEventTarget,
        /// The value associated with this event.
        value: Box<dyn ReactorData>,
    },

    /// The scheduler should terminate after processing this event.
    Shutdown {
        /// The delay after which the scheduler should terminate.
        delay: Duration,
    },
}

impl Debug for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TagRelease { enclave, tag } => f
                .debug_struct("TagRelease")
                .field("enclave", enclave)
                .field("tag", tag)
                .finish(),
            Self::TagReleaseProvisional { enclave, tag } => f
                .debug_struct("TagReleaseProvisional")
                .field("enclave", enclave)
                .field("tag", tag)
                .finish(),
            Self::Logical { tag, target, value } => f
                .debug_struct("Logical")
                .field("tag", tag)
                .field("target", target)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Physical {
                time,
                target,
                value,
            } => f
                .debug_struct("Physical")
                .field("time", time)
                .field("target", target)
                .field(
                    "value",
                    &format!("Box<{}>", std::any::type_name_of_val(&**value)),
                )
                .finish(),
            Self::Shutdown { delay } => f.debug_struct("Shutdown").field("delay", delay).finish(),
        }
    }
}

impl Display for AsyncEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncEvent::TagRelease { enclave, tag } => {
                write!(f, "TagRelease[enclave={enclave:?},tag={tag:.3}]")
            }
            AsyncEvent::TagReleaseProvisional { enclave, tag } => {
                write!(f, "TagReleaseProvisional[enclave={enclave:?},tag={tag:.3}]")
            }
            AsyncEvent::Logical {
                tag,
                target,
                value: _,
            } => {
                write!(f, "Logical[tag={tag:.3},target={target:?},value=..]",)
            }
            AsyncEvent::Physical {
                time,
                target,
                value: _,
            } => {
                write!(f, "Physical[tag={time:?},target={target:?},value=..]",)
            }
            AsyncEvent::Shutdown { delay } => {
                write!(f, "Shutdown[delay={delay:.3}]")
            }
        }
    }
}

impl AsyncEvent {
    /// Create a release event.
    pub(crate) fn release(enclave: EnclaveKey, tag: Tag) -> Self {
        AsyncEvent::TagRelease { enclave, tag }
    }

    /// Create a provisional release event.
    pub(crate) fn provisional(enclave: EnclaveKey, tag: Tag) -> Self {
        AsyncEvent::TagReleaseProvisional { enclave, tag }
    }

    /// Create a logical event.
    #[allow(dead_code)]
    pub(crate) fn logical(key: ActionKey, tag: Tag, value: Box<dyn ReactorData>) -> Self {
        AsyncEvent::Logical {
            tag,
            target: AsyncEventTarget::Action(key),
            value,
        }
    }

    /// Create a physical event.
    pub(crate) fn physical(
        key: ActionKey,
        time: std::time::Instant,
        value: Box<dyn ReactorData>,
    ) -> Self {
        AsyncEvent::Physical {
            time,
            target: AsyncEventTarget::Action(key),
            value,
        }
    }

    /// Create a shutdown event.
    pub(crate) fn shutdown(delay: Duration) -> Self {
        AsyncEvent::Shutdown { delay }
    }
}
