use super::*;
use boomerang_runtime::{
    image::{BoundaryId, CompiledDeploymentView, CoordinationProjection, RtiImage, RtiImageView},
    BoundarySubmissionError, CoordinationRevision, FederateAcquisition, FederateCompletion,
    FederateCoordinationBackend, FederateCoordinationError, FederatePublication,
    InboundBoundaryAdapter, OutboundBoundarySink, TaggedPayload,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tinymap::TinySecondaryMap;

/// Preflight bindings and shared reporting state for one validated coordination member.
///
/// Artifact generation must bind `identity` to this exact image. Admission verifies agreement
/// with the RTI before any scheduler starts; execution retains only the resolved typed bindings.
#[derive(Debug)]
pub struct RtiClientBindings<'a> {
    /// Validated projection descriptor owned during preflight; backing tables remain borrowed.
    pub(crate) image: RtiImage<'a>,
    /// Member owning the source and destination bindings being prepared.
    pub(crate) member: FederateIndex,
    /// Compiler-issued identity embedded alongside this image in the artifact.
    pub(crate) identity: CoordinationIdentity,
    /// Report suppression state shared with all outbound routes.
    pub(super) reports: Arc<Mutex<ReportState>>,
}
impl<'a> RtiClientBindings<'a> {
    /// Creates the parent span for this Federate's compiled execution.
    ///
    /// Enter it around binding installation and execution. Runtime workers inherit this
    /// context, correlating reaction and codec events with RTI events. When the coordination
    /// target is disabled, no identity formatting or subscriber storage is required.
    pub fn execution_span(&self) -> tracing::Span {
        tracing::debug_span!(
            target: "boomerang::coordination",
            "federate",
            coordination = self.identity.bytes().as_slice(),
            federate = self.member.as_u32(),
        )
    }

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
            image: image.clone(),
            member,
            identity,
            reports: Arc::default(),
        })
    }

    /// Consumes a validated RTI-only projection to select a member without retaining remote Enclaves.
    pub fn from_image(
        view: RtiImageView<'a>,
        member: FederateIndex,
        identity: CoordinationIdentity,
    ) -> Result<Self, CentralRtiError> {
        if view.members().get(member).is_none() {
            return Err(CentralRtiError::new("unknown compiled Federate key"));
        }
        Ok(Self {
            image: view.into_image(),
            member,
            identity,
            reports: Arc::default(),
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
        Ok(Arc::new(CompiledOutbound {
            sink,
            identity: self.identity,
            member: self.member,
            route,
            reports: self.reports.clone(),
        }))
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
    /// Opaque compiler-issued identity shared by every member of this coordination deployment.
    identity: CoordinationIdentity,
    /// Federate owning this client and its ordered RTI connection.
    member: FederateIndex,
    /// Shared DNET and latest suppressed publication.
    reports: Arc<Mutex<ReportState>>,
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
        let inbound = bindings.inbound(inbound);
        let mut client = Self {
            identity: bindings.identity,
            member: bindings.member,
            reports: bindings.reports,
            sink,
            source: Box::new(source),
            inbound: TinySecondaryMap::new(),
            timeout,
            idle: None,
            idle_request: None,
            failure: None,
        };
        let result = inbound
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
        if self.failure.is_none() {
            let message = error.to_string();
            tracing::event!(
                target: "boomerang::coordination",
                tracing::Level::ERROR,
                event = "coordination.failure.first",
                coordination = self.identity.bytes().as_slice(),
                federate = self.member.as_u32(),
            );
            self.failure = Some(message);
        }
        self.failure.clone().expect("first failure was recorded")
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
            Some(RtiReply::SuppressPublication { tag }) => {
                let mut reports = self
                    .reports
                    .lock()
                    .map_err(|_| CentralRtiError::new("report state lock poisoned"))?;
                reports.dnet = tag;
                tracing::debug!(
                    target: "boomerang::coordination",
                    event = "coordination.dnet.received",
                    coordination = self.identity.bytes().as_slice(),
                    federate = self.member.as_u32(),
                    tag_kind = tag.kind_str(),
                    tag_offset_ns = (tag).offset_ns(),
                    tag_microstep = (tag).microstep(),
                );
                if let Some((revision, next)) = reports.skipped {
                    if next > tag {
                        self.sink.send(RtiRequest::Publish {
                            revision,
                            next_event: Some(next),
                        })?;
                        tracing::debug!(
                            target: "boomerang::coordination",
                            event = "coordination.publication.restored",
                            coordination = self.identity.bytes().as_slice(),
                            federate = self.member.as_u32(),
                            revision,
                            next_event_kind = next.kind_str(),
                            next_event_offset_ns = (next).offset_ns(),
                            next_event_microstep = (next).microstep(),
                        );
                        reports.skipped = None;
                    }
                }
                Ok(None)
            }
            Some(RtiReply::Grant { revision, tag }) => {
                let tag = crate::tag_conversion::runtime_horizon_from_wire(tag)
                    .map_err(|e| CentralRtiError::new(e.to_string()))?;
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.grant.received",
                    coordination = self.identity.bytes().as_slice(),
                    federate = self.member.as_u32(),
                    revision,
                    tag_kind = tag.kind_str(),
                    tag_offset_ns = tag.offset().whole_nanoseconds(),
                    tag_microstep = tag.microstep(),
                );
                Ok(Some(FederateAcquisition::new(
                    CoordinationRevision::new(revision),
                    tag,
                )))
            }
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
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.payload.received",
                    federate = self.member.as_u32(),
                    coordination = self.identity.bytes().as_slice(),
                    route = route.as_u32(),
                    tag_kind = tag.kind_str(),
                    tag_offset_ns = tag.offset().whole_nanoseconds(),
                    tag_microstep = tag.microstep(),
                );
                if let Err(error) = adapter.admit(tag, &payload) {
                    tracing::event!(
                        target: "boomerang::coordination",
                        tracing::Level::WARN,
                        event = "coordination.boundary.rejected",
                        federate = self.member.as_u32(),
                        coordination = self.identity.bytes().as_slice(),
                        route = route.as_u32(),
                        tag_kind = tag.kind_str(),
                        tag_offset_ns = tag.offset().whole_nanoseconds(),
                        tag_microstep = tag.microstep(),
                        reason = error.kind_str(),
                    );
                    return Err(CentralRtiError::new(error.to_string()));
                }
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.boundary.admitted",
                    federate = self.member.as_u32(),
                    coordination = self.identity.bytes().as_slice(),
                    route = route.as_u32(),
                    tag_kind = tag.kind_str(),
                    tag_offset_ns = tag.offset().whole_nanoseconds(),
                    tag_microstep = tag.microstep(),
                );
                Ok(None)
            }
            Some(RtiReply::Idle { revision }) => {
                self.idle = Some(revision);
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.idle.received",
                    federate = self.member.as_u32(),
                    revision,
                    coordination = self.identity.bytes().as_slice(),
                );
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
                let mut reports = self
                    .reports
                    .lock()
                    .map_err(|_| CentralRtiError::new("report state lock poisoned"))?;
                let revision = publication.revision().value();
                if let Some(next) = next_event.filter(|next| *next <= reports.dnet) {
                    if publication
                        .next_event()
                        .zip(publication.grant_horizon())
                        .is_some_and(|(candidate, horizon)| candidate <= horizon)
                    {
                        reports.skipped = Some((revision, next));
                        tracing::debug!(
                            target: "boomerang::coordination",
                            event = "coordination.publication.suppressed",
                            coordination = self.identity.bytes().as_slice(),
                            federate = self.member.as_u32(),
                            revision,
                            next_event_kind = next.kind_str(),
                            next_event_offset_ns = (next).offset_ns(),
                            next_event_microstep = (next).microstep(),
                            dnet_kind = reports.dnet.kind_str(),
                            dnet_offset_ns = (reports.dnet).offset_ns(),
                            dnet_microstep = (reports.dnet).microstep(),
                        );
                        return Ok(());
                    }
                }
                self.sink.send(RtiRequest::Publish {
                    revision,
                    next_event,
                })?;
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.publication.sent",
                    federate = self.member.as_u32(),
                    revision,
                    coordination = self.identity.bytes().as_slice(),
                    next_event_kind = next_event.map_or("none", crate::WireTag::kind_str),
                    next_event_offset_ns = next_event.and_then(crate::WireTag::offset_ns),
                    next_event_microstep = next_event.and_then(crate::WireTag::microstep),
                );
                reports.skipped = None;
                Ok(())
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
        if !completion.confirms_network_input() {
            return Ok(());
        }
        let result = crate::wire_tag_from_runtime(completion.completed())
            .map_err(|e| CentralRtiError::new(e.to_string()))
            .and_then(|tag| {
                let mut reports = self
                    .reports
                    .lock()
                    .map_err(|_| CentralRtiError::new("report state lock poisoned"))?;
                self.sink.send(RtiRequest::Complete { tag })?;
                tracing::event!(
                    target: "boomerang::coordination",
                    tracing::Level::DEBUG,
                    event = "coordination.completion.sent",
                    federate = self.member.as_u32(),
                    coordination = self.identity.bytes().as_slice(),
                    tag_kind = completion.completed().kind_str(),
                    tag_offset_ns = completion.completed().offset().whole_nanoseconds(),
                    tag_microstep = completion.completed().microstep(),
                );
                if reports.skipped.is_some_and(|(_, next)| next <= tag) {
                    reports.skipped = None;
                }
                Ok(())
            });
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
        tracing::debug!(
            target: "boomerang::coordination",
            event = "coordination.shutdown.requested",
            coordination = self.identity.bytes().as_slice(),
            federate = self.member.as_u32(),
            graceful = self.failure.is_none() && self.idle.is_some(),
        );
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
                        Some(RtiReply::Stopped) => {
                            tracing::debug!(
                                target: "boomerang::coordination",
                                event = "coordination.shutdown.completed",
                                coordination = self.identity.bytes().as_slice(),
                                federate = self.member.as_u32(),
                            );
                            return Ok(());
                        }
                        Some(RtiReply::Failed { message }) => {
                            return Err(CentralRtiError::new(message))
                        }
                        Some(RtiReply::Idle { .. } | RtiReply::SuppressPublication { .. }) => {
                            continue
                        }
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
    /// Shared suppression decision serialized with outbound submission.
    reports: Arc<Mutex<ReportState>>,
    /// Shared ordered connection.
    sink: Arc<dyn RtiRequestSink>,
    /// Opaque compiler-issued identity shared by every member of this coordination deployment.
    identity: CoordinationIdentity,
    /// Federate that owns this compiled route binding.
    member: FederateIndex,
    /// Shared coordination route resolved and authorized during preflight.
    route: RtiRouteIndex,
}
impl OutboundBoundarySink for CompiledOutbound {
    fn send(&self, message: TaggedPayload) -> Result<(), BoundarySubmissionError> {
        let tag = crate::wire_tag_from_runtime(message.tag)
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))?;
        let mut reports = self
            .reports
            .lock()
            .map_err(|_| BoundarySubmissionError::new("report state lock poisoned"))?;
        reports.dnet = reports.dnet.min(tag);
        self.sink
            .send(RtiRequest::Payload {
                route: self.route,
                tag,
                payload: message.payload,
            })
            .map_err(|e| BoundarySubmissionError::new(e.to_string()))?;
        tracing::event!(
            target: "boomerang::coordination",
            tracing::Level::DEBUG,
            event = "coordination.payload.sent",
            coordination = self.identity.bytes().as_slice(),
            federate = self.member.as_u32(),
            route = self.route.as_u32(),
            tag_kind = message.tag.kind_str(),
            tag_offset_ns = message.tag.offset().whole_nanoseconds(),
            tag_microstep = message.tag.microstep(),
        );
        Ok(())
    }
}

/// Constant-space reporting state shared by the client and its route producers.
#[derive(Debug)]
pub(super) struct ReportState {
    /// Most recent downstream bound, tightened by locally submitted payloads.
    dnet: WireTag,
    /// Latest finite publication safely suppressed under accepted grant authority.
    skipped: Option<(u64, WireTag)>,
}
impl Default for ReportState {
    fn default() -> Self {
        Self {
            dnet: WireTag::NEVER,
            skipped: None,
        }
    }
}
