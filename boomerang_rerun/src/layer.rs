use std::collections::{HashMap, HashSet};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::entities::{
    bounded_fragment, compact_runtime_key, escape_entity_segment, RegistrationSnapshot,
};
use super::session::SessionState;

const TRACE_TARGET: &str = "boomerang::trace";

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

#[derive(Clone)]
/// Subscriber layer that maps Boomerang tracing callbacks to Rerun archetypes.
pub(super) struct RerunLayer {
    recording: rerun::RecordingStream,
    state: SessionState,
    adapter: AdapterState,
}

#[derive(Clone)]
pub(super) struct AdapterState {
    pub(super) registration: Arc<RegistrationState>,
    named_series: Arc<RwLock<HashSet<String>>>,
}

#[derive(Default)]
pub(super) struct RegistrationState {
    claimed: AtomicBool,
    snapshot: OnceLock<RegistrationSnapshot>,
}

impl RegistrationState {
    pub(super) fn claim(&self) -> bool {
        self.claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn get(&self) -> Option<&RegistrationSnapshot> {
        self.snapshot.get()
    }

    pub(super) fn initialize(&self, snapshot: RegistrationSnapshot) -> bool {
        self.snapshot.set(snapshot).is_ok()
    }
}

impl Default for AdapterState {
    fn default() -> Self {
        Self {
            registration: Arc::new(RegistrationState::default()),
            named_series: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

#[derive(Clone, Copy)]
struct TimePoint {
    logical_ns: Option<i64>,
}

enum SpanKind {
    Scheduler {
        state: String,
    },
    Tag {
        terminal: bool,
        state: String,
    },
    Reaction {
        reactor: String,
        reaction_key: Option<String>,
        reaction: String,
        level: u64,
        state: String,
    },
    Wait {
        state: String,
    },
    Send {
        kind: String,
        destination: Option<String>,
        destination_federate: Option<String>,
        action_key: String,
        action: String,
        value_type: String,
        value_size: u64,
        outcome: OnceLock<String>,
    },
}

struct SpanState {
    context: SpanContext,
    path: String,
    entity_label: Option<String>,
    time: TimePoint,
    timing: Mutex<SpanTiming>,
    kind: SpanKind,
}

#[derive(Clone)]
struct SpanContext {
    federate: Option<String>,
    enclave: String,
    tag: Option<LogicalTag>,
}

#[derive(Clone, Copy)]
struct LogicalTag {
    logical_ns: u64,
    microstep: u64,
}

impl SpanContext {
    fn untimed(federate: Option<String>, enclave: String) -> Self {
        Self {
            federate,
            enclave,
            tag: None,
        }
    }

    fn logical(federate: Option<String>, enclave: String, tag: LogicalTag) -> Self {
        Self {
            federate,
            enclave,
            tag: Some(tag),
        }
    }

    fn federate(&self) -> Option<&str> {
        self.federate.as_deref()
    }

    fn enclave(&self) -> &str {
        &self.enclave
    }

    fn tag(&self) -> Option<LogicalTag> {
        self.tag
    }

    fn logical_ns(&self) -> Option<u64> {
        self.tag().map(|tag| tag.logical_ns)
    }

    fn microstep(&self) -> Option<u64> {
        self.tag().map(|tag| tag.microstep)
    }
}

struct ResolvedPath {
    path: String,
    entity_label: Option<String>,
}

#[derive(Default)]
struct SpanTiming {
    active: HashMap<ThreadId, (Instant, usize)>,
    total: Duration,
}

impl SpanTiming {
    fn enter(&mut self) {
        let active = self
            .active
            .entry(std::thread::current().id())
            .or_insert_with(|| (Instant::now(), 0));
        active.1 = active.1.saturating_add(1);
    }

    fn exit(&mut self) {
        let thread = std::thread::current().id();
        let started = self.active.get_mut(&thread).and_then(|(started, depth)| {
            *depth = depth.checked_sub(1)?;
            (*depth == 0).then_some(*started)
        });
        if let Some(started) = started {
            self.active.remove(&thread);
            self.total += started.elapsed();
        }
    }
}

#[derive(Clone, Debug)]
enum Captured {
    Text(String),
    U64(u64),
    Bool(bool),
}

#[derive(Default)]
struct CallbackFields(HashMap<&'static str, Captured>);

impl CallbackFields {
    fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(Captured::Text(value)) => Some(value),
            _ => None,
        }
    }

    fn u64(&self, name: &str) -> Option<u64> {
        match self.0.get(name) {
            Some(Captured::U64(value)) => Some(*value),
            _ => None,
        }
    }

    fn boolean(&self, name: &str) -> Option<bool> {
        match self.0.get(name) {
            Some(Captured::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    fn required_text(&self, name: &str) -> Result<String, String> {
        self.text(name)
            .map(str::to_owned)
            .ok_or_else(|| format!("missing or invalid `{name}`"))
    }

    fn required_u64(&self, name: &str) -> Result<u64, String> {
        self.u64(name)
            .ok_or_else(|| format!("missing or invalid `{name}`"))
    }
}

fn required_discriminator(
    fields: &CallbackFields,
    name: &str,
    allowed: &[&str],
) -> Result<String, String> {
    let value = fields.required_text(name)?;
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(format!("invalid `{name}` discriminator `{value}`"))
    }
}

impl Visit for CallbackFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name(), Captured::Text(value.to_owned()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name(), Captured::U64(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name(), Captured::Bool(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let value = format!("{value:?}");
        self.0.insert(
            field.name(),
            Captured::Text(
                value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(&value)
                    .to_owned(),
            ),
        );
    }
}

impl RerunLayer {
    pub(super) fn new(
        recording: rerun::RecordingStream,
        state: SessionState,
        adapter: AdapterState,
    ) -> Self {
        Self {
            recording,
            state,
            adapter,
        }
    }

    fn isolate(&self, callback: impl FnOnce()) {
        if catch_unwind(AssertUnwindSafe(callback)).is_err() {
            self.state
                .disable_on_error(&"Rerun trace callback panicked");
        }
    }

    fn timepoint(&self, logical_ns: Option<u64>) -> TimePoint {
        TimePoint {
            logical_ns: logical_ns.and_then(|value| i64::try_from(value).ok()),
        }
    }

    fn write(&self, path: &str, time: TimePoint, value: &dyn rerun::AsComponents) {
        if !self.state.is_enabled() {
            return;
        }
        self.recording.reset_time();
        let _reset = TimeReset(&self.recording);
        let mut point = rerun::TimePoint::default();
        if let Some(logical) = time.logical_ns {
            point.insert_cell("logical", rerun::TimeCell::from_duration_nanos(logical));
        }
        self.recording.set_timepoint(point);
        if let Err(error) = self.recording.log(path, value) {
            self.state.disable_on_error(&error);
        }
    }

    fn write_measure(
        &self,
        path: &str,
        entity_label: Option<&str>,
        time: TimePoint,
        value: &dyn rerun::AsComponents,
    ) {
        if self
            .adapter
            .named_series
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(path)
        {
            self.write(path, time, value);
            return;
        }
        let first_measure = self
            .adapter
            .named_series
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path.to_owned());
        if first_measure {
            let name = compact_series_name(path, entity_label);
            if let Err(error) = self
                .recording
                .log_static(path, &rerun::SeriesPoints::new().with_names([name]))
            {
                self.state.disable_on_error(&error);
                return;
            }
        }
        self.write(path, time, value);
    }

    fn diagnostic(&self, event: Option<&str>, error: impl fmt::Display) {
        let message = match event {
            Some(event) => format!("invalid `{event}` trace annotation: {error}"),
            None => format!("invalid trace annotation: {error}"),
        };
        let archetype = common(
            "boomerang.SchemaDiagnostic",
            "schema_diagnostic",
            None,
            None,
            None,
            None,
        )
        .with_component::<rerun::components::Text>("boomerang.trace.error", [message.as_str()]);
        let text_log = rerun::TextLog::new(message).with_level(rerun::TextLogLevel::ERROR);
        self.write(
            "/diagnostics/schema",
            self.timepoint(None),
            &Combined::new([&archetype, &text_log]),
        );
    }

    fn with_context<S, T>(
        &self,
        ctx: &Context<'_, S>,
        explicit_parent: Option<&Id>,
        contextual: bool,
        callback: impl FnOnce(Option<&SpanState>) -> T,
    ) -> T
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let parent = match explicit_parent {
            Some(id) => ctx.span(id),
            None if contextual => ctx.lookup_current(),
            None => None,
        };
        let Some(parent) = parent else {
            return callback(None);
        };
        let extensions = parent.extensions();
        callback(extensions.get::<SpanState>())
    }

    fn inherited_tag(
        fields: &CallbackFields,
        inherited: Option<LogicalTag>,
    ) -> Result<Option<LogicalTag>, String> {
        match (fields.u64("logical_ns"), fields.u64("microstep")) {
            (Some(logical_ns), Some(microstep)) => Ok(Some(LogicalTag {
                logical_ns,
                microstep,
            })),
            (None, None) => Ok(inherited),
            _ => Err("`logical_ns` and `microstep` must be recorded together".into()),
        }
    }

    fn path(
        &self,
        federate: Option<&str>,
        enclave: Option<&str>,
        kind: &'static str,
        identity: &str,
        event: &str,
    ) -> ResolvedPath {
        let registered = enclave.and_then(|enclave| {
            self.adapter.registration.get().and_then(|registration| {
                registration
                    .resolve(federate, enclave, kind, identity)
                    .map(|entity| (entity.path.clone(), entity.label.clone()))
            })
        });
        if let Some((path, label)) = registered {
            ResolvedPath {
                path: format!("{path}/{}", escape_entity_segment(event)),
                entity_label: Some(label),
            }
        } else {
            let root = match (federate, enclave) {
                (Some(federate), Some(enclave)) => format!(
                    "/federates/{}/enclaves/{}",
                    escape_entity_segment(federate),
                    escape_entity_segment(enclave)
                ),
                (None, Some(enclave)) => {
                    format!("/enclaves/{}", escape_entity_segment(enclave))
                }
                _ => "/runtime/unresolved".to_owned(),
            };
            ResolvedPath {
                path: format!("{root}/{}", escape_entity_segment(event)),
                entity_label: None,
            }
        }
    }
}

impl<S> Layer<S> for RerunLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            let mut fields = CallbackFields::default();
            attrs.record(&mut fields);
            let Some(event) = fields.text("event").map(str::to_owned) else {
                self.diagnostic(None, "missing or invalid `event`");
                return;
            };
            let parent = self.with_context(&ctx, attrs.parent(), attrs.is_contextual(), |parent| {
                parent.map(|span| span.context.clone())
            });
            let federate = fields.text("federate").map(str::to_owned).or_else(|| {
                parent
                    .as_ref()
                    .and_then(|context| context.federate().map(str::to_owned))
            });
            let enclave = fields
                .text("enclave")
                .map(str::to_owned)
                .or_else(|| parent.as_ref().map(|context| context.enclave().to_owned()));
            let tag = match Self::inherited_tag(&fields, parent.and_then(|context| context.tag())) {
                Ok(tag) => tag,
                Err(error) => {
                    self.diagnostic(Some(&event), error);
                    return;
                }
            };
            let built = (|| -> Result<(SpanKind, &'static str, String), String> {
                Ok(match event.as_str() {
                    "scheduler_thread" => (
                        SpanKind::Scheduler {
                            state: required_discriminator(&fields, "state", &["running"])?,
                        },
                        "scheduler",
                        "scheduler".into(),
                    ),
                    "tag_process" => (
                        SpanKind::Tag {
                            terminal: fields
                                .boolean("terminal")
                                .ok_or("missing or invalid `terminal`")?,
                            state: required_discriminator(&fields, "state", &["processing"])?,
                        },
                        "scheduler",
                        "scheduler".into(),
                    ),
                    "reaction_execute" => {
                        let reaction = fields.required_text("reaction")?;
                        let reaction_key = fields.text("reaction_key").map(str::to_owned);
                        (
                            SpanKind::Reaction {
                                reactor: fields.required_text("reactor")?,
                                reaction_key: reaction_key.clone(),
                                reaction: reaction.clone(),
                                level: fields.required_u64("level")?,
                                state: required_discriminator(&fields, "state", &["begin"])?,
                            },
                            "reaction",
                            reaction_key.unwrap_or(reaction),
                        )
                    }
                    "coordination_wait" => (
                        SpanKind::Wait {
                            state: required_discriminator(&fields, "state", &["waiting"])?,
                        },
                        "scheduler",
                        "scheduler".into(),
                    ),
                    "propagation_send" => {
                        let kind = fields.required_text("kind")?;
                        if !matches!(kind.as_str(), "logical" | "physical") {
                            return Err(format!("invalid `kind` discriminator `{kind}`"));
                        }
                        enclave.as_deref().ok_or("missing or invalid `enclave`")?;
                        match (
                            fields.text("destination"),
                            fields.text("destination_federate"),
                        ) {
                            (Some(_), None) => {}
                            (None, Some(_)) if kind == "logical" => {}
                            (None, None) => return Err("missing propagation destination".into()),
                            _ => return Err("ambiguous propagation destination".into()),
                        }
                        let initial_outcome = fields
                            .text("outcome")
                            .map(|_| {
                                required_discriminator(&fields, "outcome", &["accepted", "failed"])
                            })
                            .transpose()?;
                        let outcome = OnceLock::new();
                        if let Some(initial_outcome) = initial_outcome {
                            let _ = outcome.set(initial_outcome);
                        }
                        let action_key = fields.required_text("action_key")?;
                        (
                            SpanKind::Send {
                                kind,
                                destination: fields.text("destination").map(str::to_owned),
                                destination_federate: fields
                                    .text("destination_federate")
                                    .map(str::to_owned),
                                action_key: action_key.clone(),
                                action: fields.required_text("action")?,
                                value_type: fields.required_text("value_type")?,
                                value_size: fields.required_u64("value_size")?,
                                outcome,
                            },
                            "scheduler",
                            "scheduler".into(),
                        )
                    }
                    _ => return Err(format!("unsupported span event `{event}`")),
                })
            })();
            let (kind, entity_kind, identity) = match built {
                Ok(value) => value,
                Err(error) => {
                    self.diagnostic(Some(&event), error);
                    return;
                }
            };
            let Some(enclave) = enclave else {
                let error = if matches!(kind, SpanKind::Send { .. }) {
                    "missing or invalid source enclave"
                } else {
                    "missing or invalid `enclave`"
                };
                self.diagnostic(Some(&event), error);
                return;
            };
            let context = match &kind {
                SpanKind::Tag { .. } | SpanKind::Reaction { .. } | SpanKind::Wait { .. } => {
                    let Some(tag) = tag else {
                        self.diagnostic(Some(&event), "missing logical tag context");
                        return;
                    };
                    SpanContext::logical(federate, enclave, tag)
                }
                SpanKind::Send { kind, .. } if kind == "logical" => {
                    let Some(tag) = tag else {
                        self.diagnostic(Some(&event), "missing logical tag context");
                        return;
                    };
                    SpanContext::logical(federate, enclave, tag)
                }
                SpanKind::Scheduler { .. } | SpanKind::Send { .. } => {
                    SpanContext::untimed(federate, enclave)
                }
            };
            let resolved_path = self.path(
                context.federate(),
                Some(context.enclave()),
                entity_kind,
                &identity,
                &event,
            );
            let phase = match &kind {
                SpanKind::Tag { .. } => Some("processing tag"),
                SpanKind::Reaction { .. } => Some("executing reaction"),
                SpanKind::Wait { .. } => Some("waiting for coordination"),
                _ => None,
            };
            let span = SpanState {
                time: self.timepoint(context.logical_ns()),
                context,
                path: resolved_path.path,
                entity_label: resolved_path.entity_label,
                timing: Mutex::new(SpanTiming::default()),
                kind,
            };
            if let Some(scope) = ctx.span(id) {
                scope.extensions_mut().insert(span);
            }
            if let Some(phase) = phase {
                let _ = with_span_state(id, &ctx, |span| {
                    self.write(
                        &span.path,
                        self.timepoint(None),
                        &rerun::StateChange::single(phase),
                    );
                });
            }
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        self.isolate(|| {
            let mut fields = CallbackFields::default();
            values.record(&mut fields);
            if let Some(outcome) = fields.text("outcome") {
                let duplicate = with_span_state(id, &ctx, |span| match &span.kind {
                    SpanKind::Send {
                        outcome: current, ..
                    } => current.set(outcome.to_owned()).is_err(),
                    _ => false,
                })
                .unwrap_or(false);
                if duplicate {
                    self.diagnostic(Some("propagation_send"), "duplicate `outcome` record");
                }
            }
        });
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        self.isolate(|| {
            let mut fields = CallbackFields::default();
            event.record(&mut fields);
            let Some(name) = fields.text("event").map(str::to_owned) else {
                self.diagnostic(None, "missing or invalid `event`");
                return;
            };
            let result = self.with_context(&ctx, event.parent(), event.is_contextual(), |parent| {
                self.emit_event(&name, &fields, parent)
            });
            if let Err(error) = result {
                self.diagnostic(Some(&name), error);
            }
        });
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            let _ = with_span_state(id, &ctx, |span| {
                span.timing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .enter();
            });
        });
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            let _ = with_span_state(id, &ctx, |span| {
                span.timing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .exit();
            });
        });
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            let _ = with_span_state(&id, &ctx, |span| {
                let duration = saturating_i64(
                    span.timing
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .total
                        .as_nanos(),
                ) as u64;
                let kind = &span.kind;
                if let SpanKind::Send { outcome, .. } = kind {
                    match outcome.get().map(String::as_str) {
                        Some("accepted" | "failed") => {}
                        Some(outcome) => {
                            self.diagnostic(
                                Some("propagation_send"),
                                format!("invalid `outcome` discriminator `{outcome}`"),
                            );
                            return;
                        }
                        None => {
                            self.diagnostic(
                                Some("propagation_send"),
                                "missing or invalid `outcome`",
                            );
                            return;
                        }
                    }
                }
                let mut payload = match kind {
                    SpanKind::Scheduler { state } => common(
                        "boomerang.SchedulerRunning",
                        "scheduler_thread",
                        span.context.federate(),
                        Some(span.context.enclave()),
                        None,
                        None,
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.state",
                        [state.as_str()],
                    ),
                    SpanKind::Tag { terminal, state } => common(
                        "boomerang.TagProcessing",
                        "tag_process",
                        span.context.federate(),
                        Some(span.context.enclave()),
                        span.context.logical_ns(),
                        span.context.microstep(),
                    )
                    .with_component_from_data("boomerang.trace.terminal", bool_array(*terminal))
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.state",
                        [state.as_str()],
                    ),
                    SpanKind::Reaction {
                        reactor,
                        reaction_key,
                        reaction,
                        level,
                        state,
                    } => {
                        let mut value = common(
                            "boomerang.ReactionExecution",
                            "reaction_execute",
                            span.context.federate(),
                            Some(span.context.enclave()),
                            span.context.logical_ns(),
                            span.context.microstep(),
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.reactor",
                            [reactor.as_str()],
                        );
                        if let Some(key) = reaction_key {
                            value = value.with_component::<rerun::components::Text>(
                                "boomerang.trace.reaction_key",
                                [key.as_str()],
                            );
                        }
                        value
                            .with_component::<rerun::components::Text>(
                                "boomerang.trace.reaction",
                                [reaction.as_str()],
                            )
                            .with_component_from_data("boomerang.trace.level", u64_array(*level))
                            .with_component::<rerun::components::Text>(
                                "boomerang.trace.state",
                                [state.as_str()],
                            )
                    }
                    SpanKind::Wait { state } => common(
                        "boomerang.CoordinationWait",
                        "coordination_wait",
                        span.context.federate(),
                        Some(span.context.enclave()),
                        span.context.logical_ns(),
                        span.context.microstep(),
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.state",
                        [state.as_str()],
                    ),
                    SpanKind::Send {
                        kind,
                        destination,
                        destination_federate,
                        action_key,
                        action,
                        value_type,
                        value_size,
                        outcome,
                    } => {
                        let archetype = if destination_federate.is_some() {
                            "boomerang.PropagationSerializedSend"
                        } else if kind == "physical" {
                            "boomerang.PropagationPhysicalSend"
                        } else {
                            "boomerang.PropagationLogicalSend"
                        };
                        let mut value = common(
                            archetype,
                            "propagation_send",
                            span.context.federate(),
                            Some(span.context.enclave()),
                            span.context.logical_ns(),
                            span.context.microstep(),
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.action_key",
                            [action_key.as_str()],
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.action",
                            [action.as_str()],
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.value_type",
                            [value_type.as_str()],
                        )
                        .with_component_from_data(
                            "boomerang.trace.value_size",
                            u64_array(*value_size),
                        );
                        if let Some(destination) = destination {
                            value = value.with_component::<rerun::components::Text>(
                                "boomerang.trace.destination",
                                [destination.as_str()],
                            );
                        }
                        if let Some(destination) = destination_federate {
                            value = value.with_component::<rerun::components::Text>(
                                "boomerang.trace.destination_federate",
                                [destination.as_str()],
                            );
                        }
                        if let Some(outcome) = outcome.get() {
                            value = value.with_component::<rerun::components::Text>(
                                "boomerang.trace.outcome",
                                [outcome.as_str()],
                            );
                        }
                        value
                    }
                };
                payload = payload
                    .with_component_from_data("boomerang.trace.duration_ns", u64_array(duration));
                let measure = match kind {
                    SpanKind::Send { value_size, .. } => *value_size as f64,
                    SpanKind::Tag { terminal, .. } => u8::from(*terminal) as f64,
                    SpanKind::Scheduler { .. }
                    | SpanKind::Reaction { .. }
                    | SpanKind::Wait { .. } => duration as f64,
                };
                {
                    let scalar = rerun::Scalars::new([measure]);
                    self.write_measure(
                        &span.path,
                        span.entity_label.as_deref(),
                        span.time,
                        &Combined::new([&payload, &scalar]),
                    );
                }
                if matches!(
                    kind,
                    SpanKind::Tag { .. } | SpanKind::Reaction { .. } | SpanKind::Wait { .. }
                ) {
                    self.write(
                        &span.path,
                        self.timepoint(None),
                        &rerun::StateChange::clear_fields(),
                    );
                }
            });
        });
    }
}

