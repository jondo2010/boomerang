use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::entities::{
    entity_path, RegistrationIndex, TraceFields, TraceId, TraceRecord, TraceTimePoint, TraceWriter,
    TraceWriterError,
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

    fn timepoint(&self, fields: &TraceFields) -> TraceTimePoint {
        TraceTimePoint {
            elapsed_ns: saturating_i64(self.started.elapsed().as_nanos()),
            wall_clock_unix_ns: saturating_i64(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            ),
            logical_ns: fields
                .logical_ns
                .and_then(|value| i64::try_from(value).ok()),
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
        let fields = TraceFields {
            event: Some("diagnostic".to_owned()),
            state: Some("schema_error".to_owned()),
            outcome: Some("ignored".to_owned()),
            error: Some(message),
            ..TraceFields::default()
        };
        let Some(id) = self.next_id("unknown") else {
            return;
        };
        self.write(TraceRecord {
            id,
            parent_id: None,
            entity_path: "/diagnostics/schema".to_owned(),
            event: "diagnostic".to_owned(),
            timepoint: self.timepoint(&fields),
            microstep: None,
            duration_ns: None,
            terminal_state: None,
            fields,
        });
    }

    fn make_record(
        &self,
        fields: TraceFields,
        parent_id: Option<TraceId>,
        id: Option<TraceId>,
        duration: Option<Duration>,
    ) -> Option<TraceRecord> {
        let Some(event) = fields.event.clone() else {
            self.diagnostic("missing required field `event`");
            return None;
        };
        let Some(enclave) = fields.enclave.as_deref() else {
            self.diagnostic(format!(
                "event `{event}` is missing required field `enclave`"
            ));
            return None;
        };
        let id = if let Some(id) = id {
            id
        } else {
            self.next_id(enclave)?
        };
        let entity_path = self
            .adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entity_path(&fields, &event)
            .unwrap_or_else(|| entity_path(&fields, &event));
        let entity_path = if event == "propagation_send" {
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
            event,
            timepoint: self.timepoint(&fields),
            microstep: fields.microstep,
            duration_ns: duration.map(|value| u64::try_from(value.as_nanos()).unwrap_or(u64::MAX)),
            terminal_state: None,
            fields,
        })
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
            match record.event.as_str() {
                "propagation_send" | "async_ingress" | "action_schedule" => registration
                    .resolve_entity(&record.fields, record.event.as_str())
                    .map(|action| {
                        let reactions = registration.triggered_reactions(&action).to_vec();
                        (action, reactions)
                    }),
                "reaction_execute" => registration
                    .resolve_entity(&record.fields, record.event.as_str())
                    .map(|reaction| (reaction, Vec::new())),
                _ => None,
            }
        };

        let mut correlation = lock_unpoisoned(&self.adapter.correlation);
        correlation.advance();
        match record.event.as_str() {
            "propagation_send" if record.fields.kind.as_deref() == Some("logical") => {
                if let (Some((action, _)), Some(tag)) =
                    (topology, CompleteTag::from_fields(&record.fields))
                {
                    let key = PropagationKey {
                        federate: record
                            .fields
                            .destination_federate
                            .clone()
                            .or_else(|| record.fields.federate.clone()),
                        action,
                        tag,
                    };
                    let accepted = record.fields.outcome.as_deref() == Some("accepted");
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
            "async_ingress"
                if record.fields.kind.as_deref() == Some("logical")
                    && record.fields.outcome.as_deref() == Some("accepted") =>
            {
                let (Some((action, _)), Some(tag)) =
                    (topology, CompleteTag::from_fields(&record.fields))
                else {
                    return Vec::new();
                };
                let key = PropagationKey {
                    federate: record.fields.federate.clone(),
                    action,
                    tag: tag.clone(),
                };
                if let Some(send) = correlation.take_send(&key) {
                    return self.derive_receive(&mut correlation, send, record.clone());
                }
                if correlation.capture_early_ingress(key, record.clone()) {
                    return Vec::new();
                }
                Vec::new()
            }
            "action_schedule" if record.fields.outcome.as_deref() == Some("rebased") => {
                if let (Some((_, reactions)), Some(enclave), Some(old_tag), Some(destination_tag)) = (
                    topology,
                    record.fields.enclave.as_deref(),
                    CompleteTag::old_from_fields(&record.fields),
                    CompleteTag::destination_from_fields(&record.fields),
                ) {
                    correlation.rebase_predecessors(
                        record.fields.federate.as_deref(),
                        enclave,
                        &reactions,
                        &old_tag,
                        &destination_tag,
                    );
                }
                Vec::new()
            }
            "reaction_execute" => {
                let (Some((reaction, _)), Some(tag), Some(enclave)) = (
                    topology,
                    CompleteTag::from_fields(&record.fields),
                    record.fields.enclave.clone(),
                ) else {
                    return Vec::new();
                };
                correlation
                    .take_predecessors(&ReactionKey {
                        federate: record.fields.federate.clone(),
                        enclave: enclave.clone(),
                        reaction,
                        tag,
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
        let Some(enclave) = ingress.fields.enclave.clone() else {
            return Vec::new();
        };
        let Some(tag) = CompleteTag::from_fields(&ingress.fields) else {
            return Vec::new();
        };
        let reactions = {
            let registration = self
                .adapter
                .registration
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registration
                .resolve_entity(&ingress.fields, ingress.event.as_str())
                .map(|action| registration.triggered_reactions(&action).to_vec())
                .unwrap_or_default()
        };
        let Some(receive_id) = self.next_id(&enclave) else {
            return Vec::new();
        };
        let mut receive_fields = ingress.fields.clone();
        receive_fields.event = Some("propagation_receive".to_owned());
        let receive = TraceRecord {
            entity_path: format!(
                "/propagation/receives/{}",
                super::entities::escape_entity_segment(&receive_id.0)
            ),
            event: "propagation_receive".to_owned(),
            id: receive_id.clone(),
            parent_id: Some(send.clone()),
            timepoint: ingress.timepoint.clone(),
            microstep: ingress.microstep,
            duration_ns: None,
            terminal_state: None,
            fields: receive_fields,
        };
        for reaction in reactions {
            correlation.insert_predecessor(
                ReactionKey {
                    federate: ingress.fields.federate.clone(),
                    enclave: enclave.clone(),
                    reaction,
                    tag: tag.clone(),
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

    fn begin_propagation_send(&self, fields: &TraceFields, id: TraceId) {
        if fields.event.as_deref() != Some("propagation_send")
            || fields.kind.as_deref() != Some("logical")
        {
            return;
        }
        let action = self
            .adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resolve_entity(fields, "propagation_send");
        if let (Some(action), Some(tag)) = (action, CompleteTag::from_fields(fields)) {
            lock_unpoisoned(&self.adapter.correlation).begin_open_send(
                PropagationKey {
                    federate: fields
                        .destination_federate
                        .clone()
                        .or_else(|| fields.federate.clone()),
                    action,
                    tag,
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
        let fields = TraceFields {
            event: Some("causal_link".to_owned()),
            enclave: Some(enclave.to_owned()),
            source: Some(source.0.clone()),
            destination: Some(destination.0.clone()),
            logical_ns: at.fields.logical_ns,
            microstep: at.fields.microstep,
            state: Some("exact".to_owned()),
            outcome: Some("matched".to_owned()),
            ..TraceFields::default()
        };
        Some(TraceRecord {
            entity_path: format!(
                "/propagation/links/{}",
                super::entities::escape_entity_segment(&id.0)
            ),
            event: "causal_link".to_owned(),
            id,
            parent_id: Some(source.clone()),
            timepoint: at.timepoint.clone(),
            microstep: at.microstep,
            duration_ns: None,
            terminal_state: None,
            fields,
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

impl CompleteTag {
    fn from_fields(fields: &TraceFields) -> Option<Self> {
        Some(Self {
            logical_ns: fields.destination_logical_ns.or(fields.logical_ns)?,
            microstep: fields.destination_microstep.or(fields.microstep)?,
        })
    }

    fn old_from_fields(fields: &TraceFields) -> Option<Self> {
        Some(Self {
            logical_ns: fields.old_logical_ns?,
            microstep: fields.old_microstep?,
        })
    }

    fn destination_from_fields(fields: &TraceFields) -> Option<Self> {
        Some(Self {
            logical_ns: fields.destination_logical_ns?,
            microstep: fields.destination_microstep?,
        })
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
            let mut fields = TraceFields::default();
            attrs.record(&mut fields);
            let parent = span_parent(attrs, &ctx);
            if let Some(parent) = &parent {
                fields.inherit_missing(&parent.fields());
            }
            let Some(event) = fields.event.as_deref() else {
                self.diagnostic("missing required field `event`");
                return;
            };
            let Some(enclave) = fields.enclave.as_deref() else {
                self.diagnostic(format!(
                    "span `{event}` is missing required field `enclave`"
                ));
                return;
            };
            let Some(trace_id) = self.next_id(enclave) else {
                return;
            };
            self.begin_propagation_send(&fields, trace_id.clone());
            let timepoint = self.timepoint(&fields);
            let span_state = Arc::new(SpanState::new(
                trace_id,
                parent.map(|parent| parent.id.clone()),
                fields,
                timepoint,
            ));
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(span_state);
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
            let terminal_state = terminal_span_state(&fields);
            if let Some(mut record) = self.make_record(
                fields,
                state.parent_id.clone(),
                Some(state.id.clone()),
                Some(state.close_duration()),
            ) {
                record.timepoint = state.timepoint.clone();
                record.terminal_state = terminal_state;
                self.write_with_causality(record);
            }
        });
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        self.observe_callback(|| {
            if event.metadata().target() != TRACE_TARGET {
                return;
            }
            let mut fields = TraceFields::default();
            event.record(&mut fields);
            let parent = event_parent(event, &ctx);
            if let Some(parent) = &parent {
                fields.inherit_missing(&parent.fields());
            }
            if let Some(record) =
                self.make_record(fields, parent.map(|parent| parent.id.clone()), None, None)
            {
                self.write_with_causality(record);
            }
        });
    }
}

struct SpanState {
    id: TraceId,
    parent_id: Option<TraceId>,
    fields: Mutex<TraceFields>,
    timepoint: TraceTimePoint,
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
        fields: TraceFields,
        timepoint: TraceTimePoint,
    ) -> Self {
        Self {
            id,
            parent_id,
            fields: Mutex::new(fields),
            timepoint,
            timing: Mutex::new(SpanTiming::default()),
        }
    }

    fn fields(&self) -> TraceFields {
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

fn terminal_span_state(fields: &TraceFields) -> Option<String> {
    match fields.event.as_deref() {
        Some("tag_process" | "reaction_execute" | "coordination_wait") => Some(
            if fields.terminal == Some(true) {
                "terminal"
            } else {
                "complete"
            }
            .to_owned(),
        ),
        _ => None,
    }
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

impl Visit for TraceFields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "logical_ns" => self.logical_ns = Some(value),
            "microstep" => self.microstep = Some(value),
            "destination_logical_ns" => self.destination_logical_ns = Some(value),
            "destination_microstep" => self.destination_microstep = Some(value),
            "old_logical_ns" => self.old_logical_ns = Some(value),
            "old_microstep" => self.old_microstep = Some(value),
            "value_size" => self.value_size = Some(value),
            _ => {}
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if value >= 0 {
            self.record_u64(field, value as u64);
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "terminal" {
            self.terminal = Some(value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_text(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut value = format!("{value:?}");
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_owned();
        }
        self.record_text(field.name(), value);
    }
}

impl TraceFields {
    fn record_text(&mut self, name: &str, value: String) {
        match name {
            "event" => self.event = Some(value),
            "federate" => self.federate = Some(value),
            "enclave" => self.enclave = Some(value),
            "kind" => self.kind = Some(value),
            "reactor" => self.reactor = Some(value),
            "reaction_key" => self.reaction_key = Some(value),
            "reaction" => self.reaction = Some(value),
            "action_key" => self.action_key = Some(value),
            "action" => self.action = Some(value),
            "port_key" => self.port_key = Some(value),
            "port" => self.port = Some(value),
            "destination" => self.destination = Some(value),
            "destination_federate" => self.destination_federate = Some(value),
            "source" => self.source = Some(value),
            "level" => self.level = Some(value),
            "state" => self.state = Some(value),
            "value_type" => self.value_type = Some(value),
            "outcome" => self.outcome = Some(value),
            "error" => self.error = Some(value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::rerun::{RerunSessionBuilder, TraceRecord, TraceWriter, TraceWriterError};

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
                source = "during",
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
            source = "later",
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
            event: "async_ingress".to_owned(),
            timepoint: TraceTimePoint {
                elapsed_ns: 0,
                wall_clock_unix_ns: 0,
                logical_ns: Some(1),
            },
            microstep: Some(0),
            duration_ns: None,
            terminal_state: None,
            fields: TraceFields::default(),
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
                tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = action, logical_ns = 3_u64, microstep = 0_u64, outcome = "accepted");
                tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = action, logical_ns = 3_u64, microstep = 0_u64, destination_logical_ns = 3_u64, destination_microstep = 0_u64, outcome = "accepted");
            }
            let reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reaction_key = "r0", logical_ns = 3_u64, microstep = 0_u64, state = "begin");
            let _entered = reaction.enter();
        });

        let records = lock_unpoisoned(&capture.0);
        let reaction = records
            .iter()
            .find(|record| record.event == "reaction_execute")
            .unwrap();
        let receives = records
            .iter()
            .filter(|record| record.event == "propagation_receive")
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 2);
        let predecessors = records
            .iter()
            .filter(|record| {
                record.event == "causal_link"
                    && record.fields.destination.as_deref() == Some(reaction.id.0.as_str())
            })
            .filter_map(|record| record.fields.source.clone())
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
                tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = "accepted");
            }
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            let poisoned_reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reaction_key = "r0", logical_ns = 1_u64, microstep = 0_u64, state = "begin");
            drop(poisoned_reaction.enter());
            drop(poisoned_reaction);
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 2_u64, microstep = 0_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 2_u64, microstep = 0_u64, destination_logical_ns = 2_u64, destination_microstep = 0_u64, outcome = "accepted");
            let fresh_reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reaction_key = "r0", logical_ns = 2_u64, microstep = 0_u64, state = "begin");
            drop(fresh_reaction.enter());
            drop(fresh_reaction);
        });

        let records = lock_unpoisoned(&capture.0);
        let receives = records
            .iter()
            .filter(|record| record.event == "propagation_receive")
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 1);
        assert_eq!(receives[0].fields.logical_ns, Some(2));
        assert_eq!(
            records
                .iter()
                .filter(|record| record.event == "causal_link")
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
                logical_ns = 1_u64,
                microstep = 0_u64,
                outcome = tracing::field::Empty,
            );
            let entered = send.enter();
            tracing::trace!(
                target: TRACE_TARGET,
                event = "async_ingress",
                enclave = "e0",
                kind = "logical",
                action_key = "a0",
                logical_ns = 1_u64,
                microstep = 0_u64,
                destination_logical_ns = 1_u64,
                destination_microstep = 0_u64,
                outcome = "accepted",
            );
            send.record("outcome", "accepted");
            drop(entered);
            drop(send);
        });

        let records = lock_unpoisoned(&capture.0);
        let send = records
            .iter()
            .find(|record| record.event == "propagation_send")
            .unwrap();
        let receive = records
            .iter()
            .find(|record| record.event == "propagation_receive")
            .expect("accepted send must correlate with ingress observed before span close");
        assert_eq!(receive.parent_id.as_ref(), Some(&send.id));
        let ingress = records
            .iter()
            .find(|record| record.event == "async_ingress")
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
                record.event == "propagation_send"
                    && record.fields.destination_federate.as_deref() == Some("b")
            })
            .unwrap_or_else(|| panic!("missing serialized send record: {records:#?}"));
        let receives = records
            .iter()
            .filter(|record| record.event == "propagation_receive")
            .collect::<Vec<_>>();
        assert_eq!(receives.len(), 1);
        assert_eq!(receives[0].parent_id.as_ref(), Some(&send.id));
        assert_eq!(receives[0].fields.source.as_deref(), Some("during"));
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
            let send = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = tracing::field::Empty);
            let entered = send.enter();
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            send.record("outcome", "failed");
            drop(entered);
            drop(send);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| record.event != "propagation_receive" && record.event != "causal_link"));
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
            let first = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = tracing::field::Empty);
            let second = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = tracing::field::Empty);
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            first.record("outcome", "accepted");
            second.record("outcome", "accepted");
            drop(first);
            drop(second);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| record.event != "propagation_receive" && record.event != "causal_link"));
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
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = "accepted");
            let open = tracing::trace_span!(target: TRACE_TARGET, "propagation_send", event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = tracing::field::Empty);
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            open.record("outcome", "accepted");
            drop(open);
        });

        assert!(lock_unpoisoned(&capture.0)
            .iter()
            .all(|record| record.event != "propagation_receive" && record.event != "causal_link"));
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
            tracing::trace!(target: TRACE_TARGET, event = "propagation_send", enclave = "source", kind = "logical", destination = "e0", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "async_ingress", enclave = "e0", kind = "logical", action_key = "a0", logical_ns = 1_u64, microstep = 0_u64, destination_logical_ns = 1_u64, destination_microstep = 0_u64, outcome = "accepted");
            tracing::trace!(target: TRACE_TARGET, event = "action_schedule", enclave = "e0", action_key = "a0", old_logical_ns = 1_u64, old_microstep = 0_u64, destination_logical_ns = 5_u64, destination_microstep = 0_u64, outcome = "rebased");
            let reaction = tracing::trace_span!(target: TRACE_TARGET, "reaction_execute", event = "reaction_execute", enclave = "e0", reaction_key = "r0", logical_ns = 5_u64, microstep = 0_u64, state = "begin");
            drop(reaction);
        });

        let records = lock_unpoisoned(&capture.0);
        let receive = records
            .iter()
            .find(|record| record.event == "propagation_receive")
            .unwrap();
        let reaction = records
            .iter()
            .find(|record| record.event == "reaction_execute")
            .unwrap();
        assert!(records.iter().any(|record| {
            record.event == "causal_link"
                && record.fields.source.as_deref() == Some(receive.id.0.as_str())
                && record.fields.destination.as_deref() == Some(reaction.id.0.as_str())
        }));
    }
}
