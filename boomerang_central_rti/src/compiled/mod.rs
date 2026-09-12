//! Compiled central-RTI execution over ordered transport interfaces.
//!
//! This module contains no in-memory transport. Dense keys are server-local bindings;
//! transports identify members by their stable compiled identity before dispatch.
//! Image dependencies are authoritative and are never recomputed from a topology.
mod client;
mod state;
#[cfg(test)]
mod tests;

use crate::WireTag;
use boomerang_runtime::image::FederateIndex;
pub use client::CentralRtiClient;
pub use state::CompiledRti;

/// Opaque compiler-issued identity shared by the RTI and every Federate artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoordinationIdentity([u8; 32]);
impl CoordinationIdentity {
    /// Embeds the same deployment coordination digest in all participating artifacts.
    pub const fn new(digest: [u8; 32]) -> Self {
        Self(digest)
    }
    /// Returns the digest for transport encoding.
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Terminal compiled session or transport failure at a hosted interface boundary.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("central RTI: {0}")]
pub struct CentralRtiError(String);
impl CentralRtiError {
    /// Retains an actionable diagnostic without exposing a transport implementation.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Ordered requests from one connection bound to a stable compiled Federate identity.
#[derive(Clone, Debug)]
pub enum RtiRequest {
    /// Admits this connection against the shared compiled coordination identity.
    Hello {
        /// Identity embedded in the connecting artifact.
        identity: CoordinationIdentity,
    },
    /// Publishes a reversible local candidate; `None` permits later inbound work.
    Publish {
        /// Revision owned by the compiled Federate coordinator.
        revision: u64,
        /// Current earliest local event, or local idle.
        next_event: Option<WireTag>,
    },
    /// Reports completion after all payload submissions at this tag.
    Complete {
        /// Greatest completed local tag.
        tag: WireTag,
    },
    /// Submits one encoded route value with delay already applied.
    Payload {
        /// Stable boundary identity; never an Enclave-local route ordinal.
        boundary: String,
        /// Final destination tag.
        tag: WireTag,
        /// Codec-produced bytes.
        payload: Vec<u8>,
    },
    /// Participates in terminal quiescence for a current local-idle revision.
    ConfirmIdle {
        /// Revision awaiting a final fixed-point check.
        revision: u64,
    },
    /// Commits the globally authorized idle stop.
    Stop,
    /// Fails the session after a local scheduler or transport failure.
    Abort {
        /// Diagnostic retained as the session failure cause.
        message: String,
    },
}

/// Ordered replies to a bound Federate connection.
#[derive(Clone, Debug)]
pub enum RtiReply {
    /// Every expected artifact passed identity admission.
    Started,
    /// Authorizes one publication after all preceding payloads were delivered.
    Grant {
        /// Publication revision being authorized.
        revision: u64,
        /// Logical execution horizon.
        tag: WireTag,
    },
    /// Delivers one payload before any grant that could execute it.
    Payload {
        /// Stable destination boundary binding.
        boundary: String,
        /// Final logical tag; no receiver-side delay is applied.
        tag: WireTag,
        /// Encoded application data.
        payload: Vec<u8>,
    },
    /// Confirms global quiescence for the named local publication.
    Idle {
        /// Locally idle revision for which no future inbound work remains.
        revision: u64,
    },
    /// Acknowledges terminal member stop.
    Stopped,
    /// Terminates all execution after a session failure.
    Failed {
        /// Original terminal failure diagnostic.
        message: String,
    },
}

/// One server-local delivery selected by the compiled member domain.
#[derive(Clone, Debug)]
pub struct RtiDelivery {
    /// Bound recipient; transport serialization must use its stable member identity.
    pub member: FederateIndex,
    /// Ordered reply for that recipient.
    pub reply: RtiReply,
}

/// Nonblocking reliable ordered submission shared by control and payload producers.
pub trait RtiRequestSink: Send + Sync {
    /// Accepts one request in the same ordering domain as every other request on this connection.
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError>;
}
/// Bounded receive interface implemented by the selected central-RTI transport.
pub trait RtiReplySource: Send {
    /// Receives in order, returning `None` on timeout and an error on connection loss.
    fn receive(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<RtiReply>, CentralRtiError>;
}