impl RerunLayer {
    fn emit_event(
        &self,
        name: &str,
        f: &CallbackFields,
        parent: Option<&SpanState>,
    ) -> Result<(), String> {
        let federate = f
            .text("federate")
            .or_else(|| parent.and_then(|p| p.context.federate()));
        let enclave = f
            .text("enclave")
            .or_else(|| parent.map(|p| p.context.enclave()));
        let logical = f
            .u64("logical_ns")
            .or_else(|| parent.and_then(|p| p.context.logical_ns()));
        let microstep = f
            .u64("microstep")
            .or_else(|| parent.and_then(|p| p.context.microstep()));
        let time = self.timepoint(logical);
        let (path, payload) = match name {
            "async_ingress" => {
                let kind = f.required_text("kind")?;
                if kind == "logical" || kind == "physical" {
                    let action_key = f.required_text("action_key")?;
                    let action = f.required_text("action")?;
                    let logical_ns = logical.ok_or("missing or invalid `logical_ns`")?;
                    let microstep = microstep.ok_or("missing or invalid `microstep`")?;
                    let destination_logical_ns = f.required_u64("destination_logical_ns")?;
                    let destination_microstep = f.required_u64("destination_microstep")?;
                    let value_type = f.required_text("value_type")?;
                    let value_size = f.required_u64("value_size")?;
                    let outcome = required_discriminator(
                        f,
                        "outcome",
                        &["accepted", "ignored_past", "failed"],
                    )?;
                    let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                    let accepted_logical = kind == "logical" && outcome == "accepted";
                    let event_name = if accepted_logical {
                        "propagation_receive"
                    } else {
                        name
                    };
                    let path =
                        self.path(federate, Some(enclave), "action", &action_key, event_name);
                    let archetype = if accepted_logical {
                        "boomerang.PropagationReceive"
                    } else if kind == "logical" {
                        "boomerang.LogicalIngress"
                    } else {
                        "boomerang.PhysicalIngress"
                    };
                    let payload = common(
                        archetype,
                        event_name,
                        federate,
                        Some(enclave),
                        Some(logical_ns),
                        Some(microstep),
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.action_key",
                        [action_key.as_str()],
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.action",
                        [action.as_str()],
                    )
                    .with_component_from_data(
                        "boomerang.trace.destination_logical_ns",
                        u64_array(destination_logical_ns),
                    )
                    .with_component_from_data(
                        "boomerang.trace.destination_microstep",
                        u64_array(destination_microstep),
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.value_type",
                        [value_type.as_str()],
                    )
                    .with_component_from_data("boomerang.trace.value_size", u64_array(value_size))
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.outcome",
                        [outcome.as_str()],
                    );
                    (path, payload)
                } else if matches!(kind.as_str(), "shutdown" | "provisional_release") {
                    let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                    logical.ok_or("missing or invalid `logical_ns`")?;
                    microstep.ok_or("missing or invalid `microstep`")?;
                    let outcome = f.required_text("outcome")?;
                    let valid_outcome = match kind.as_str() {
                        "shutdown" => outcome == "accepted",
                        "provisional_release" => {
                            matches!(outcome.as_str(), "accepted" | "ignored_past")
                        }
                        _ => unreachable!(),
                    };
                    if !valid_outcome {
                        return Err(format!("invalid `outcome` discriminator `{outcome}`"));
                    }
                    (
                        self.path(federate, Some(enclave), "scheduler", "scheduler", name),
                        common(
                            "boomerang.ControlIngress",
                            name,
                            federate,
                            Some(enclave),
                            logical,
                            microstep,
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.kind",
                            [kind.as_str()],
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.outcome",
                            [outcome.as_str()],
                        ),
                    )
                } else {
                    return Err(format!("invalid `kind` discriminator `{kind}`"));
                }
            }
            "action_schedule" => {
                let action_key = f.required_text("action_key")?;
                let outcome = f.required_text("outcome")?;
                let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                let archetype = if outcome == "startup" {
                    f.required_text("action")?;
                    f.required_u64("destination_logical_ns")?;
                    f.required_u64("destination_microstep")?;
                    f.required_text("value_type")?;
                    f.required_u64("value_size")?;
                    "boomerang.ActionStartup"
                } else if outcome == "rebased" {
                    f.required_u64("old_logical_ns")?;
                    f.required_u64("old_microstep")?;
                    f.required_u64("destination_logical_ns")?;
                    f.required_u64("destination_microstep")?;
                    "boomerang.ActionRebased"
                } else if outcome == "scheduled" {
                    logical.ok_or("missing or invalid `logical_ns`")?;
                    microstep.ok_or("missing or invalid `microstep`")?;
                    f.required_text("action")?;
                    f.required_u64("destination_logical_ns")?;
                    f.required_u64("destination_microstep")?;
                    f.required_text("value_type")?;
                    f.required_u64("value_size")?;
                    "boomerang.ActionScheduled"
                } else {
                    return Err(format!("invalid `outcome` discriminator `{outcome}`"));
                };
                if outcome == "startup" {
                    logical.ok_or("missing or invalid `logical_ns`")?;
                    microstep.ok_or("missing or invalid `microstep`")?;
                }
                let mut payload =
                    common(archetype, name, federate, Some(enclave), logical, microstep)
                        .with_component::<rerun::components::Text>(
                        "boomerang.trace.action_key",
                        [action_key.as_str()],
                    );
                for (component, field) in [
                    (
                        "boomerang.trace.destination_logical_ns",
                        "destination_logical_ns",
                    ),
                    (
                        "boomerang.trace.destination_microstep",
                        "destination_microstep",
                    ),
                    ("boomerang.trace.old_logical_ns", "old_logical_ns"),
                    ("boomerang.trace.old_microstep", "old_microstep"),
                    ("boomerang.trace.value_size", "value_size"),
                ] {
                    if let Some(value) = f.u64(field) {
                        payload = payload.with_component_from_data(component, u64_array(value));
                    }
                }
                for (component, field) in [
                    ("boomerang.trace.action", "action"),
                    ("boomerang.trace.value_type", "value_type"),
                ] {
                    if let Some(value) = f.text(field) {
                        payload =
                            payload.with_component::<rerun::components::Text>(component, [value]);
                    }
                }
                payload = payload.with_component::<rerun::components::Text>(
                    "boomerang.trace.outcome",
                    [outcome.as_str()],
                );
                (
                    self.path(federate, Some(enclave), "action", &action_key, name),
                    payload,
                )
            }
            "port_write" => {
                if !matches!(
                    parent.map(|parent| &parent.kind),
                    Some(SpanKind::Reaction { .. })
                ) {
                    return Err("missing reaction span context".into());
                }
                let port_key = f.required_text("port_key")?;
                let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                let outcome = f.required_text("outcome")?;
                if outcome != "mutable_access" {
                    return Err(format!("invalid `outcome` discriminator `{outcome}`"));
                }
                let mut payload = common(
                    "boomerang.PortWrite",
                    name,
                    federate,
                    Some(enclave),
                    logical,
                    microstep,
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.port_key",
                    [port_key.as_str()],
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.port",
                    [f.required_text("port")?.as_str()],
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.value_type",
                    [f.required_text("value_type")?.as_str()],
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.outcome",
                    [outcome.as_str()],
                );
                if let Some(SpanKind::Reaction {
                    reactor,
                    reaction_key,
                    reaction,
                    ..
                }) = parent.map(|parent| &parent.kind)
                {
                    payload = payload
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.reactor",
                            [reactor.as_str()],
                        )
                        .with_component::<rerun::components::Text>(
                            "boomerang.trace.reaction",
                            [reaction.as_str()],
                        );
                    if let Some(key) = reaction_key {
                        payload = payload.with_component::<rerun::components::Text>(
                            "boomerang.trace.reaction_key",
                            [key.as_str()],
                        );
                    }
                }
                (
                    self.path(federate, Some(enclave), "port", &port_key, name),
                    payload,
                )
            }
            "frontier_publish" => {
                let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                let state = f.required_text("state")?;
                let outcome = f.required_text("outcome")?;
                if !matches!(outcome.as_str(), "published" | "failed") {
                    return Err(format!("invalid `outcome` discriminator `{outcome}`"));
                }
                let archetype = match state.as_str() {
                    "candidate" => {
                        logical.ok_or("missing or invalid `logical_ns`")?;
                        microstep.ok_or("missing or invalid `microstep`")?;
                        "boomerang.FrontierCandidate"
                    }
                    "idle" | "finished" => "boomerang.FrontierState",
                    _ => return Err(format!("invalid `state` discriminator `{state}`")),
                };
                let payload = common(archetype, name, federate, Some(enclave), logical, microstep)
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.state",
                        [state.as_str()],
                    )
                    .with_component::<rerun::components::Text>(
                        "boomerang.trace.outcome",
                        [outcome.as_str()],
                    );
                (
                    self.path(federate, Some(enclave), "scheduler", "scheduler", name),
                    payload,
                )
            }
            "coordination_grant" | "tag_release" | "tag_complete" | "shutdown" => {
                let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                logical.ok_or("missing or invalid `logical_ns`")?;
                microstep.ok_or("missing or invalid `microstep`")?;
                let outcome = f.required_text("outcome")?;
                let valid_outcome = match name {
                    "coordination_grant" => matches!(
                        outcome.as_str(),
                        "granted" | "interrupted_local" | "interrupted_external"
                    ),
                    "tag_release" => matches!(outcome.as_str(), "accepted" | "failed"),
                    "tag_complete" => matches!(outcome.as_str(), "completed" | "failed"),
                    "shutdown" => outcome == "success",
                    _ => unreachable!(),
                };
                if !valid_outcome {
                    return Err(format!("invalid `outcome` discriminator `{outcome}`"));
                }
                if name == "tag_release" {
                    f.required_text("destination")?;
                }
                if name == "tag_complete" {
                    f.boolean("terminal")
                        .ok_or("missing or invalid `terminal`")?;
                }
                let archetype = match name {
                    "coordination_grant" => "boomerang.CoordinationGrant",
                    "tag_release" => "boomerang.TagRelease",
                    "tag_complete" => "boomerang.TagComplete",
                    _ => "boomerang.Shutdown",
                };
                let mut payload =
                    common(archetype, name, federate, Some(enclave), logical, microstep);
                for (component, field) in [
                    ("boomerang.trace.destination", "destination"),
                    ("boomerang.trace.state", "state"),
                    ("boomerang.trace.outcome", "outcome"),
                ] {
                    if let Some(value) = f.text(field) {
                        payload =
                            payload.with_component::<rerun::components::Text>(component, [value]);
                    }
                }
                if let Some(value) = f.boolean("terminal") {
                    payload = payload
                        .with_component_from_data("boomerang.trace.terminal", bool_array(value));
                }
                if name == "shutdown" {
                    let state = f.required_text("state")?;
                    if state != "complete" {
                        return Err(format!("invalid `state` discriminator `{state}`"));
                    }
                }
                (
                    self.path(federate, Some(enclave), "scheduler", "scheduler", name),
                    payload,
                )
            }
            "diagnostic" => {
                let enclave = enclave.ok_or("missing or invalid `enclave`")?;
                let error = f.required_text("error")?;
                let payload = common(
                    "boomerang.RuntimeDiagnostic",
                    name,
                    federate,
                    Some(enclave),
                    None,
                    None,
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.error",
                    [error.as_str()],
                );
                let text_log = rerun::TextLog::new(error).with_level(rerun::TextLogLevel::ERROR);
                self.write(
                    "/diagnostics/runtime",
                    time,
                    &Combined::new([&payload, &text_log]),
                );
                return Ok(());
            }
            _ => return Err(format!("unsupported event `{name}`")),
        };
        let scalar = match name {
            "async_ingress" | "action_schedule" => f.u64("value_size").map(|value| value as f64),
            "tag_complete" => f.boolean("terminal").map(|value| u8::from(value) as f64),
            _ => None,
        };
        if let Some(scalar) = scalar {
            let measure = rerun::Scalars::new([scalar]);
            self.write_measure(
                &path.path,
                path.entity_label.as_deref(),
                time,
                &Combined::new([&payload, &measure]),
            );
        } else {
            self.write(&path.path, time, &payload);
        }
        Ok(())
    }
}

