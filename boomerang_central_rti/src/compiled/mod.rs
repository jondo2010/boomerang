//! Compiled central-RTI execution over ordered transport interfaces.
//!
//! Dense keys are server-local bindings; transports identify members by their stable compiled
//! identity before dispatch. The in-memory adapters serve testing/reference execution.
//! Image dependencies are authoritative and are never recomputed from a topology.
//!
//! ## Coordination tracing
//!
//! Compiled execution emits optional structured [`tracing`] events at the
//! `boomerang::coordination` target. Every event uses the compiler-issued coordination
//! fingerprint and the owning typed Federate key; route, tag, and revision fields are included
//! when that event concerns them. The `event` field identifies a lifecycle action such as
//! `coordination.payload.sent` or `coordination.grant.received`.
//!
//! No event includes application payload bytes, stable display labels, or an error's diagnostic
//! text. Applications select off, bounded retention, or hosted export by configuring their
//! `tracing` subscriber; the runtime itself owns no trace buffer or exporter.
mod client;
pub mod hosted;
pub mod in_memory;
mod state;
#[cfg(test)]
mod tests;

use crate::WireTag;
use boomerang_runtime::image::{FederateIndex, RtiRouteIndex};
pub use client::{CentralRtiClient, RtiClientBindings};
pub use state::{CompiledRti, RtiResourceError};

/// Compiler-issued shared coordination digest, identical at the core and wire boundary.
pub use boomerang_federated::wire::CoordinationFingerprint as CoordinationIdentity;

/// Terminal compiled session or transport failure at a hosted interface boundary.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CentralRtiError {
    /// A compiled coordination storage budget could not be satisfied.
    #[error("central RTI: {0}")]
    Resource(#[from] RtiResourceError),
    /// A terminal coordination diagnostic independent of any transport implementation.
    #[error("central RTI: {0}")]
    Coordination(String),
    /// Original hosted failure shared with all observers of the terminated connection.
    #[error("central RTI: {0}")]
    Hosted(#[source] std::sync::Arc<hosted::HostedError>),
}
impl CentralRtiError {
    /// Records a coordination diagnostic at the core boundary.
    pub fn new(message: impl Into<String>) -> Self {
        Self::Coordination(message.into())
    }
}
impl From<hosted::HostedError> for CentralRtiError {
    fn from(error: hosted::HostedError) -> Self {
        Self::Hosted(std::sync::Arc::new(error))
    }
}

/// Owned upstream records in the compiler's original route-key domain.
pub type RtiRequest = boomerang_federated::wire::Request<RtiRouteIndex, Vec<u8>, String>;
/// Owned downstream records; their definition is shared with borrowed wire messages.
pub type RtiReply = boomerang_federated::wire::Reply<RtiRouteIndex, Vec<u8>, String>;

/// One server-local delivery selected by the compiled member domain.
#[derive(Clone, Debug)]
pub struct RtiDelivery {
    /// Recipient in the fingerprint-verified coordination image member domain.
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
