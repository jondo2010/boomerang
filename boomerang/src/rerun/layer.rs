use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::entities::{
    entity_path, RegistrationIndex, TraceId, TraceStateChange, TraceStateRecord, TraceTimePoint,
    TraceWriter, TraceWriterError,
};
use super::schema::{
    CausalLink, CausalOutcome, CausalState, DeliveryOutcome, IngressOutcome, OpenSpan,
    PropagationReceive, RawTraceFields, SchemaDiagnostic, TraceEvent, TraceRecord, TraceTag,
};
use super::session::SessionState;

const TRACE_TARGET: &str = "boomerang::trace";
const INTERNAL_TARGET: &str = "boomerang::rerun_internal";

#[derive(Clone)]
pub(super) struct SessionFilter {
    state: SessionState,
}

impl SessionFilter {
    pub(super) fn new(state: SessionState) -> Self {
        Self { state }
    }
}

impl<S> Filter<S> for SessionFilter {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: &Context<'_, S>) -> bool {
        metadata.target() == TRACE_TARGET && self.state.is_enabled()
    }

    fn callsite_enabled(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        if metadata.target() != TRACE_TARGET {
            tracing::subscriber::Interest::never()
        } else if self.state.is_enabled() {
            tracing::subscriber::Interest::sometimes()
        } else {
            tracing::subscriber::Interest::never()
        }
    }
}

/// A composable tracing layer that maps Boomerang's structured runtime facts to Rerun.
///
/// With an active memory, file, or tee sink, writes use Rerun 0.36.1's bounded batching pipeline
/// and may backpressure the scheduler callback under saturation. This layer deliberately owns no
/// second dynamic-record queue. The layer returned by [`RerunSession::layer`](super::RerunSession::layer)
/// uses a dynamic per-layer filter: after session failure it avoids trace metadata work when no
/// other interested layer is composed. It expresses no interest in unrelated targets, while
/// leaving other composed layers free to enable them.
#[derive(Clone)]
pub struct RerunLayer {
    recording: rerun::RecordingStream,
    state: SessionState,
    source_id: Arc<str>,
    started: Instant,
    writer: Arc<dyn TraceWriter>,
    adapter: AdapterState,
}

#[derive(Clone)]
pub(super) struct AdapterState {
    pub(super) next_id: Arc<AtomicU64>,
    pub(super) registration: Arc<RwLock<RegistrationIndex>>,
    correlation: Arc<Mutex<CorrelationState>>,
}

impl Default for AdapterState {
    fn default() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            registration: Arc::new(RwLock::new(RegistrationIndex::default())),
            correlation: Arc::new(Mutex::new(CorrelationState::default())),
        }
    }
}

impl RerunLayer {
    pub(super) fn new(
        recording: rerun::RecordingStream,
        state: SessionState,
        source_id: Arc<str>,
        writer: Arc<dyn TraceWriter>,
        started: Instant,
        adapter: AdapterState,
    ) -> Self {
        Self {
            recording,
            state,
            source_id,
            started,
            writer,
            adapter,
        }
    }

