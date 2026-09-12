use super::*;
use boomerang_runtime::{
    image::BoundaryId, BoundarySubmissionError, CoordinationRevision, FederateAcquisition,
    FederateCompletion, FederateCoordinationBackend, FederateCoordinationError,
    FederatePublication, InboundBoundaryAdapter, OutboundBoundarySink, TaggedPayload,
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// Compiled Federate adapter over a selected reliable ordered transport.
///
/// No transport thread or channel is created here. The connection owner supplies bounded
/// receive and nonblocking submission, shared with every outbound boundary producer.
pub struct CentralRtiClient {
    /// Ordered connection shared by payload and coordination producers.
    sink: Arc<dyn RtiRequestSink>,
    /// Exclusive receiver preserving payload-before-grant ordering.
    source: Box<dyn RtiReplySource>,
    /// Prepared typed destination adapters resolved by stable boundary identity.
    inbound: BTreeMap<String, InboundBoundaryAdapter>,
    /// Maximum admission and stop acknowledgement wait.
    timeout: Duration,
    /// Latest globally authorized local idle revision.
    idle: Option<u64>,
    /// Last terminal quiescence probe, avoiding repeated transport traffic.
    idle_request: Option<u64>,
    /// First observed terminal backend failure.
    failure: Option<String>,
}
impl CentralRtiClient {
    /// Admits the artifact before any compiled scheduler starts.
    pub fn connect(
        sink: Arc<dyn RtiRequestSink>,
        source: impl RtiReplySource + 'static,
        identity: CoordinationIdentity,
        inbound: BTreeMap<BoundaryId<'_>, InboundBoundaryAdapter>,
        timeout: Duration,
    ) -> Result<Self, FederateCoordinationError> {
        let mut client = Self {
            sink,
            source: Box::new(source),
            inbound: inbound
                .into_iter()
                .map(|(id, adapter)| (id.as_str().to_owned(), adapter))
                .collect(),
            timeout,
            idle: None,
            idle_request: None,
            failure: None,
        };
        let result = client
            .sink
            .send(RtiRequest::Hello { identity })
            .and_then(|()| match client.source.receive(timeout)? {
                Some(RtiReply::Started) => Ok(()),
                Some(RtiReply::Failed { message }) => Err(CentralRtiError::new(message)),
                None => Err(CentralRtiError::new("admission timed out")),
                _ => Err(CentralRtiError::new("unexpected reply during admission")),
            });
        result.map_err(|error| {
            let message = error.to_string();
            let _ = client.sink.send(RtiRequest::Abort {
                message: message.clone(),
            });
            FederateCoordinationError::BackendAcquire { message }
        })?;
        Ok(client)
    }
    /// Binds a stable outbound boundary to the same ordered connection as publications.
    pub fn outbound_sink(
        sink: Arc<dyn RtiRequestSink>,
        boundary: BoundaryId<'_>,
    ) -> Arc<dyn OutboundBoundarySink> {
        Arc::new(CompiledOutbound {
            sink,
            boundary: boundary.as_str().to_owned(),
        })
    }
    /// Records the first failure so cleanup can release peers without replacing its cause.
    fn failed(&mut self, error: impl std::fmt::Display) -> String {
        self.failure
            .get_or_insert_with(|| error.to_string())
            .clone()
    }
    /// Receives one ordered reply and admits payloads before exposing subsequent grants.
    fn receive(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<FederateAcquisition>, CentralRtiError> {
        if let Some(message) = &self.failure {
            return Err(CentralRtiError::new(message.clone()));
        }
        match self.source.receive(timeout)? {
            None => Ok(None),
            Some(RtiReply::Grant { revision, tag }) => Ok(Some(FederateAcquisition::new(
                CoordinationRevision::new(revision),
                crate::runtime_tag_from_wire(tag)
                    .map_err(|e| CentralRtiError::new(e.to_string()))?,
            ))),
            Some(RtiReply::Payload {
                boundary,
                tag,
                payload,
            }) => {
                let adapter = self
                    .inbound
                    .get(&boundary)
                    .ok_or_else(|| CentralRtiError::new("unknown inbound boundary"))?;
                let tag = crate::runtime_tag_from_wire(tag)
                    .map_err(|e| CentralRtiError::new(e.to_string()))?;
                adapter
                    .admit(tag, &payload)
                    .map_err(|e| CentralRtiError::new(e.to_string()))?;
                Ok(None)
            }
            Some(RtiReply::Idle { revision }) => {
                self.idle = Some(revision);
                Ok(None)
            }
            Some(RtiReply::Failed { message }) => Err(CentralRtiError::new(message)),
            _ => Err(CentralRtiError::new("unexpected reply during execution")),
        }
    }
}
impl FederateCoordinationBackend for CentralRtiClient {
    fn publish(
        &mut self,
        publication: FederatePublication,
    ) -> Result<(), FederateCoordinationError> {
        let result = publication
            .next_event()
            .map(crate::wire_tag_from_runtime)
            .transpose()
            .map_err(|e| CentralRtiError::new(e.to_string()))
            .and_then(|next_event| {
                self.sink.send(RtiRequest::Publish {
                    revision: publication.revision().value(),
                    next_event,
                })
            });
        result.map_err(|error| FederateCoordinationError::BackendPublish {
            message: self.failed(error),
        })
    }
    fn progress(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
        self.receive(timeout)
            .map_err(|error| FederateCoordinationError::BackendAcquire {
                message: self.failed(error),
            })
    }
    fn complete(
        &mut self,
        completion: FederateCompletion,
    ) -> Result<(), FederateCoordinationError> {
        let result = crate::wire_tag_from_runtime(completion.completed())
            .map_err(|e| CentralRtiError::new(e.to_string()))
            .and_then(|tag| self.sink.send(RtiRequest::Complete { tag }));
        result.map_err(|error| FederateCoordinationError::BackendComplete {
            message: self.failed(error),
        })
    }
    fn confirm_idle(
        &mut self,
        revision: CoordinationRevision,
    ) -> Result<bool, FederateCoordinationError> {
        let revision = revision.value();
        if self.idle_request != Some(revision) {
            self.sink
                .send(RtiRequest::ConfirmIdle { revision })
                .map_err(|error| FederateCoordinationError::BackendStop {
                    message: self.failed(error),
                })?;
            self.idle_request = Some(revision);
        }
        Ok(self.idle == Some(revision))
    }
    fn stop(&mut self) -> Result<(), FederateCoordinationError> {
        let result = if self.failure.is_some() || self.idle.is_none() {
            let message = self
                .failure
                .clone()
                .unwrap_or_else(|| "compiled Federate stopped before global quiescence".into());
            let _ = self.sink.send(RtiRequest::Abort {
                message: message.clone(),
            });
            Err(CentralRtiError::new(message))
        } else {
            self.sink.send(RtiRequest::Stop).and_then(|()| {
                let started = Instant::now();
                loop {
                    let remaining = self.timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return Err(CentralRtiError::new("stop acknowledgement timed out"));
                    }
                    match self.source.receive(remaining)? {
                        Some(RtiReply::Stopped) => return Ok(()),
                        Some(RtiReply::Failed { message }) => {
                            return Err(CentralRtiError::new(message))
                        }
                        Some(RtiReply::Idle { .. }) => continue,
                        None => return Err(CentralRtiError::new("stop acknowledgement timed out")),
                        _ => return Err(CentralRtiError::new("unexpected reply during stop")),
                    }
                }
            })
        };
        result.map_err(|error| {
            let message = self.failed(error);
            let _ = self.sink.send(RtiRequest::Abort {
                message: message.clone(),
            });
            FederateCoordinationError::BackendStop { message }
        })
    }
}
/// Prebound route submission with no legacy action or graph-node adapter.
struct CompiledOutbound {
    /// Shared ordered connection.
    sink: Arc<dyn RtiRequestSink>,
    /// Stable compiled boundary identity.
    boundary: String,
}
impl OutboundBoundarySink for CompiledOutbound {
    fn send(&self, message: TaggedPayload) -> Result<(), BoundarySubmissionError> {
        let tag = crate::wire_tag_from_runtime(message.tag)
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))?;
        self.sink
            .send(RtiRequest::Payload {
                boundary: self.boundary.clone(),
                tag,
                payload: message.payload,
            })
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))
    }
}
