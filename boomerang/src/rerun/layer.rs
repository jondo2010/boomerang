use std::collections::{BTreeMap, HashMap};
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
                .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
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
                "propagation_send" | "async_ingress" => registration
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
            "propagation_send"
                if record.fields.kind.as_deref() == Some("logical")
                    && record.fields.outcome.as_deref() == Some("accepted") =>
            {
                if let (Some((action, _)), Some(tag), Some(destination)) = (
                    topology,
                    CompleteTag::from_fields(&record.fields),
                    record.fields.destination.clone(),
                ) {
                    correlation.insert_send(
                        PropagationKey {
                            enclave: destination,
                            action,
                            tag,
                        },
                        record.id.clone(),
                    );
                }
                Vec::new()
            }
            "async_ingress"
                if record.fields.kind.as_deref() == Some("logical")
                    && record.fields.outcome.as_deref() == Some("accepted") =>
            {
                let (Some((action, reactions)), Some(tag), Some(enclave)) = (
                    topology,
                    CompleteTag::from_fields(&record.fields),
                    record.fields.enclave.clone(),
                ) else {
                    return Vec::new();
                };
                let Some(send) = correlation.take_send(&PropagationKey {
                    enclave: enclave.clone(),
                    action,
                    tag: tag.clone(),
                }) else {
                    return Vec::new();
                };
                let Some(receive_id) = self.next_id(&enclave) else {
                    return Vec::new();
                };
                let mut receive_fields = record.fields.clone();
                receive_fields.event = Some("propagation_receive".to_owned());
                let receive = TraceRecord {
                    entity_path: format!(
                        "/propagation/receives/{}",
                        super::entities::escape_entity_segment(&receive_id.0)
                    ),
                    event: "propagation_receive".to_owned(),
                    id: receive_id.clone(),
                    parent_id: Some(send.clone()),
                    timepoint: record.timepoint.clone(),
                    microstep: record.microstep,
                    duration_ns: None,
                    terminal_state: None,
                    fields: receive_fields,
                };
                for reaction in reactions {
                    correlation.insert_predecessor(
                        ReactionKey {
                            enclave: enclave.clone(),
                            reaction,
                            tag: tag.clone(),
                        },
                        receive_id.clone(),
                    );
                }
                let mut derived = vec![receive];
                if let Some(link) = self.causal_link(&enclave, &send, &receive_id, record) {
                    derived.push(link);
                }
                derived
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
                    .take_predecessor(&ReactionKey {
                        enclave: enclave.clone(),
                        reaction,
                        tag,
                    })
                    .and_then(|receive| self.causal_link(&enclave, &receive, &record.id, record))
                    .into_iter()
                    .collect()
            }
            "tag_process" => {
                if let (Some(enclave), Some(tag)) = (
                    record.fields.enclave.as_deref(),
                    CompleteTag::from_fields(&record.fields),
                ) {
                    correlation.cleanup_through(enclave, &tag);
                }
                Vec::new()
            }
            _ => Vec::new(),
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
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
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PropagationKey {
    enclave: String,
    action: String,
    tag: CompleteTag,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReactionKey {
    enclave: String,
    reaction: String,
    tag: CompleteTag,
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
    sends: BTreeMap<PropagationKey, PendingResolution>,
    predecessors: BTreeMap<ReactionKey, PendingResolution>,
}

impl CorrelationState {
    fn advance(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
        let minimum = self.epoch.saturating_sub(MAX_CORRELATION_AGE);
        self.sends.retain(|_, value| value.epoch() >= minimum);
        self.predecessors
            .retain(|_, value| value.epoch() >= minimum);
        self.enforce_bound();
    }

    fn insert_send(&mut self, key: PropagationKey, id: TraceId) {
        insert_resolution(&mut self.sends, key, id, self.epoch);
        self.enforce_bound();
    }

    fn take_send(&mut self, key: &PropagationKey) -> Option<TraceId> {
        self.sends
            .remove(key)
            .and_then(PendingResolution::unique_id)
    }

    fn insert_predecessor(&mut self, key: ReactionKey, id: TraceId) {
        insert_resolution(&mut self.predecessors, key, id, self.epoch);
        self.enforce_bound();
    }

    fn take_predecessor(&mut self, key: &ReactionKey) -> Option<TraceId> {
        self.predecessors
            .remove(key)
            .and_then(PendingResolution::unique_id)
    }

    fn cleanup_through(&mut self, enclave: &str, tag: &CompleteTag) {
        self.sends
            .retain(|key, _| key.enclave != enclave || key.tag > *tag);
        self.predecessors
            .retain(|key, _| key.enclave != enclave || key.tag > *tag);
    }

    fn enforce_bound(&mut self) {
        while self.sends.len() + self.predecessors.len() > MAX_PENDING_CORRELATIONS {
            let oldest_send = self
                .sends
                .iter()
                .map(|(key, value)| (value.epoch(), key.clone()));
            let oldest_predecessor = self
                .predecessors
                .iter()
                .map(|(key, value)| (value.epoch(), key.clone()));
            match (oldest_send.min(), oldest_predecessor.min()) {
                (Some(send), Some(predecessor)) if send.0 <= predecessor.0 => {
                    self.sends.remove(&send.1);
                }
                (Some(_), Some(predecessor)) => {
                    self.predecessors.remove(&predecessor.1);
                }
                (Some(send), None) => {
                    self.sends.remove(&send.1);
                }
                (None, Some(predecessor)) => {
                    self.predecessors.remove(&predecessor.1);
                }
                (None, None) => break,
            }
        }
    }
}

fn insert_resolution<K: Ord>(
    map: &mut BTreeMap<K, PendingResolution>,
    key: K,
    id: TraceId,
    epoch: u64,
) {
    map.entry(key)
        .and_modify(|resolution| *resolution = PendingResolution::Ambiguous { epoch })
        .or_insert(PendingResolution::Unique { id, epoch });
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
            let span_state = Arc::new(SpanState::new(
                trace_id,
                parent.map(|parent| parent.id.clone()),
                fields,
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
    timing: Mutex<SpanTiming>,
}

#[derive(Default)]
struct SpanTiming {
    entered: HashMap<ThreadId, Vec<Instant>>,
    accumulated: Duration,
}

impl SpanState {
    fn new(id: TraceId, parent_id: Option<TraceId>, fields: TraceFields) -> Self {
        Self {
            id,
            parent_id,
            fields: Mutex::new(fields),
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
                    enclave: "e0".to_owned(),
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

        state.cleanup_through(
            "e0",
            &CompleteTag {
                logical_ns: u64::MAX,
                microstep: u64::MAX,
            },
        );
        assert!(state.sends.is_empty());
        assert!(state.predecessors.is_empty());
    }

    #[test]
    fn duplicate_pending_predecessors_are_ambiguous_not_first_wins() {
        let mut state = CorrelationState::default();
        state.advance();
        let key = ReactionKey {
            enclave: "e0".to_owned(),
            reaction: "reaction".to_owned(),
            tag: CompleteTag {
                logical_ns: 1,
                microstep: 0,
            },
        };
        state.insert_predecessor(key.clone(), TraceId("first".to_owned()));
        state.insert_predecessor(key.clone(), TraceId("second".to_owned()));
        assert_eq!(state.take_predecessor(&key), None);
    }
}