    fn next_id(&self, enclave: &str) -> Option<TraceId> {
        match self
            .adapter
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sequence| {
                sequence.checked_add(1)
            }) {
            Ok(sequence) => Some(TraceId::new(&self.source_id, enclave, sequence)),
            Err(_) => {
                self.disable_safely(&"Rerun trace ID sequence exhausted");
                None
            }
        }
    }

    fn timepoint(&self, tag: Option<TraceTag>) -> TraceTimePoint {
        TraceTimePoint {
            elapsed_ns: saturating_i64(self.started.elapsed().as_nanos()),
            wall_clock_unix_ns: saturating_i64(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            ),
            logical_ns: tag.and_then(|value| i64::try_from(value.logical_ns).ok()),
        }
    }

    fn write(&self, record: TraceRecord) {
        if !self.state.try_begin_attempt() {
            return;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.writer.write(&self.recording, &record)
        }))
        .unwrap_or_else(|panic| Err(panic_error("trace writer", panic.as_ref())));
        if let Err(error) = result {
            self.disable_safely(&error);
        }
    }

    fn write_state(&self, record: TraceStateRecord) {
        if !self.state.try_begin_attempt() {
            return;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.writer.write_state(&self.recording, &record)
        }))
        .unwrap_or_else(|panic| Err(panic_error("trace writer", panic.as_ref())));
        if let Err(error) = result {
            self.disable_safely(&error);
        }
    }

    fn observe_callback(&self, callback: impl FnOnce()) {
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
            self.disable_safely(&panic_error("trace callback", panic.as_ref()));
        }
    }

    fn disable_safely(&self, error: &dyn fmt::Display) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.state.disable_on_error(error);
        }));
    }

    fn diagnostic(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::warn!(target: INTERNAL_TARGET, error = %message, "invalid Boomerang trace record");
        let Some(id) = self.next_id("unknown") else {
            return;
        };
        self.write(TraceRecord {
            id,
            parent_id: None,
            entity_path: "/diagnostics/schema".to_owned(),
            timepoint: self.timepoint(None),
            duration_ns: None,
            terminal_state: None,
            event: TraceEvent::SchemaDiagnostic(SchemaDiagnostic { error: message }),
        });
    }

    fn make_record(
        &self,
        event: TraceEvent,
        parent_id: Option<TraceId>,
        id: Option<TraceId>,
        duration: Option<Duration>,
    ) -> Option<TraceRecord> {
        let enclave = event.enclave().unwrap_or("unknown");
        let id = if let Some(id) = id {
            id
        } else {
            self.next_id(enclave)?
        };
        let entity_path = self.resolved_entity_path(&event);
        let entity_path = if matches!(
            event,
            TraceEvent::PropagationLogicalSend(_)
                | TraceEvent::PropagationPhysicalSend(_)
                | TraceEvent::PropagationSerializedSend(_)
        ) {
            format!(
                "/propagation/sends/{}",
                super::entities::escape_entity_segment(&id.0)
            )
        } else {
            entity_path
        };
        Some(TraceRecord {
            id,
            parent_id,
            entity_path,
            timepoint: self.timepoint(event.tag()),
            duration_ns: duration.map(|value| u64::try_from(value.as_nanos()).unwrap_or(u64::MAX)),
            terminal_state: None,
            event,
        })
    }

    fn resolved_entity_path(&self, event: &TraceEvent) -> String {
        self.adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entity_path(event)
            .unwrap_or_else(|| entity_path(event))
    }

    fn write_with_causality(&self, record: TraceRecord) {
        let derived = self.derive_causality(&record);
        self.write(record);
        for record in derived {
            self.write(record);
        }
    }

    fn derive_causality(&self, record: &TraceRecord) -> Vec<TraceRecord> {
        let topology = {
            let registration = self
                .adapter
                .registration
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &record.event {
                TraceEvent::PropagationLogicalSend(_)
                | TraceEvent::PropagationSerializedSend(_)
                | TraceEvent::LogicalIngress(_)
                | TraceEvent::ActionRebased(_) => {
                    registration.resolve_entity(&record.event).map(|action| {
                        let reactions = registration.triggered_reactions(&action).to_vec();
                        (action, reactions)
                    })
                }
                TraceEvent::ReactionExecution(_) => registration
                    .resolve_entity(&record.event)
                    .map(|reaction| (reaction, Vec::new())),
                _ => None,
            }
        };

        let mut correlation = lock_unpoisoned(&self.adapter.correlation);
        correlation.advance();
        match &record.event {
            TraceEvent::PropagationLogicalSend(send) => {
                if let Some((action, _)) = topology {
                    let key = PropagationKey {
                        federate: send.federate.clone(),
                        action,
                        tag: send.tag.into(),
                    };
                    let accepted = send.outcome == DeliveryOutcome::Accepted;
                    match correlation.finish_open_send(&key, &record.id, accepted) {
                        FinishOpenSend::NotOpen if accepted => {
                            correlation.insert_send(key, record.id.clone());
                        }
                        FinishOpenSend::EarlyIngress(ingress) => {
                            return self.derive_receive(
                                &mut correlation,
                                record.id.clone(),
                                *ingress,
                            );
                        }
                        FinishOpenSend::NotOpen | FinishOpenSend::Handled => {}
                    }
                }
                Vec::new()
            }
            TraceEvent::PropagationSerializedSend(send) => {
                if let Some((action, _)) = topology {
                    let key = PropagationKey {
                        federate: send
                            .destination_federate
                            .clone()
                            .or_else(|| send.federate.clone()),
                        action,
                        tag: send.tag.into(),
                    };
                    let accepted = send.outcome == DeliveryOutcome::Accepted;
                    match correlation.finish_open_send(&key, &record.id, accepted) {
                        FinishOpenSend::NotOpen if accepted => {
                            correlation.insert_send(key, record.id.clone())
                        }
                        FinishOpenSend::EarlyIngress(ingress) => {
                            return self.derive_receive(
                                &mut correlation,
                                record.id.clone(),
                                *ingress,
                            )
                        }
                        FinishOpenSend::NotOpen | FinishOpenSend::Handled => {}
                    }
                }
                Vec::new()
            }
            TraceEvent::LogicalIngress(ingress) if ingress.outcome == IngressOutcome::Accepted => {
                let Some((action, _)) = topology else {
                    return Vec::new();
                };
                let key = PropagationKey {
                    federate: ingress.federate.clone(),
                    action,
                    tag: ingress.destination_tag.into(),
                };
                if let Some(send) = correlation.take_send(&key) {
                    return self.derive_receive(&mut correlation, send, record.clone());
                }
                if correlation.capture_early_ingress(key, record.clone()) {
                    return Vec::new();
                }
                Vec::new()
            }
            TraceEvent::ActionRebased(rebased) => {
                if let (Some((_, reactions)), Some(enclave)) =
                    (topology, rebased.enclave.as_deref())
                {
                    correlation.rebase_predecessors(
                        rebased.federate.as_deref(),
                        enclave,
                        &reactions,
                        &rebased.old_tag.into(),
                        &rebased.destination_tag.into(),
                    );
                }
                Vec::new()
            }
            TraceEvent::ReactionExecution(reaction_event) => {
                let Some((reaction, _)) = topology else {
                    return Vec::new();
                };
                let enclave = reaction_event.enclave.clone();
                correlation
                    .take_predecessors(&ReactionKey {
                        federate: reaction_event.federate.clone(),
                        enclave: enclave.clone(),
                        reaction,
                        tag: reaction_event.tag.into(),
                    })
                    .into_iter()
                    .filter_map(|receive| self.causal_link(&enclave, &receive, &record.id, record))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn derive_receive(
        &self,
        correlation: &mut CorrelationState,
        send: TraceId,
        ingress: TraceRecord,
    ) -> Vec<TraceRecord> {
        let TraceEvent::LogicalIngress(value) = &ingress.event else {
            return Vec::new();
        };
        let enclave = value.enclave.clone();
        let correlation_tag: CompleteTag = value.destination_tag.into();
        let reactions = {
            let registration = self
                .adapter
                .registration
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration
                .resolve_entity(&ingress.event)
                .map(|action| registration.triggered_reactions(&action).to_vec())
                .unwrap_or_default()
        };
        let Some(receive_id) = self.next_id(&enclave) else {
            return Vec::new();
        };
        let receive = TraceRecord {
            entity_path: format!(
                "/propagation/receives/{}",
                super::entities::escape_entity_segment(&receive_id.0)
            ),
            id: receive_id.clone(),
            parent_id: Some(send.clone()),
            timepoint: ingress.timepoint.clone(),
            duration_ns: None,
            terminal_state: None,
            event: TraceEvent::PropagationReceive(PropagationReceive {
                federate: value.federate.clone(),
                enclave: value.enclave.clone(),
                action_key: value.action_key.clone(),
                action: value.action.clone(),
                tag: value.tag,
                destination_tag: value.destination_tag,
                value: value.value.clone(),
                outcome: value.outcome,
            }),
        };
        for reaction in reactions {
            correlation.insert_predecessor(
                ReactionKey {
                    federate: value.federate.clone(),
                    enclave: enclave.clone(),
                    reaction,
                    tag: correlation_tag.clone(),
                },
                receive_id.clone(),
            );
        }
        let mut derived = vec![receive];
        if let Some(link) = self.causal_link(&enclave, &send, &receive_id, &ingress) {
            derived.push(link);
        }
        derived
    }

    fn begin_propagation_send(&self, event: &TraceEvent, id: TraceId) {
        let (federate, tag) = match event {
            TraceEvent::PropagationLogicalSend(send) => (send.federate.clone(), send.tag),
            TraceEvent::PropagationSerializedSend(send) => (
                send.destination_federate
                    .clone()
                    .or_else(|| send.federate.clone()),
                send.tag,
            ),
            _ => return,
        };
        let action = self
            .adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resolve_entity(event);
        if let Some(action) = action {
            lock_unpoisoned(&self.adapter.correlation).begin_open_send(
                PropagationKey {
                    federate,
                    action,
                    tag: tag.into(),
                },
                id,
            );
        }
    }

    fn abort_propagation_send(&self, event: &TraceEvent, id: &TraceId) {
        let (federate, tag) = match event {
            TraceEvent::PropagationLogicalSend(send) => (send.federate.clone(), send.tag),
            TraceEvent::PropagationSerializedSend(send) => (
                send.destination_federate
                    .clone()
                    .or_else(|| send.federate.clone()),
                send.tag,
            ),
            _ => return,
        };
        let action = self
            .adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resolve_entity(event);
        if let Some(action) = action {
            lock_unpoisoned(&self.adapter.correlation).abort_open_send(
                &PropagationKey {
                    federate,
                    action,
                    tag: tag.into(),
                },
                id,
            );
        }
    }

    fn causal_link(
        &self,
        enclave: &str,
        source: &TraceId,
        destination: &TraceId,
        at: &TraceRecord,
    ) -> Option<TraceRecord> {
        let id = self.next_id(enclave)?;
        let tag = at.event.tag()?;
        Some(TraceRecord {
            entity_path: format!(
                "/propagation/links/{}",
                super::entities::escape_entity_segment(&id.0)
            ),
            id,
            parent_id: Some(source.clone()),
            timepoint: at.timepoint.clone(),
            duration_ns: None,
            terminal_state: None,
            event: TraceEvent::CausalLink(CausalLink {
                enclave: enclave.to_owned(),
                source: source.clone(),
                destination: destination.clone(),
                tag,
                state: CausalState::Exact,
                outcome: CausalOutcome::Matched,
            }),
        })
    }
}

const MAX_PENDING_CORRELATIONS: usize = 4096;
const MAX_CORRELATION_AGE: u64 = 4096;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CompleteTag {
    logical_ns: u64,
    microstep: u64,
}

impl From<TraceTag> for CompleteTag {
    fn from(tag: TraceTag) -> Self {
        Self {
            logical_ns: tag.logical_ns,
            microstep: tag.microstep,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PropagationKey {
    federate: Option<String>,
    action: String,
    tag: CompleteTag,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReactionKey {
    federate: Option<String>,
    enclave: String,
    reaction: String,
    tag: CompleteTag,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PropagationBucketKey {
    federate: Option<String>,
    action: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReactionBucketKey {
    federate: Option<String>,
    enclave: String,
    reaction: String,
}

#[derive(Clone, Debug)]
enum PendingResolution {
    Unique { id: TraceId, epoch: u64 },
    Ambiguous { epoch: u64 },
}

impl PendingResolution {
    fn epoch(&self) -> u64 {
        match self {
            Self::Unique { epoch, .. } | Self::Ambiguous { epoch } => *epoch,
        }
    }

    fn unique_id(self) -> Option<TraceId> {
        match self {
            Self::Unique { id, .. } => Some(id),
            Self::Ambiguous { .. } => None,
        }
    }
}

#[derive(Default)]
pub(super) struct CorrelationState {
    epoch: u64,
    next_generation: u64,
    pending_count: usize,
    eviction: VecDeque<EvictionToken>,
    generation_eviction: VecDeque<TagGenerationToken>,
    propagation_generations: HashMap<PropagationBucketKey, VecDeque<TagGeneration>>,
    reaction_generations: HashMap<ReactionBucketKey, VecDeque<TagGeneration>>,
    sends: HashMap<PropagationKey, PendingResolution>,
    open_sends: HashMap<PropagationKey, PendingResolution>,
    early_ingress: HashMap<PropagationKey, PendingIngress>,
    predecessors: HashMap<ReactionKey, Vec<PendingPredecessor>>,
}

#[derive(Clone, Debug)]
struct TagGeneration {
    tag: CompleteTag,
    generation: u64,
}

#[derive(Clone, Debug)]
struct TagGenerationToken {
    epoch: u64,
    key: TagGenerationKey,
    tag: CompleteTag,
    generation: u64,
}

#[derive(Clone, Debug)]
enum TagGenerationKey {
    Propagation(PropagationBucketKey),
    Reaction(ReactionBucketKey),
}

#[derive(Clone, Debug)]
struct EvictionToken {
    epoch: u64,
    key: EvictionKey,
}

#[derive(Clone, Debug)]
enum EvictionKey {
    Send(PropagationKey),
    OpenSend(PropagationKey),
    EarlyIngress(PropagationKey),
    Predecessor(ReactionKey, TraceId),
}

#[derive(Clone, Debug)]
enum PendingIngress {
    Unique {
        record: Box<TraceRecord>,
        epoch: u64,
    },
    Ambiguous {
        epoch: u64,
    },
}

impl PendingIngress {
    fn epoch(&self) -> u64 {
        match self {
            Self::Unique { epoch, .. } | Self::Ambiguous { epoch } => *epoch,
        }
    }
}

enum FinishOpenSend {
    NotOpen,
    EarlyIngress(Box<TraceRecord>),
    Handled,
}

#[derive(Clone, Debug)]
struct PendingPredecessor {
    id: TraceId,
    epoch: u64,
}

impl CorrelationState {
    fn advance(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
        let minimum = self.epoch.saturating_sub(MAX_CORRELATION_AGE);
        while self
            .eviction
            .front()
            .is_some_and(|token| token.epoch < minimum)
        {
            self.evict_front();
        }
        while self
            .generation_eviction
            .front()
            .is_some_and(|token| token.epoch < minimum)
        {
            self.expire_generation();
        }
    }

    fn insert_send(&mut self, key: PropagationKey, id: TraceId) {
        self.cleanup_older_propagations(&key);
        if self.open_sends.contains_key(&key) {
            self.poison_send_key(key);
        } else {
            let inserted = insert_resolution(&mut self.sends, key.clone(), id, self.epoch);
            self.track(EvictionKey::Send(key), inserted);
        }
        self.enforce_bound();
    }

    fn begin_open_send(&mut self, key: PropagationKey, id: TraceId) {
        self.cleanup_older_propagations(&key);
        if self.sends.contains_key(&key) {
            self.poison_send_key(key);
        } else {
            let inserted = insert_resolution(&mut self.open_sends, key.clone(), id, self.epoch);
            self.track(EvictionKey::OpenSend(key), inserted);
        }
        self.enforce_bound();
    }

    fn poison_send_key(&mut self, key: PropagationKey) {
        let send_inserted = self
            .sends
            .insert(
                key.clone(),
                PendingResolution::Ambiguous { epoch: self.epoch },
            )
            .is_none();
        self.track(EvictionKey::Send(key.clone()), send_inserted);
        let open_inserted = self
            .open_sends
            .insert(
                key.clone(),
                PendingResolution::Ambiguous { epoch: self.epoch },
            )
            .is_none();
        self.track(EvictionKey::OpenSend(key), open_inserted);
    }

    fn capture_early_ingress(&mut self, key: PropagationKey, record: TraceRecord) -> bool {
        self.cleanup_older_propagations(&key);
        if !self.open_sends.contains_key(&key) {
            return false;
        }
        let inserted = !self.early_ingress.contains_key(&key);
        self.early_ingress
            .entry(key.clone())
            .and_modify(|ingress| *ingress = PendingIngress::Ambiguous { epoch: self.epoch })
            .or_insert(PendingIngress::Unique {
                record: Box::new(record),
                epoch: self.epoch,
            });
        self.track(EvictionKey::EarlyIngress(key), inserted);
        self.enforce_bound();
        true
    }

    fn finish_open_send(
        &mut self,
        key: &PropagationKey,
        id: &TraceId,
        accepted: bool,
    ) -> FinishOpenSend {
        self.cleanup_older_propagations(key);
        match self.open_sends.get(key) {
            None => FinishOpenSend::NotOpen,
            Some(PendingResolution::Ambiguous { .. }) => FinishOpenSend::Handled,
            Some(PendingResolution::Unique { id: pending, .. }) if pending != id => {
                FinishOpenSend::Handled
            }
            Some(PendingResolution::Unique { .. }) => {
                self.open_sends.remove(key);
                self.pending_count = self.pending_count.saturating_sub(1);
                let ingress = self.early_ingress.remove(key);
                if ingress.is_some() {
                    self.pending_count = self.pending_count.saturating_sub(1);
                }
                if !accepted {
                    return FinishOpenSend::Handled;
                }
                match ingress {
                    Some(PendingIngress::Unique { record, .. }) => {
                        FinishOpenSend::EarlyIngress(record)
                    }
                    Some(PendingIngress::Ambiguous { .. }) => {
                        let inserted = self
                            .sends
                            .insert(
                                key.clone(),
                                PendingResolution::Ambiguous { epoch: self.epoch },
                            )
                            .is_none();
                        self.track(EvictionKey::Send(key.clone()), inserted);
                        FinishOpenSend::Handled
                    }
                    None => {
                        self.insert_send(key.clone(), id.clone());
                        FinishOpenSend::Handled
                    }
                }
            }
        }
    }

    fn abort_open_send(&mut self, key: &PropagationKey, id: &TraceId) {
        let remove = matches!(self.open_sends.get(key), Some(PendingResolution::Unique { id: pending, .. }) if pending == id);
        if remove {
            self.open_sends.remove(key);
            self.pending_count = self.pending_count.saturating_sub(1);
            if self.early_ingress.remove(key).is_some() {
                self.pending_count = self.pending_count.saturating_sub(1);
            }
        }
    }

    fn take_send(&mut self, key: &PropagationKey) -> Option<TraceId> {
        self.cleanup_older_propagations(key);
        if self.open_sends.contains_key(key) && self.sends.contains_key(key) {
            self.poison_send_key(key.clone());
            return None;
        }
        if self.open_sends.contains_key(key) {
            return None;
        }
        match self.sends.get(key) {
            Some(PendingResolution::Unique { .. }) => {
                let resolution = self.sends.remove(key);
                self.pending_count = self.pending_count.saturating_sub(1);
                resolution.and_then(PendingResolution::unique_id)
            }
            Some(PendingResolution::Ambiguous { .. }) | None => None,
        }
    }

    fn insert_predecessor(&mut self, key: ReactionKey, id: TraceId) {
        self.cleanup_older_reactions(&key);
        let inserted = {
            let predecessors = self.predecessors.entry(key.clone()).or_default();
            if predecessors.iter().any(|predecessor| predecessor.id == id) {
                false
            } else {
                predecessors.push(PendingPredecessor {
                    id: id.clone(),
                    epoch: self.epoch,
                });
                true
            }
        };
        if inserted {
            self.track(EvictionKey::Predecessor(key, id), true);
        }
        self.enforce_bound();
    }

    fn take_predecessors(&mut self, key: &ReactionKey) -> Vec<TraceId> {
        self.cleanup_older_reactions(key);
        let predecessors = self.predecessors.remove(key).unwrap_or_default();
        self.pending_count = self.pending_count.saturating_sub(predecessors.len());
        predecessors
            .into_iter()
            .map(|predecessor| predecessor.id)
            .collect()
    }

    fn rebase_predecessors(
        &mut self,
        federate: Option<&str>,
        enclave: &str,
        reactions: &[String],
        old_tag: &CompleteTag,
        destination_tag: &CompleteTag,
    ) {
        if old_tag == destination_tag {
            return;
        }
        for reaction in reactions {
            let old_key = ReactionKey {
                federate: federate.map(str::to_owned),
                enclave: enclave.to_owned(),
                reaction: reaction.clone(),
                tag: old_tag.clone(),
            };
            let Some(moved) = self.predecessors.remove(&old_key) else {
                continue;
            };
            let destination_key = ReactionKey {
                federate: federate.map(str::to_owned),
                enclave: enclave.to_owned(),
                reaction: reaction.clone(),
                tag: destination_tag.clone(),
            };
            self.cleanup_older_reactions(&destination_key);
            let destination = self.predecessors.entry(destination_key.clone());
            let predecessors = destination.or_default();
            let mut tracked = Vec::new();
            for predecessor in moved {
                if !predecessors
                    .iter()
                    .any(|existing| existing.id == predecessor.id)
                {
                    tracked.push(EvictionKey::Predecessor(
                        destination_key.clone(),
                        predecessor.id.clone(),
                    ));
                    predecessors.push(predecessor);
                } else {
                    self.pending_count = self.pending_count.saturating_sub(1);
                }
            }
            for key in tracked {
                self.track(key, false);
            }
        }
    }

    fn enforce_bound(&mut self) {
        while self.pending_count > MAX_PENDING_CORRELATIONS && !self.eviction.is_empty() {
            self.evict_front();
        }
    }

    fn track(&mut self, key: EvictionKey, inserted: bool) {
        if inserted {
            self.pending_count += 1;
        }
        self.eviction.push_back(EvictionToken {
            epoch: self.epoch,
            key: key.clone(),
        });
        self.ensure_generation(&key);
    }

    fn evict_front(&mut self) {
        let Some(token) = self.eviction.pop_front() else {
            return;
        };
        let removed = match token.key {
            EvictionKey::Send(key) => remove_resolution_at(&mut self.sends, &key, token.epoch),
            EvictionKey::OpenSend(key) => {
                let removed = remove_resolution_at(&mut self.open_sends, &key, token.epoch);
                if removed && self.early_ingress.remove(&key).is_some() {
                    self.pending_count = self.pending_count.saturating_sub(1);
                }
                removed
            }
            EvictionKey::EarlyIngress(key) => {
                if self
                    .early_ingress
                    .get(&key)
                    .is_some_and(|value| value.epoch() == token.epoch)
                {
                    self.early_ingress.remove(&key);
                    true
                } else {
                    false
                }
            }
            EvictionKey::Predecessor(key, id) => {
                remove_predecessor_at(&mut self.predecessors, &key, &id, token.epoch)
            }
        };
        if removed {
            self.pending_count = self.pending_count.saturating_sub(1);
        }
    }

    fn cleanup_older_propagations(&mut self, key: &PropagationKey) {
        let bucket = PropagationBucketKey {
            federate: key.federate.clone(),
            action: key.action.clone(),
        };
        let (obsolete, empty) =
            pop_older_generations(self.propagation_generations.get_mut(&bucket), &key.tag);
        if empty {
            self.propagation_generations.remove(&bucket);
        }
        for tag in obsolete {
            self.remove_propagation(&PropagationKey {
                federate: bucket.federate.clone(),
                action: bucket.action.clone(),
                tag,
            });
        }
    }

    fn cleanup_older_reactions(&mut self, key: &ReactionKey) {
        let bucket = ReactionBucketKey {
            federate: key.federate.clone(),
            enclave: key.enclave.clone(),
            reaction: key.reaction.clone(),
        };
        let (obsolete, empty) =
            pop_older_generations(self.reaction_generations.get_mut(&bucket), &key.tag);
        if empty {
            self.reaction_generations.remove(&bucket);
        }
        for tag in obsolete {
            self.remove_reaction(&ReactionKey {
                federate: bucket.federate.clone(),
                enclave: bucket.enclave.clone(),
                reaction: bucket.reaction.clone(),
                tag,
            });
        }
    }

    fn ensure_generation(&mut self, key: &EvictionKey) {
        let (generation_key, tag, create) = match key {
            EvictionKey::Send(key)
            | EvictionKey::OpenSend(key)
            | EvictionKey::EarlyIngress(key) => {
                let bucket = PropagationBucketKey {
                    federate: key.federate.clone(),
                    action: key.action.clone(),
                };
                let create = self
                    .propagation_generations
                    .get(&bucket)
                    .and_then(|queue| queue.back())
                    .is_none_or(|generation| generation.tag < key.tag);
                (
                    TagGenerationKey::Propagation(bucket),
                    key.tag.clone(),
                    create,
                )
            }
            EvictionKey::Predecessor(key, _) => {
                let bucket = ReactionBucketKey {
                    federate: key.federate.clone(),
                    enclave: key.enclave.clone(),
                    reaction: key.reaction.clone(),
                };
                let create = self
                    .reaction_generations
                    .get(&bucket)
                    .and_then(|queue| queue.back())
                    .is_none_or(|generation| generation.tag < key.tag);
                (TagGenerationKey::Reaction(bucket), key.tag.clone(), create)
            }
        };
        if !create {
            return;
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        let value = TagGeneration {
            tag: tag.clone(),
            generation,
        };
        match &generation_key {
            TagGenerationKey::Propagation(bucket) => self
                .propagation_generations
                .entry(bucket.clone())
                .or_default()
                .push_back(value),
            TagGenerationKey::Reaction(bucket) => self
                .reaction_generations
                .entry(bucket.clone())
                .or_default()
                .push_back(value),
        }
        self.generation_eviction.push_back(TagGenerationToken {
            epoch: self.epoch,
            key: generation_key,
            tag,
            generation,
        });
    }

    fn expire_generation(&mut self) {
        let Some(token) = self.generation_eviction.pop_front() else {
            return;
        };
        match &token.key {
            TagGenerationKey::Propagation(bucket) => {
                let empty =
                    expire_generation_from(self.propagation_generations.get_mut(bucket), &token);
                if empty {
                    self.propagation_generations.remove(bucket);
                }
            }
            TagGenerationKey::Reaction(bucket) => {
                let empty =
                    expire_generation_from(self.reaction_generations.get_mut(bucket), &token);
                if empty {
                    self.reaction_generations.remove(bucket);
                }
            }
        }
    }

    fn remove_propagation(&mut self, key: &PropagationKey) {
        for removed in [
            self.sends.remove(key).is_some(),
            self.open_sends.remove(key).is_some(),
            self.early_ingress.remove(key).is_some(),
        ] {
            if removed {
                self.pending_count = self.pending_count.saturating_sub(1);
            }
        }
    }

    fn remove_reaction(&mut self, key: &ReactionKey) {
        if let Some(predecessors) = self.predecessors.remove(key) {
            self.pending_count = self.pending_count.saturating_sub(predecessors.len());
        }
    }
}

fn pop_older_generations(
    queue: Option<&mut VecDeque<TagGeneration>>,
    tag: &CompleteTag,
) -> (Vec<CompleteTag>, bool) {
    let Some(queue) = queue else {
        return (Vec::new(), false);
    };
    let mut obsolete = Vec::new();
    while queue
        .front()
        .is_some_and(|generation| generation.tag < *tag)
    {
        if let Some(generation) = queue.pop_front() {
            obsolete.push(generation.tag);
        }
    }
    (obsolete, queue.is_empty())
}

fn expire_generation_from(
    queue: Option<&mut VecDeque<TagGeneration>>,
    token: &TagGenerationToken,
) -> bool {
    let Some(queue) = queue else {
        return false;
    };
    if queue.front().is_some_and(|generation| {
        generation.generation == token.generation && generation.tag == token.tag
    }) {
        queue.pop_front();
    }
    queue.is_empty()
}

fn insert_resolution<K: Eq + std::hash::Hash>(
    map: &mut HashMap<K, PendingResolution>,
    key: K,
    id: TraceId,
    epoch: u64,
) -> bool {
    use std::collections::hash_map::Entry;
    match map.entry(key) {
        Entry::Occupied(mut entry) => {
            entry.insert(PendingResolution::Ambiguous { epoch });
            false
        }
        Entry::Vacant(entry) => {
            entry.insert(PendingResolution::Unique { id, epoch });
            true
        }
    }
}

fn remove_resolution_at(
    map: &mut HashMap<PropagationKey, PendingResolution>,
    key: &PropagationKey,
    epoch: u64,
) -> bool {
    if map.get(key).is_some_and(|value| value.epoch() == epoch) {
        map.remove(key);
        true
    } else {
        false
    }
}

fn remove_predecessor_at(
    predecessors: &mut HashMap<ReactionKey, Vec<PendingPredecessor>>,
    key: &ReactionKey,
    id: &TraceId,
    epoch: u64,
) -> bool {
    let Some(values) = predecessors.get_mut(key) else {
        return false;
    };
    let Some(index) = values
        .iter()
        .position(|value| value.epoch == epoch && value.id == *id)
    else {
        return false;
    };
    values.remove(index);
    if values.is_empty() {
        predecessors.remove(key);
    }
    true
}

impl<S> Layer<S> for RerunLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            if attrs.metadata().target() != TRACE_TARGET {
                return;
            }
            let mut fields = RawTraceFields::default();
            attrs.record(&mut fields);
            let parent = span_parent(attrs, &ctx);
            if let Some(parent) = &parent {
                fields.inherit_missing(&parent.fields());
            }
            let open = match fields.parse_open_span() {
                Ok(open) => open,
                Err(error) => {
                    self.diagnostic(error.to_string());
                    return;
                }
            };
            let Some(trace_id) = self.next_id(open.event().enclave().unwrap_or("unknown")) else {
                return;
            };
            if let Some(send) = open.propagation() {
                self.begin_propagation_send(send.event(), trace_id.clone());
            }
            let timepoint = self.timepoint(open.event().tag());
            let duration_state = open
                .event()
                .duration_phase()
                .map(|phase| (self.resolved_entity_path(open.event()), phase));
            let span_state = Arc::new(SpanState::new(
                trace_id,
                parent.map(|parent| parent.id.clone()),
                open,
                fields,
                timepoint.clone(),
                duration_state,
            ));
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(span_state.clone());
            }
            if let Some((entity_path, phase)) = &span_state.duration_state {
                self.write_state(TraceStateRecord {
                    entity_path: entity_path.clone(),
                    timepoint,
                    change: TraceStateChange::Set((*phase).to_owned()),
                });
            }
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            let Some(span) = ctx.span(id) else {
                return;
            };
            let extensions = span.extensions();
            if let Some(state) = extensions.get::<Arc<SpanState>>() {
                values.record(&mut *lock_unpoisoned(&state.fields));
            }
        });
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            if let Some(state) = span_state(id, &ctx) {
                state.enter();
            }
        });
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            if let Some(state) = span_state(id, &ctx) {
                state.exit();
            }
        });
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            let Some(state) = span_state(&id, &ctx) else {
                return;
            };
            let fields = state.fields();
            let parsed = match &state.open {
                OpenSpan::Complete(event) => Ok(event.clone()),
                OpenSpan::Propagation(_) => fields.parse(),
            };
            if let Ok(event) = parsed {
                let terminal_state = terminal_span_state(&event);
                if let Some(mut record) = self.make_record(
                    event,
                    state.parent_id.clone(),
                    Some(state.id.clone()),
                    Some(state.close_duration()),
                ) {
                    record.timepoint = state.timepoint.clone();
                    record.terminal_state = terminal_state;
                    self.write_with_causality(record);
                }
            } else if let Err(error) = parsed {
                if let Some(open) = state.open.propagation() {
                    self.abort_propagation_send(open.event(), &state.id);
                }
                self.diagnostic(error.to_string());
            }
            if let Some((entity_path, _)) = &state.duration_state {
                self.write_state(TraceStateRecord {
                    entity_path: entity_path.clone(),
                    timepoint: self.timepoint(state.open.event().tag()),
                    change: TraceStateChange::Reset,
                });
            }
        });
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            if event.metadata().target() != TRACE_TARGET {
                return;
            }
            let mut fields = RawTraceFields::default();
            event.record(&mut fields);
            let parent = event_parent(event, &ctx);
            if let Some(parent) = &parent {
                fields.inherit_missing(&parent.fields());
            }
            match fields.parse() {
                Ok(event) => {
                    if let Some(record) =
                        self.make_record(event, parent.map(|parent| parent.id.clone()), None, None)
                    {
                        self.write_with_causality(record);
                    }
                }
                Err(error) => self.diagnostic(error.to_string()),
            }
        });
    }
}

