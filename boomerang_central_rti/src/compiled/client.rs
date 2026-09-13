use super::*;
use boomerang_runtime::{
    image::{BoundaryId, CompiledDeploymentView, CoordinationProjection, RtiImage},
    BoundarySubmissionError, CoordinationRevision, FederateAcquisition, FederateCompletion,
    FederateCoordinationBackend, FederateCoordinationError, FederatePublication,
    InboundBoundaryAdapter, OutboundBoundarySink, TaggedPayload,
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tinymap::TinySecondaryMap;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Immutable preflight context for one member of a validated coordination image.
///
/// Artifact generation must bind `identity` to this exact image. Admission verifies agreement
/// with the RTI before any scheduler starts; execution retains only the resolved typed bindings.
#[derive(Clone, Copy, Debug)]
pub struct RtiClientBindings<'a> {
    /// Validated mechanical coordination projection used only during preflight.
    image: RtiImage<'a>,
    /// Member owning the source and destination bindings being prepared.
    member: FederateIndex,
    /// Compiler-issued identity embedded alongside this image in the artifact.
    identity: CoordinationIdentity,
}
impl<'a> RtiClientBindings<'a> {
    /// Selects an existing member from a validated central-RTI deployment without retaining Enclaves.
    pub fn new(
        view: &CompiledDeploymentView<'a>,
        member: FederateIndex,
        identity: CoordinationIdentity,
    ) -> Result<Self, CentralRtiError> {
        let CoordinationProjection::CentralRti(image) = view.coordination() else {
            return Err(CentralRtiError::new(
                "compiled deployment does not select central-rti",
            ));
        };
        if view.federates().get(member).is_none() {
            return Err(CentralRtiError::new("unknown compiled Federate key"));
        }
        Ok(Self {
            image,
            member,
            identity,
        })
    }

    /// Resolves and authorizes an outbound boundary once, before any scheduler starts.
    pub fn outbound_sink(
        &self,
        sink: Arc<dyn RtiRequestSink>,
        boundary: BoundaryId<'_>,
    ) -> Result<Arc<dyn OutboundBoundarySink>, CentralRtiError> {
        let route = self
            .image
            .routes()
            .iter()
            .find_map(|(key, route)| {
                (route.source() == self.member && self.image.route_boundary(key) == boundary)
                    .then_some(key)
            })
            .ok_or_else(|| CentralRtiError::new("unknown outbound boundary for member"))?;
        Ok(Arc::new(CompiledOutbound { sink, route }))
    }

    /// Resolves exactly this member's incoming routes into the shared RTI key domain.
    fn inbound(
        &self,
        mut adapters: BTreeMap<BoundaryId<'a>, InboundBoundaryAdapter>,
    ) -> Result<TinySecondaryMap<RtiRouteIndex, InboundBoundaryAdapter>, CentralRtiError> {
        let mut inbound = TinySecondaryMap::new();
        for (key, route) in self.image.routes().iter() {
            if route.target() == self.member {
                let adapter = adapters
                    .remove(&self.image.route_boundary(key))
                    .ok_or_else(|| CentralRtiError::new("missing inbound boundary adapter"))?;
                inbound.insert(key, adapter);
            }
        }
        if !adapters.is_empty() {
            return Err(CentralRtiError::new("unknown inbound boundary for member"));
        }
        Ok(inbound)
    }
}

/// Compiled Federate adapter over a selected reliable ordered transport.
///
/// No transport thread or channel is created here. The connection owner supplies bounded
/// receive and nonblocking submission, shared with every outbound boundary producer.
pub struct CentralRtiClient {
    /// Ordered connection shared by payload and coordination producers.
    sink: Arc<dyn RtiRequestSink>,
    /// Exclusive receiver preserving payload-before-grant ordering.
    source: Box<dyn RtiReplySource>,
    /// Preflight mapping from shared RTI route keys to Enclave-local typed adapters.
    inbound: TinySecondaryMap<RtiRouteIndex, InboundBoundaryAdapter>,
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
    pub fn connect<'image>(
        sink: Arc<dyn RtiRequestSink>,
        source: impl RtiReplySource + 'static,
        bindings: RtiClientBindings<'image>,
        inbound: BTreeMap<BoundaryId<'image>, InboundBoundaryAdapter>,
        timeout: Duration,
    ) -> Result<Self, FederateCoordinationError> {
        let mut client = Self {
            sink,
            source: Box::new(source),
            inbound: TinySecondaryMap::new(),
            timeout,
            idle: None,
            idle_request: None,
            failure: None,
        };
        let result = bindings
            .inbound(inbound)
            .and_then(|inbound| {
                client.inbound = inbound;
                client.sink.send(RtiRequest::Hello {
                    identity: bindings.identity,
                })
            })
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
                route,
                tag,
                payload,
            }) => {
                let adapter = self
                    .inbound
                    .get(route)
                    .ok_or_else(|| CentralRtiError::new("unknown inbound RTI route key"))?;
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
    /// Shared coordination route resolved and authorized during preflight.
    route: RtiRouteIndex,
}
impl OutboundBoundarySink for CompiledOutbound {
    fn send(&self, message: TaggedPayload) -> Result<(), BoundarySubmissionError> {
        let tag = crate::wire_tag_from_runtime(message.tag)
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))?;
        self.sink
            .send(RtiRequest::Payload {
                route: self.route,
                tag,
                payload: message.payload,
            })
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))
    }
}