struct TimeReset<'a>(&'a rerun::RecordingStream);
impl Drop for TimeReset<'_> {
    fn drop(&mut self) {
        self.0.reset_time();
    }
}

struct Combined(Vec<rerun::SerializedComponentBatch>);

impl Combined {
    fn new<const N: usize>(values: [&dyn rerun::AsComponents; N]) -> Self {
        Self(
            values
                .into_iter()
                .flat_map(rerun::AsComponents::as_serialized_batches)
                .collect(),
        )
    }
}

impl rerun::AsComponents for Combined {
    fn as_serialized_batches(&self) -> Vec<rerun::SerializedComponentBatch> {
        self.0.clone()
    }
}

fn common(
    archetype: &'static str,
    event: &str,
    federate: Option<&str>,
    enclave: Option<&str>,
    logical: Option<u64>,
    microstep: Option<u64>,
) -> rerun::DynamicArchetype {
    let mut value = rerun::DynamicArchetype::new(archetype)
        .with_component::<rerun::components::Text>("boomerang.trace.event", [event]);
    if let Some(federate) = federate {
        value =
            value.with_component::<rerun::components::Text>("boomerang.trace.federate", [federate]);
    }
    if let Some(enclave) = enclave {
        value =
            value.with_component::<rerun::components::Text>("boomerang.trace.enclave", [enclave]);
    }
    if let Some(logical) = logical {
        value = value.with_component_from_data("boomerang.trace.logical_ns", u64_array(logical));
    }
    if let Some(microstep) = microstep {
        value = value.with_component_from_data("boomerang.trace.microstep", u64_array(microstep));
    }
    value
}