struct SpanState {
    id: TraceId,
    parent_id: Option<TraceId>,
    open: OpenSpan,
    fields: Mutex<RawTraceFields>,
    timepoint: TraceTimePoint,
    duration_state: Option<(String, &'static str)>,
    timing: Mutex<SpanTiming>,
}

#[derive(Default)]
struct SpanTiming {
    entered: HashMap<ThreadId, Vec<Instant>>,
    accumulated: Duration,
}

impl SpanState {
    fn new(
        id: TraceId,
        parent_id: Option<TraceId>,
        open: OpenSpan,
        fields: RawTraceFields,
        timepoint: TraceTimePoint,
        duration_state: Option<(String, &'static str)>,
    ) -> Self {
        Self {
            id,
            parent_id,
            open,
            fields: Mutex::new(fields),
            timepoint,
            duration_state,
            timing: Mutex::new(SpanTiming::default()),
        }
    }

    fn fields(&self) -> RawTraceFields {
        lock_unpoisoned(&self.fields).clone()
    }

    fn enter(&self) {
        lock_unpoisoned(&self.timing)
            .entered
            .entry(std::thread::current().id())
            .or_default()
            .push(Instant::now());
    }

    fn exit(&self) {
        let now = Instant::now();
        let mut timing = lock_unpoisoned(&self.timing);
        if let Some(start) = timing
            .entered
            .get_mut(&std::thread::current().id())
            .and_then(Vec::pop)
        {
            timing.accumulated = timing.accumulated.saturating_add(now.duration_since(start));
        }
    }

    fn close_duration(&self) -> Duration {
        let now = Instant::now();
        let mut timing = lock_unpoisoned(&self.timing);
        let outstanding = timing
            .entered
            .values_mut()
            .flat_map(|entries| entries.drain(..))
            .fold(Duration::ZERO, |total, start| {
                total.saturating_add(now.duration_since(start))
            });
        timing.accumulated.saturating_add(outstanding)
    }
}

fn span_state<S>(id: &Id, ctx: &Context<'_, S>) -> Option<Arc<SpanState>>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    ctx.span(id)
        .and_then(|span| span.extensions().get::<Arc<SpanState>>().cloned())
}

fn span_parent<S>(attrs: &Attributes<'_>, ctx: &Context<'_, S>) -> Option<Arc<SpanState>>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    if attrs.is_root() {
        None
    } else if let Some(parent) = attrs.parent() {
        span_state(parent, ctx)
    } else if attrs.is_contextual() {
        ctx.lookup_current()
            .and_then(|span| span.extensions().get::<Arc<SpanState>>().cloned())
    } else {
        None
    }
}

fn terminal_span_state(event: &TraceEvent) -> Option<String> {
    event.duration_phase().map(|_| {
        if event.terminal() == Some(true) {
            "terminal"
        } else {
            "complete"
        }
        .to_owned()
    })
}

fn event_parent<S>(event: &Event<'_>, ctx: &Context<'_, S>) -> Option<Arc<SpanState>>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    if let Some(parent) = event.parent().and_then(|id| span_state(id, ctx)) {
        return Some(parent);
    }
    ctx.event_scope(event).and_then(|scope| {
        scope
            .from_root()
            .filter_map(|span| span.extensions().get::<Arc<SpanState>>().cloned())
            .last()
    })
}

