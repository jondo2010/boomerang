//! In-memory transport adapters for testing and reference execution, not the production hot path.
//!
//! These unbounded standard-library channels provide ordered in-process delivery. They do not
//! model network framing, backpressure, or cross-process authentication. The harness owns RTI
//! dispatch and lifetime, binding each sender to a member resolved from its stable identity.
use super::{CentralRtiError, RtiReply, RtiReplySource, RtiRequest, RtiRequestSink};
use boomerang_runtime::image::FederateIndex;
use std::{sync::mpsc, time::Duration};

/// Reference request sender bound to one RTI-local member.
pub struct InMemorySender {
    /// Member resolved by the harness before constructing this connection.
    member: FederateIndex,
    /// Shared request channel consumed by the harness's RTI worker.
    requests: mpsc::Sender<(FederateIndex, RtiRequest)>,
}
impl InMemorySender {
    /// Binds outgoing requests to a member in the receiving RTI's typed domain.
    pub fn new(member: FederateIndex, requests: mpsc::Sender<(FederateIndex, RtiRequest)>) -> Self {
        Self { member, requests }
    }
}
impl RtiRequestSink for InMemorySender {
    fn send(&self, request: RtiRequest) -> Result<(), CentralRtiError> {
        self.requests
            .send((self.member, request))
            .map_err(|error| CentralRtiError::new(error.to_string()))
    }
}
/// Reference receiver for one member's ordered replies.
pub struct InMemoryReceiver {
    /// Reply channel populated by the harness's RTI worker.
    replies: mpsc::Receiver<RtiReply>,
}
impl InMemoryReceiver {
    /// Wraps the reply channel for one connection.
    pub fn new(replies: mpsc::Receiver<RtiReply>) -> Self {
        Self { replies }
    }
}
impl RtiReplySource for InMemoryReceiver {
    fn receive(&mut self, timeout: Duration) -> Result<Option<RtiReply>, CentralRtiError> {
        match self.replies.recv_timeout(timeout) {
            Ok(reply) => Ok(Some(reply)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(error) => Err(CentralRtiError::new(error.to_string())),
        }
    }
}
