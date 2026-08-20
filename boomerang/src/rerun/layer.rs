use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::entities::{
    entity_path, TraceFields, TraceId, TraceRecord, TraceTimePoint, TraceWriter, TraceWriterError,
};
use super::session::SessionState;

const TRACE_TARGET: &str = "boomerang::trace";
const INTERNAL_TARGET: &str = "boomerang::rerun_internal";

/// A composable tracing layer that maps Boomerang's structured runtime facts to Rerun.
#[derive(Clone)]
pub struct RerunLayer {
    recording: rerun::RecordingStream,
    state: SessionState,
    source_id: Arc<str>,
    started: Instant,
    next_id: Arc<AtomicU64>,
    writer: Arc<dyn TraceWriter>,
}

impl RerunLayer {
    pub(super) fn new(
        recording: rerun::RecordingStream,
        state: SessionState,
        source_id: Arc<str>,
        writer: Arc<dyn TraceWriter>,
        started: Instant,
        next_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            recording,
            state,
            source_id,
            started,
            next_id,
            writer,
        }
    }

    fn next_id(&self, enclave: &str) -> TraceId {
        TraceId::new(
            &self.source_id,
            enclave,
            self.next_id.fetch_add(1, Ordering::Relaxed),
        )
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
        .unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            Err(TraceWriterError(format!(
                "trace writer panicked: {message}"
            )))
        });
        if let Err(error) = result {
            self.state.disable_on_error(&error);
        }
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
        self.write(TraceRecord {
            id: self.next_id("unknown"),
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
        Some(TraceRecord {
            id: id.unwrap_or_else(|| self.next_id(enclave)),
            parent_id,
            entity_path: entity_path(&fields, &event),
            event,
            timepoint: self.timepoint(&fields),
            microstep: fields.microstep,
            duration_ns: duration.map(|value| u64::try_from(value.as_nanos()).unwrap_or(u64::MAX)),
            terminal_state: None,
            fields,
        })
    }
}

impl<S> Layer<S> for RerunLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
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
        let span_state = Arc::new(SpanState::new(
            self.next_id(enclave),
            parent.map(|parent| parent.id.clone()),
            fields,
        ));
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(span_state);
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let extensions = span.extensions();
        if let Some(state) = extensions.get::<Arc<SpanState>>() {
            values.record(&mut *state.fields.lock().unwrap());
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(state) = span_state(id, &ctx) {
            state.enter();
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(state) = span_state(id, &ctx) {
            state.exit();
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
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
            self.write(record);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
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
            self.write(record);
        }
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
        self.fields.lock().unwrap().clone()
    }

    fn enter(&self) {
        self.timing
            .lock()
            .unwrap()
            .entered
            .entry(std::thread::current().id())
            .or_default()
            .push(Instant::now());
    }

    fn exit(&self) {
        let now = Instant::now();
        let mut timing = self.timing.lock().unwrap();
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
        let mut timing = self.timing.lock().unwrap();
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
            "level" => self.level = Some(value),
            "state" => self.state = Some(value),
            "value_type" => self.value_type = Some(value),
            "outcome" => self.outcome = Some(value),
            "error" => self.error = Some(value),
            _ => {}
        }
    }
}