fn u64_array(value: u64) -> Arc<dyn rerun::external::arrow::array::Array> {
    Arc::new(rerun::external::arrow::array::UInt64Array::from(vec![
        value,
    ]))
}
fn bool_array(value: bool) -> Arc<dyn rerun::external::arrow::array::Array> {
    Arc::new(rerun::external::arrow::array::BooleanArray::from(vec![
        value,
    ]))
}
fn saturating_i64(value: u128) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn compact_series_name(path: &str, entity_label: Option<&str>) -> String {
    let (entity, event) = path.rsplit_once('/').unwrap_or((path, path));
    let entity = entity_label.map(str::to_owned).unwrap_or_else(|| {
        let leaf = entity.rsplit('/').next().unwrap_or(entity);
        compact_runtime_key(leaf)
    });
    bounded_fragment(
        &format!("{entity} · {}", bounded_fragment(compact_event(event), 16)),
        64,
    )
}

fn compact_event(event: &str) -> &str {
    match event {
        "reaction_execute" => "exec",
        "propagation_send" => "send",
        "propagation_receive" => "recv",
        "action_schedule" => "schedule",
        "async_ingress" => "ingress",
        "tag_process" => "tag",
        "tag_complete" => "tag done",
        "coordination_wait" => "coord wait",
        other => other,
    }
}
fn with_span_state<S, T>(
    id: &Id,
    ctx: &Context<'_, S>,
    callback: impl FnOnce(&SpanState) -> T,
) -> Option<T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let span = ctx.span(id)?;
    let extensions = span.extensions();
    extensions.get::<SpanState>().map(callback)
}