fn saturating_i64(value: u128) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn panic_error(context: &str, panic: &(dyn std::any::Any + Send)) -> TraceWriterError {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    TraceWriterError(format!("{context} panicked: {message}"))
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::rerun::schema::{
        CausalLink, IngressOutcome, LogicalIngress, PropagationReceive, TraceEvent, TraceTag,
        ValueDescriptor,
    };
    use crate::rerun::{RerunSessionBuilder, TraceRecord, TraceWriter, TraceWriterError};

    fn causal_link(record: &TraceRecord) -> Option<&CausalLink> {
        match &record.event {
            TraceEvent::CausalLink(link) => Some(link),
            _ => None,
        }
    }

    fn propagation_receive(record: &TraceRecord) -> Option<&PropagationReceive> {
        match &record.event {
            TraceEvent::PropagationReceive(receive) => Some(receive),
            _ => None,
        }
    }

    #[derive(Default)]
    struct IdCapture(Mutex<Vec<TraceId>>);

    impl TraceWriter for IdCapture {
        fn write(
            &self,
            _recording: &rerun::RecordingStream,
            record: &TraceRecord,
        ) -> Result<(), TraceWriterError> {
            lock_unpoisoned(&self.0).push(record.id.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordCapture(Mutex<Vec<TraceRecord>>);

    impl TraceWriter for RecordCapture {
        fn write(
            &self,
            _recording: &rerun::RecordingStream,
            record: &TraceRecord,
        ) -> Result<(), TraceWriterError> {
            lock_unpoisoned(&self.0).push(record.clone());
            Ok(())
        }
    }

    #[cfg(feature = "federated")]
    struct IngressDuringSerializedSend {
        target: Option<boomerang_federated::FederateId>,
        action_key: String,
    }

    #[cfg(feature = "federated")]
    impl boomerang_federated::FederatedOutboundSink for IngressDuringSerializedSend {
        fn target_federate(&self) -> Option<&boomerang_federated::FederateId> {
            self.target.as_ref()
        }

        fn send(
            &self,
            command: boomerang_federated::FederatedOutboundCommand,
        ) -> Result<(), boomerang_federated::FederatedEndpointError> {
            let boomerang_federated::FederatedOutboundCommand::Msg(message) = command;
            tracing::trace!(
                target: TRACE_TARGET,
                event = "async_ingress",
                federate = "b",
                enclave = "e0",
                kind = "logical",
                action_key = %self.action_key,
                logical_ns = boomerang_runtime::trace::logical_ns(message.tag),
                microstep = boomerang_runtime::trace::microstep(message.tag),
                action = "input",
                destination_logical_ns = boomerang_runtime::trace::logical_ns(message.tag),
                destination_microstep = boomerang_runtime::trace::microstep(message.tag),
                value_type = "u32",
                value_size = std::mem::size_of::<u32>(),
                outcome = "accepted",
            );
            Ok(())
        }
    }

    #[cfg(feature = "federated")]
    fn exercise_serialized_send_race(
        session: &crate::rerun::RerunSession,
        target: Option<boomerang_federated::FederateId>,
    ) {
        use boomerang_runtime::ActionCommon;

        let action = boomerang_runtime::Action::<u32>::new(
            "input",
            boomerang_runtime::ActionKey::from(0),
            None,
            true,
        );
        let action_ref =
            boomerang_runtime::AsyncActionRef::try_from(boomerang_runtime::DynActionRef(&action))
                .unwrap();
        let action_key = action_ref.key().to_string();
        session
            .adapter
            .registration
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register_in_federate(
                Some("b"),
                "e0",
                "action",
                &action_key,
                "input",
                "/federates/b/enclaves/e0/actions/input",
            );
        let sink = boomerang_federated::SerializedInterPartitionEventSink::new(
            Box::new(|value: &u32| Ok(value.to_le_bytes().to_vec())),
            Box::new(IngressDuringSerializedSend { target, action_key }),
            boomerang_federated::FederatedFaultState::default(),
        );
        let tag = boomerang_runtime::Tag::ZERO;
        let reaction = tracing::trace_span!(
            target: TRACE_TARGET,
            "reaction_execute",
            event = "reaction_execute",
            federate = "a",
            enclave = "source",
            reaction_key = "sender",
            reactor = "source",
            reaction = "sender",
            level = 0_u64,
            logical_ns = boomerang_runtime::trace::logical_ns(tag),
            microstep = boomerang_runtime::trace::microstep(tag),
            state = "begin",
        );
        {
            let _entered = reaction.enter();
            boomerang_runtime::InterPartitionEventSink::send(
                &sink,
                boomerang_runtime::InterPartitionEventTime::Logical(tag),
                &action_ref,
                &7,
            );
        }
        drop(reaction);
        tracing::trace!(
            target: TRACE_TARGET,
            event = "async_ingress",
            federate = "b",
            enclave = "e0",
            kind = "logical",
            action_key = %action_ref.key(),
            logical_ns = boomerang_runtime::trace::logical_ns(tag),
            microstep = boomerang_runtime::trace::microstep(tag),
            action = "input",
            destination_logical_ns = boomerang_runtime::trace::logical_ns(tag),
            destination_microstep = boomerang_runtime::trace::microstep(tag),
            value_type = "u32",
            value_size = std::mem::size_of::<u32>(),
            outcome = "accepted",
        );
    }

    #[test]
    fn sequence_exhaustion_disables_without_emitting_duplicate_id() {
        let captured = Arc::new(IdCapture::default());
        let session = RerunSessionBuilder::new("sequence-exhaustion")
            .source_id("sequence-exhaustion")
            .trace_writer(captured.clone())
            .build()
            .unwrap();
        session
            .adapter
            .next_id
            .store(u64::MAX - 1, Ordering::Relaxed);
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                tracing::trace!(
                    target: TRACE_TARGET,
                    event = "shutdown",
                    enclave = "e0",
                    logical_ns = 0_u64,
                    microstep = 0_u64,
                    state = "complete",
                    outcome = "success",
                );
            }
        });

        assert!(!session.is_enabled());
        assert_eq!(session.error_count(), 1);
        assert_eq!(
            lock_unpoisoned(&captured.0).clone(),
            [TraceId::new("sequence-exhaustion", "e0", u64::MAX - 1)]
        );
    }

    #[test]
    fn correlation_state_evicts_stale_entries_and_is_strictly_bounded() {
        let mut state = CorrelationState::default();
        for logical_ns in 0..(MAX_PENDING_CORRELATIONS as u64 + 32) {
            state.advance();
            state.insert_send(
                PropagationKey {
                    federate: None,
                    action: format!("action-{logical_ns}"),
                    tag: CompleteTag {
                        logical_ns,
                        microstep: 0,
                    },
                },
                TraceId(format!("id-{logical_ns}")),
            );
        }
        assert_eq!(state.sends.len(), MAX_PENDING_CORRELATIONS);
        assert!(!state.sends.keys().any(|key| key.tag.logical_ns == 0));

        for _ in 0..=MAX_CORRELATION_AGE {
            state.advance();
        }
        assert!(state.sends.is_empty());
        assert!(state.predecessors.is_empty());
        assert!(state.propagation_generations.is_empty());
        assert!(state.reaction_generations.is_empty());
    }

    #[test]
    fn distinct_pending_predecessors_are_all_retained_for_one_reaction() {
        let mut state = CorrelationState::default();
        state.advance();
        let key = ReactionKey {
            federate: None,
            enclave: "e0".to_owned(),
            reaction: "reaction".to_owned(),
            tag: CompleteTag {
                logical_ns: 1,
                microstep: 0,
            },
        };
        state.insert_predecessor(key.clone(), TraceId("first".to_owned()));
        state.insert_predecessor(key.clone(), TraceId("second".to_owned()));
        assert_eq!(
            state.take_predecessors(&key),
            vec![TraceId("first".to_owned()), TraceId("second".to_owned())]
        );
    }

    #[test]
    fn ambiguous_send_poison_survives_until_age_cleanup() {
        let mut state = CorrelationState::default();
        state.advance();
        let key = PropagationKey {
            federate: None,
            action: "action".to_owned(),
            tag: CompleteTag {
                logical_ns: 1,
                microstep: 0,
            },
        };
        state.insert_send(key.clone(), TraceId("first".to_owned()));
        state.insert_send(key.clone(), TraceId("second".to_owned()));
        assert_eq!(state.take_send(&key), None);
        state.insert_send(key.clone(), TraceId("later".to_owned()));
        assert_eq!(state.take_send(&key), None);

        for _ in 0..=MAX_CORRELATION_AGE {
            state.advance();
        }
        state.insert_send(key.clone(), TraceId("fresh".to_owned()));
        assert_eq!(state.take_send(&key), Some(TraceId("fresh".to_owned())));
    }

    #[test]
    fn ambiguous_early_ingress_poison_survives_until_age_cleanup() {
        let mut state = CorrelationState::default();
        state.advance();
        let key = PropagationKey {
            federate: Some("b".to_owned()),
            action: "/federates/b/actions/input".to_owned(),
            tag: CompleteTag {
                logical_ns: 1,
                microstep: 0,
            },
        };
        let ingress = |id: &str| TraceRecord {
            id: TraceId(id.to_owned()),
            parent_id: None,
            entity_path: "/ingress".to_owned(),
            timepoint: TraceTimePoint {
                elapsed_ns: 0,
                wall_clock_unix_ns: 0,
                logical_ns: Some(1),
            },
            duration_ns: None,
            terminal_state: None,
            event: TraceEvent::LogicalIngress(LogicalIngress {
                federate: Some("b".to_owned()),
                enclave: "e0".to_owned(),
                action_key: "/federates/b/actions/input".to_owned(),
                action: "input".to_owned(),
                tag: TraceTag {
                    logical_ns: 1,
                    microstep: 0,
                },
                destination_tag: TraceTag {
                    logical_ns: 1,
                    microstep: 0,
                },
                value: ValueDescriptor {
                    value_type: "u32".to_owned(),
                    value_size: 4,
                },
                outcome: IngressOutcome::Accepted,
            }),
        };
        let open = TraceId("open".to_owned());
        state.begin_open_send(key.clone(), open.clone());
        assert!(state.capture_early_ingress(key.clone(), ingress("first")));
        assert!(state.capture_early_ingress(key.clone(), ingress("second")));
        assert!(matches!(
            state.finish_open_send(&key, &open, true),
            FinishOpenSend::Handled
        ));

        state.insert_send(key.clone(), TraceId("later".to_owned()));
        assert_eq!(state.take_send(&key), None);
        for _ in 0..=MAX_CORRELATION_AGE {
            state.advance();
        }
        state.insert_send(key.clone(), TraceId("fresh".to_owned()));
        assert_eq!(state.take_send(&key), Some(TraceId("fresh".to_owned())));
    }

    #[test]
    fn two_distinct_action_receives_both_link_to_the_same_reaction() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("multiple-predecessors")
            .source_id("multiple-predecessors")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        {
            let mut registration = session
                .adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration.register("e0", "action", "a0", "a0", "/actions/a0");
            registration.register("e0", "action", "a1", "a1", "/actions/a1");
            registration.register("e0", "reaction", "r0", "r0", "/reactions/r0");
            registration.register_action_trigger("/actions/a0", "/reactions/r0");
            registration.register_action_trigger("/actions/a1", "/reactions/r0");
        }
        let subscriber = tracing_subscriber::registry().with(session.layer());
        tracing::subscriber::with_default(subscriber, || {
            for action in ["a0", "a1"] {
                tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = action, action, logical_ns = 3_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
                tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = action, action, logical_ns = 3_u64, microstep = 0_u64, destination_logical_ns = 3_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            }
            let reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reactor = "root", reaction_key = "r0", reaction = "r0", level = 0_u64, logical_ns = 3_u64, microstep = 0_u64, state = "begin");
            let _entered = reaction.enter();
        });

        let records = lock_unpoisoned(&capture.0);
        let reaction = records
            .iter()
            .find(|record| matches!(&record.event, TraceEvent::ReactionExecution(_)))
            .unwrap();
        let receives = records
            .iter()
            .filter(|record| propagation_receive(record).is_some())
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 2);
        let predecessors = records
            .iter()
            .filter_map(causal_link)
            .filter(|link| link.destination == reaction.id)
            .map(|link| link.source.0.clone())
            .collect::<Vec<_>>();
        assert_eq!(predecessors.len(), 2);
        assert!(receives
            .iter()
            .all(|receive| predecessors.contains(&receive.0)));
    }

    #[test]
    fn ambiguous_send_interleaving_stays_neutral_until_next_tag_generation() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("persistent-poison")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        {
            let mut registration = session
                .adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration.register("e0", "action", "a0", "a0", "/actions/a0");
            registration.register("e0", "reaction", "r0", "r0", "/reactions/r0");
            registration.register_action_trigger("/actions/a0", "/reactions/r0");
        }
        let subscriber = tracing_subscriber::registry().with(session.layer());
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            }
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            let poisoned_reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reactor = "root", reaction_key = "r0", reaction = "r0", level = 0_u64, logical_ns = 1_u64, microstep = 0_u64, state = "begin");
            drop(poisoned_reaction.enter());
            drop(poisoned_reaction);
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 2_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 2_u64, microstep = 0_u64, destination_logical_ns = 2_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            let fresh_reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reactor = "root", reaction_key = "r0", reaction = "r0", level = 0_u64, logical_ns = 2_u64, microstep = 0_u64, state = "begin");
            drop(fresh_reaction.enter());
            drop(fresh_reaction);
        });

        let records = lock_unpoisoned(&capture.0);
        let receives = records
            .iter()
            .filter_map(|record| propagation_receive(record).map(|receive| (record, receive)))
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 1);
        assert_eq!(receives[0].1.tag.logical_ns, 2);
        assert_eq!(
            records
                .iter()
                .filter(|record| causal_link(record).is_some())
                .count(),
            2
        );
    }

    #[test]
    fn ingress_during_open_send_span_links_only_after_accepted_close() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("two-phase-send")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        session
            .adapter
            .registration
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register("e0", "action", "a0", "a0", "/actions/a0");
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            let send = tracing::trace_span!(
                target: TRACE_TARGET,
                "propagation_send",
                event = "propagation_send",
                enclave = "source",
                kind = "logical",
                destination = "e0",
                action_key = "a0",
                action = "a0",
                logical_ns = 1_u64,
                microstep = 0_u64,
                value_type = "u32",
                value_size = 4_u64,
                outcome = tracing::field::Empty,
            );
            let entered = send.enter();
            tracing::trace!(
                target: TRACE_TARGET,
                event = "async_ingress",
                enclave = "e0",
                kind = "logical",
                action_key = "a0",
                action = "a0",
                logical_ns = 1_u64,
                microstep = 0_u64,
                destination_logical_ns = 1_u64,
                destination_microstep = 0_u64,
                value_type = "u32",
                value_size = 4_u64,
                outcome = "accepted",
            );
            send.record("outcome", "accepted");
            drop(entered);
            drop(send);
        });

        let records = lock_unpoisoned(&capture.0);
        let send = records
            .iter()
            .find(|record| matches!(&record.event, TraceEvent::PropagationLogicalSend(_)))
            .unwrap();
        let receive = records
            .iter()
            .find(|record| propagation_receive(record).is_some())
            .expect("accepted send must correlate with ingress observed before span close");
        assert_eq!(receive.parent_id.as_ref(), Some(&send.id));
        let ingress = records
            .iter()
            .find(|record| matches!(&record.event, TraceEvent::LogicalIngress(_)))
            .unwrap();
        assert!(send.timepoint.elapsed_ns <= ingress.timepoint.elapsed_ns);
    }

    #[cfg(feature = "federated")]
    #[test]
    fn serialized_send_route_identity_controls_during_dispatch_correlation() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("serialized-send-race")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            exercise_serialized_send_race(
                &session,
                Some(boomerang_federated::FederateId::new("b")),
            );
            exercise_serialized_send_race(&session, None);
        });

        let records = lock_unpoisoned(&capture.0);
        let send = records
            .iter()
            .find(|record| {
                matches!(
                    &record.event,
                    TraceEvent::PropagationSerializedSend(send)
                        if send.destination_federate.as_deref() == Some("b")
                )
            })
            .unwrap_or_else(|| panic!("missing serialized send record: {records:#?}"));
        let receives = records
            .iter()
            .filter(|record| propagation_receive(record).is_some())
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 1);
        assert_eq!(receives[0].parent_id.as_ref(), Some(&send.id));
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(&record.event, TraceEvent::LogicalIngress(_)))
                .count(),
            4
        );
    }

    #[test]
    fn ingress_during_failed_send_span_remains_neutral() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("failed-two-phase-send")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        session
            .adapter
            .registration
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register("e0", "action", "a0", "a0", "/actions/a0");
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            let send = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = tracing::field::Empty);
            let entered = send.enter();
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            send.record("outcome", "failed");
            drop(entered);
            drop(send);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| propagation_receive(record).is_none() && causal_link(record).is_none()));
    }

    #[test]
    fn invalid_late_send_outcome_rolls_back_early_ingress_correlation() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("invalid-late-send-outcome")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        {
            let mut registration = session
                .adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration.register("e0", "action", "a0", "a0", "/actions/a0");
            registration.register("e0", "reaction", "r0", "r0", "/reactions/r0");
            registration.register_action_trigger("/actions/a0", "/reactions/r0");
        }
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            let send = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = tracing::field::Empty);
            let entered = send.enter();
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            send.record("outcome", "invalid");
            drop(entered);
            drop(send);

            let reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reactor = "root", reaction_key = "r0", reaction = "r0", level = 0_u64, logical_ns = 1_u64, microstep = 0_u64, state = "begin");
            drop(reaction);
        });

        let records = lock_unpoisoned(&capture.0);
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.event, TraceEvent::SchemaDiagnostic(_)))
                .count(),
            1
        );
        assert!(records.iter().all(|record| {
            propagation_receive(record).is_none() && causal_link(record).is_none()
        }));
    }

    #[test]
    fn duplicate_open_send_candidates_keep_early_ingress_neutral() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("ambiguous-two-phase-send")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        session
            .adapter
            .registration
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register("e0", "action", "a0", "a0", "/actions/a0");
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            let first = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = tracing::field::Empty);
            let second = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = tracing::field::Empty);
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            first.record("outcome", "accepted");
            second.record("outcome", "accepted");
            drop(first);
            drop(second);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| propagation_receive(record).is_none() && causal_link(record).is_none()));
    }

    #[test]
    fn completed_and_open_send_candidates_keep_ingress_neutral() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("mixed-ambiguous-send")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        session
            .adapter
            .registration
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register("e0", "action", "a0", "a0", "/actions/a0");
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            let open = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = tracing::field::Empty);
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            open.record("outcome", "accepted");
            drop(open);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| propagation_receive(record).is_none() && causal_link(record).is_none()));
    }

    #[test]
    fn modal_rebase_moves_receive_predecessor_to_destination_tag() {
        let capture = Arc::new(RecordCapture::default());
        let session = RerunSessionBuilder::new("modal-rebase")
            .trace_writer(capture.clone())
            .build()
            .unwrap();
        {
            let mut registration = session
                .adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration.register("e0", "action", "a0", "a0", "/actions/a0");
            registration.register("e0", "reaction", "r0", "r0", "/reactions/r0");
            registration.register_action_trigger("/actions/a0", "/reactions/r0");
        }
        let subscriber = tracing_subscriber::registry().with(session.layer());

        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", action = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, value_type = "u32", value_size = 4_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "action_schedule", enclave = "e0", action_key = "a0", old_logical_ns = 1_u64, old_microstep = 0_u64, destination_logical_ns = 5_u64, destination_microstep = 0_u64, outcome = "rebased");
            let reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reactor = "root", reaction_key = "r0", reaction = "r0", level = 0_u64, logical_ns = 5_u64, microstep = 0_u64, state = "begin");
            drop(reaction);
        });

        let records = lock_unpoisoned(&capture.0);
        let receive = records
            .iter()
            .find(|record| propagation_receive(record).is_some())
            .unwrap();
        let reaction = records
            .iter()
            .find(|record| matches!(&record.event, TraceEvent::ReactionExecution(_)))
            .unwrap();
        assert!(records.iter().any(|record| {
            causal_link(record)
                .is_some_and(|link| link.source == receive.id && link.destination == reaction.id)
        }));
    }
}
