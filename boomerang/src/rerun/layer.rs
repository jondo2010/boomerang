use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::entities::{
    bounded_fragment, compact_runtime_key, escape_entity_segment, RegistrationIndex,
};
use super::session::SessionState;

const TRACE_TARGET: &str = "boomerang::trace";
const MAX_CORRELATION_ROUTES: usize = 4096;
const MAX_CORRELATIONS_PER_ROUTE: usize = 16;

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
pub struct RerunLayer {
    recording: rerun::RecordingStream,
    state: SessionState,
    source_id: Arc<str>,
    adapter: AdapterState,
}

#[derive(Clone)]
pub(super) struct AdapterState {
    next_id: Arc<AtomicU64>,
    pub(super) registration: Arc<RwLock<RegistrationIndex>>,
    correlation: Arc<Mutex<Correlation>>,
    named_series: Arc<RwLock<HashSet<String>>>,
}

impl Default for AdapterState {
    fn default() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            registration: Arc::new(RwLock::new(RegistrationIndex::default())),
            correlation: Arc::new(Mutex::new(Correlation::default())),
            named_series: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TraceId(String);

#[derive(Clone, Copy)]
struct TimePoint {
    logical_ns: Option<i64>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RouteKey {
    federate: Option<String>,
    enclave: Option<String>,
    action_key: String,
    logical_ns: u64,
    microstep: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReactionRoute {
    federate: Option<String>,
    enclave: String,
    logical_ns: u64,
    microstep: u64,
}

#[derive(Clone)]
struct PendingSend {
    id: TraceId,
    accepted: bool,
}

#[derive(Clone)]
struct PendingIngress {
    id: TraceId,
    path: String,
    time: TimePoint,
    federate: Option<String>,
    enclave: String,
    action_key: String,
    action: String,
    logical_ns: u64,
    microstep: u64,
    destination_logical_ns: u64,
    destination_microstep: u64,
    value_type: String,
    value_size: u64,
}

#[derive(Default)]
struct Correlation {
    sends: HashMap<RouteKey, Vec<PendingSend>>,
    ingresses: HashMap<RouteKey, Vec<PendingIngress>>,
    receives: HashMap<RouteKey, Vec<(TraceId, String)>>,
    poisoned: HashSet<RouteKey>,
    reactions: HashMap<ReactionRoute, Vec<PendingReaction>>,
    poisoned_reactions: HashSet<ReactionRoute>,
    route_order: VecDeque<RouteKey>,
    reaction_order: VecDeque<ReactionRoute>,
}

impl Correlation {
    fn track(&mut self, key: &RouteKey) {
        if !self.route_order.contains(key) {
            self.route_order.push_back(key.clone());
        }
        while self.route_order.len() > MAX_CORRELATION_ROUTES {
            if let Some(expired) = self.route_order.pop_front() {
                self.sends.remove(&expired);
                self.ingresses.remove(&expired);
                self.receives.remove(&expired);
                self.poisoned.remove(&expired);
            }
        }
    }

    fn push_reaction(&mut self, reaction: PendingReaction) {
        let route = ReactionRoute::from(&reaction);
        if !self.reaction_order.contains(&route) {
            self.reaction_order.push_back(route.clone());
        }
        while self.reaction_order.len() > MAX_CORRELATION_ROUTES {
            if let Some(expired) = self.reaction_order.pop_front() {
                self.reactions.remove(&expired);
                self.poisoned_reactions.remove(&expired);
            }
        }
        let values = self.reactions.entry(route.clone()).or_default();
        if values.len() >= MAX_CORRELATIONS_PER_ROUTE {
            self.poisoned_reactions.insert(route);
        } else {
            values.push(reaction);
        }
    }

    fn push_send(&mut self, key: RouteKey, value: PendingSend) {
        self.track(&key);
        push_route(&mut self.sends, &mut self.poisoned, key, value);
    }

    fn push_ingress(&mut self, key: RouteKey, value: PendingIngress) {
        self.track(&key);
        push_route(&mut self.ingresses, &mut self.poisoned, key, value);
    }

    fn push_receive(&mut self, key: RouteKey, value: (TraceId, String)) {
        self.track(&key);
        push_route(&mut self.receives, &mut self.poisoned, key, value);
    }
}

fn push_route<T>(
    routes: &mut HashMap<RouteKey, Vec<T>>,
    poisoned: &mut HashSet<RouteKey>,
    key: RouteKey,
    value: T,
) {
    let values = routes.entry(key.clone()).or_default();
    if values.len() >= MAX_CORRELATIONS_PER_ROUTE {
        poisoned.insert(key);
    } else {
        values.push(value);
    }
}

#[derive(Clone)]
struct PendingReaction {
    id: TraceId,
    federate: Option<String>,
    enclave: String,
    logical_ns: u64,
    microstep: u64,
    path: String,
    time: TimePoint,
}

impl From<&PendingReaction> for ReactionRoute {
    fn from(reaction: &PendingReaction) -> Self {
        Self {
            federate: reaction.federate.clone(),
            enclave: reaction.enclave.clone(),
            logical_ns: reaction.logical_ns,
            microstep: reaction.microstep,
        }
    }
}

#[derive(Clone)]
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
        outcome: Option<String>,
    },
}

struct SpanState {
    id: TraceId,
    parent_id: Option<TraceId>,
    federate: Option<String>,
    enclave: Option<String>,
    logical_ns: Option<u64>,
    microstep: Option<u64>,
    path: String,
    time: TimePoint,
    timing: Mutex<SpanTiming>,
    kind: Mutex<SpanKind>,
}

#[derive(Default)]
struct SpanTiming {
    entered: HashMap<ThreadId, Vec<Instant>>,
    total: Duration,
}

impl SpanTiming {
    fn enter(&mut self) {
        self.entered
            .entry(std::thread::current().id())
            .or_default()
            .push(Instant::now());
    }

    fn exit(&mut self) {
        let thread = std::thread::current().id();
        let (started, empty) = self
            .entered
            .get_mut(&thread)
            .map(|stack| (stack.pop(), stack.is_empty()))
            .unwrap_or((None, false));
        if empty {
            self.entered.remove(&thread);
            if let Some(started) = started {
                self.total += started.elapsed();
            }
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
        source_id: Arc<str>,
        adapter: AdapterState,
    ) -> Self {
        Self {
            recording,
            state,
            source_id,
            adapter,
        }
    }

    fn isolate(&self, callback: impl FnOnce()) {
        if catch_unwind(AssertUnwindSafe(callback)).is_err() {
            self.state
                .disable_on_error(&"Rerun trace callback panicked");
        }
    }

    fn next_id(&self, enclave: Option<&str>) -> Option<TraceId> {
        let sequence = self
            .adapter
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .ok()?;
        Some(TraceId(format!(
            "{}:{}:{sequence}",
            self.source_id,
            enclave.unwrap_or("process")
        )))
    }

    fn timepoint(&self, logical_ns: Option<u64>) -> TimePoint {
        TimePoint {
            logical_ns: logical_ns.and_then(|value| i64::try_from(value).ok()),
        }
    }

    fn write(&self, path: &str, time: TimePoint, value: &dyn rerun::AsComponents) {
        if !self.state.try_begin_attempt() {
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

    fn write_measure(&self, path: &str, time: TimePoint, value: &dyn rerun::AsComponents) {
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
            let name = self.compact_series_name(path);
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

    fn compact_series_name(&self, path: &str) -> String {
        let (entity, event) = path.rsplit_once('/').unwrap_or((path, path));
        let registered = self
            .adapter
            .registration
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .display_label(entity)
            .map(str::to_owned);
        let entity = registered.unwrap_or_else(|| {
            let leaf = entity.rsplit('/').next().unwrap_or(entity);
            compact_runtime_key(leaf)
        });
        bounded_fragment(
            &format!("{entity} · {}", bounded_fragment(compact_event(event), 16)),
            64,
        )
    }

    fn diagnostic(&self, event: Option<&str>, error: impl fmt::Display) {
        let message = match event {
            Some(event) => format!("invalid `{event}` trace annotation: {error}"),
            None => format!("invalid trace annotation: {error}"),
        };
        let id = self
            .next_id(Some("diagnostics"))
            .unwrap_or_else(|| TraceId("diagnostic".into()));
        let archetype = common(
            "boomerang.SchemaDiagnostic",
            &id,
            None,
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

    fn context<S>(
        &self,
        ctx: &Context<'_, S>,
        explicit_parent: Option<&Id>,
        contextual: bool,
    ) -> Option<Arc<SpanState>>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        match explicit_parent {
            Some(id) => span_state(id, ctx),
            None if contextual => ctx
                .lookup_current()
                .and_then(|span| span.extensions().get::<Arc<SpanState>>().cloned()),
            None => None,
        }
    }

    fn path(
        &self,
        federate: Option<&str>,
        enclave: Option<&str>,
        kind: &'static str,
        identity: &str,
        event: &str,
    ) -> String {
        let registered = enclave.and_then(|enclave| {
            self.adapter
                .registration
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .resolve(federate, enclave, kind, identity)
        });
        registered
            .map(|path| format!("{path}/{}", escape_entity_segment(event)))
            .unwrap_or_else(|| {
                let root = match (federate, enclave) {
                    (Some(federate), Some(enclave)) => format!(
                        "/federates/{}/enclaves/{}",
                        escape_entity_segment(federate),
                        escape_entity_segment(enclave)
                    ),
                    (None, Some(enclave)) => {
                        format!("/enclaves/{}", escape_entity_segment(enclave))
                    }
                    _ => "/propagation/unresolved".to_owned(),
                };
                format!("{root}/{}", escape_entity_segment(event))
            })
    }

    fn emit_edge(&self, source: &TraceId, destination: &TraceId, time: TimePoint, path: &str) {
        let id = self
            .next_id(Some("causal"))
            .unwrap_or_else(|| TraceId("causal".into()));
        let archetype = common(
            "boomerang.CausalLink",
            &id,
            None,
            "causal_link",
            None,
            None,
            None,
            None,
        )
        .with_component::<rerun::components::Text>("boomerang.trace.source", [source.0.as_str()])
        .with_component::<rerun::components::Text>(
            "boomerang.trace.destination",
            [destination.0.as_str()],
        )
        .with_component::<rerun::components::Text>("boomerang.trace.state", ["exact"])
        .with_component::<rerun::components::Text>("boomerang.trace.outcome", ["matched"]);
        let graph = rerun::GraphEdges::new([(source.0.as_str(), destination.0.as_str())])
            .with_graph_type(rerun::components::GraphType::Directed);
        self.write(path, time, &Combined::new([&archetype, &graph]));
    }

    fn correlate(&self, key: RouteKey) {
        let pair = {
            let mut state = self
                .adapter
                .correlation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.poisoned.contains(&key) {
                return;
            }
            let sends = state
                .sends
                .get(&key)
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            let ingresses = state.ingresses.get(&key).cloned().unwrap_or_default();
            if sends.len() != 1 || !sends[0].accepted || ingresses.len() != 1 {
                return;
            }
            state.sends.remove(&key);
            state.ingresses.remove(&key);
            for pending in state.ingresses.values_mut() {
                pending.retain(|candidate| candidate.id != ingresses[0].id);
            }
            state.ingresses.retain(|_, pending| !pending.is_empty());
            Some((sends[0].clone(), ingresses[0].clone()))
        };
        if let Some((send, ingress)) = pair {
            let receive = common(
                "boomerang.PropagationReceive",
                &ingress.id,
                None,
                "propagation_receive",
                ingress.federate.as_deref(),
                Some(&ingress.enclave),
                Some(ingress.logical_ns),
                Some(ingress.microstep),
            )
            .with_component::<rerun::components::Text>(
                "boomerang.trace.action_key",
                [ingress.action_key.as_str()],
            )
            .with_component::<rerun::components::Text>(
                "boomerang.trace.action",
                [ingress.action.as_str()],
            )
            .with_component_from_data(
                "boomerang.trace.destination_logical_ns",
                u64_array(ingress.destination_logical_ns),
            )
            .with_component_from_data(
                "boomerang.trace.destination_microstep",
                u64_array(ingress.destination_microstep),
            )
            .with_component::<rerun::components::Text>(
                "boomerang.trace.value_type",
                [ingress.value_type.as_str()],
            )
            .with_component_from_data("boomerang.trace.value_size", u64_array(ingress.value_size))
            .with_component::<rerun::components::Text>("boomerang.trace.outcome", ["accepted"]);
            let value_size = rerun::Scalars::new([ingress.value_size as f64]);
            self.write_measure(
                &ingress.path,
                ingress.time,
                &Combined::new([&receive, &value_size]),
            );
            self.emit_edge(&send.id, &ingress.id, ingress.time, &ingress.path);
            let receive_key = RouteKey {
                federate: ingress.federate.clone(),
                enclave: Some(ingress.enclave.clone()),
                action_key: ingress.action_key.clone(),
                logical_ns: ingress.logical_ns,
                microstep: ingress.microstep,
            };
            {
                let mut correlation = self
                    .adapter
                    .correlation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                correlation.push_receive(
                    receive_key,
                    (ingress.id.clone(), ingress.action_key.clone()),
                );
            }
            let reaction_route = ReactionRoute {
                federate: ingress.federate.clone(),
                enclave: ingress.enclave.clone(),
                logical_ns: ingress.logical_ns,
                microstep: ingress.microstep,
            };
            let reactions = self
                .adapter
                .correlation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let reactions = if reactions.poisoned_reactions.contains(&reaction_route) {
                Vec::new()
            } else {
                reactions
                    .reactions
                    .get(&reaction_route)
                    .into_iter()
                    .flatten()
                    .filter(|reaction| {
                        self.adapter
                            .registration
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .action_triggers_reaction(
                                ingress.federate.as_deref(),
                                &ingress.enclave,
                                &ingress.action_key,
                                &reaction.path,
                            )
                            == Some(true)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if reactions.len() == 1 {
                self.emit_edge(
                    &ingress.id,
                    &reactions[0].id,
                    reactions[0].time,
                    &reactions[0].path,
                );
            }
        }
    }

    fn reaction_predecessor(&self, reaction: &SpanState) {
        let Some(enclave) = reaction.enclave.as_deref() else {
            return;
        };
        let Some(logical_ns) = reaction.logical_ns else {
            return;
        };
        let key_prefix = (
            reaction.federate.as_deref(),
            enclave,
            logical_ns,
            reaction.microstep.unwrap_or(0),
        );
        let reaction_route = ReactionRoute {
            federate: reaction.federate.clone(),
            enclave: enclave.to_owned(),
            logical_ns,
            microstep: reaction.microstep.unwrap_or(0),
        };
        let candidates = {
            let state = self
                .adapter
                .correlation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.poisoned_reactions.contains(&reaction_route) {
                return;
            }
            state
                .receives
                .iter()
                .filter(|(key, _)| {
                    !state.poisoned.contains(*key)
                        && key.federate.as_deref() == key_prefix.0
                        && key.enclave.as_deref() == Some(key_prefix.1)
                        && key.logical_ns == key_prefix.2
                        && key.microstep == key_prefix.3
                })
                .flat_map(|(_, values)| values)
                .filter(|(_, action_key)| {
                    self.adapter
                        .registration
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .action_triggers_reaction(
                            reaction.federate.as_deref(),
                            reaction.enclave.as_deref().unwrap_or_default(),
                            action_key,
                            &reaction.path,
                        )
                        == Some(true)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        if candidates.len() == 1 {
            self.emit_edge(
                &candidates[0].0,
                &reaction.id,
                reaction.time,
                &reaction.path,
            );
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
            let parent = self.context(&ctx, attrs.parent(), attrs.is_contextual());
            let federate = fields
                .text("federate")
                .map(str::to_owned)
                .or_else(|| parent.as_ref().and_then(|p| p.federate.clone()));
            let enclave = fields
                .text("enclave")
                .map(str::to_owned)
                .or_else(|| parent.as_ref().and_then(|p| p.enclave.clone()));
            let logical_ns = fields
                .u64("logical_ns")
                .or_else(|| parent.as_ref().and_then(|p| p.logical_ns));
            let microstep = fields
                .u64("microstep")
                .or_else(|| parent.as_ref().and_then(|p| p.microstep));
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
                        if kind == "logical" {
                            logical_ns.ok_or("missing or invalid `logical_ns`")?;
                            microstep.ok_or("missing or invalid `microstep`")?;
                        }
                        let outcome = fields
                            .text("outcome")
                            .map(|_| {
                                required_discriminator(&fields, "outcome", &["accepted", "failed"])
                            })
                            .transpose()?;
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
                            "action",
                            action_key,
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
            match &kind {
                SpanKind::Scheduler { .. } => {
                    if enclave.is_none() {
                        self.diagnostic(Some(&event), "missing or invalid `enclave`");
                        return;
                    }
                }
                SpanKind::Tag { .. } | SpanKind::Reaction { .. } | SpanKind::Wait { .. } => {
                    if enclave.is_none() || logical_ns.is_none() || microstep.is_none() {
                        self.diagnostic(Some(&event), "missing enclave or logical tag context");
                        return;
                    }
                }
                SpanKind::Send { .. } => {}
            }
            let Some(trace_id) = self.next_id(enclave.as_deref()) else {
                return;
            };
            let path = if matches!(
                &kind,
                SpanKind::Send {
                    destination_federate: Some(_),
                    ..
                }
            ) {
                if let SpanKind::Send {
                    destination_federate: Some(destination),
                    ..
                } = &kind
                {
                    self.adapter
                        .registration
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .resolve_federated(destination, entity_kind, &identity)
                        .map(|path| format!("{path}/{}", escape_entity_segment(&event)))
                        .unwrap_or_else(|| {
                            format!("/propagation/unresolved/{}", escape_entity_segment(&event))
                        })
                } else {
                    unreachable!()
                }
            } else if let SpanKind::Send {
                destination: Some(destination),
                ..
            } = &kind
            {
                self.path(
                    federate.as_deref(),
                    Some(destination),
                    entity_kind,
                    &identity,
                    &event,
                )
            } else {
                self.path(
                    federate.as_deref(),
                    enclave.as_deref(),
                    entity_kind,
                    &identity,
                    &event,
                )
            };
            let span = Arc::new(SpanState {
                id: trace_id,
                parent_id: parent.map(|p| p.id.clone()),
                federate,
                enclave,
                logical_ns,
                microstep,
                path,
                time: self.timepoint(logical_ns),
                timing: Mutex::new(SpanTiming::default()),
                kind: Mutex::new(kind),
            });
            if let Some(scope) = ctx.span(id) {
                scope.extensions_mut().insert(span.clone());
            }
            let phase = match &*span
                .kind
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
            {
                SpanKind::Tag { .. } => Some("processing tag"),
                SpanKind::Reaction { .. } => Some("executing reaction"),
                SpanKind::Wait { .. } => Some("waiting for coordination"),
                _ => None,
            };
            if let Some(phase) = phase {
                self.write(
                    &span.path,
                    self.timepoint(None),
                    &rerun::StateChange::single(phase),
                );
            }
            if matches!(
                &*span
                    .kind
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                SpanKind::Reaction { .. }
            ) {
                if let (Some(enclave), Some(logical_ns)) = (span.enclave.clone(), span.logical_ns) {
                    self.adapter
                        .correlation
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push_reaction(PendingReaction {
                            id: span.id.clone(),
                            federate: span.federate.clone(),
                            enclave,
                            logical_ns,
                            microstep: span.microstep.unwrap_or(0),
                            path: span.path.clone(),
                            time: span.time,
                        });
                }
                self.reaction_predecessor(&span);
            }
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        self.isolate(|| {
            let Some(span) = span_state(id, &ctx) else {
                return;
            };
            let mut fields = CallbackFields::default();
            values.record(&mut fields);
            if let Some(outcome) = fields.text("outcome") {
                if let SpanKind::Send {
                    outcome: current, ..
                } = &mut *span
                    .kind
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                {
                    *current = Some(outcome.to_owned());
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
            let parent = self.context(&ctx, event.parent(), event.is_contextual());
            if let Err(error) = self.emit_event(&name, &fields, parent.as_deref()) {
                self.diagnostic(Some(&name), error);
            }
        });
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            if let Some(span) = span_state(id, &ctx) {
                span.timing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .enter();
            }
        });
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            if let Some(span) = span_state(id, &ctx) {
                span.timing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .exit();
            }
        });
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        self.isolate(|| {
            let Some(span) = span_state(&id, &ctx) else {
                return;
            };
            let duration = saturating_i64(
                span.timing
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .total
                    .as_nanos(),
            ) as u64;
            let kind = span
                .kind
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let SpanKind::Send { outcome, .. } = &kind {
                match outcome.as_deref() {
                    Some("accepted" | "failed") => {}
                    Some(outcome) => {
                        self.diagnostic(
                            Some("propagation_send"),
                            format!("invalid `outcome` discriminator `{outcome}`"),
                        );
                        return;
                    }
                    None => {
                        self.diagnostic(Some("propagation_send"), "missing or invalid `outcome`");
                        return;
                    }
                }
            }
            let mut payload = match &kind {
                SpanKind::Scheduler { state } => common(
                    "boomerang.SchedulerRunning",
                    &span.id,
                    span.parent_id.as_ref(),
                    "scheduler_thread",
                    span.federate.as_deref(),
                    span.enclave.as_deref(),
                    None,
                    None,
                )
                .with_component::<rerun::components::Text>(
                    "boomerang.trace.state",
                    [state.as_str()],
                ),
                SpanKind::Tag { terminal, state } => common(
                    "boomerang.TagProcessing",
                    &span.id,
                    span.parent_id.as_ref(),
                    "tag_process",
                    span.federate.as_deref(),
                    span.enclave.as_deref(),
                    span.logical_ns,
                    span.microstep,
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
                        &span.id,
                        span.parent_id.as_ref(),
                        "reaction_execute",
                        span.federate.as_deref(),
                        span.enclave.as_deref(),
                        span.logical_ns,
                        span.microstep,
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
                    &span.id,
                    span.parent_id.as_ref(),
                    "coordination_wait",
                    span.federate.as_deref(),
                    span.enclave.as_deref(),
                    span.logical_ns,
                    span.microstep,
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
                        &span.id,
                        span.parent_id.as_ref(),
                        "propagation_send",
                        span.federate.as_deref(),
                        span.enclave.as_deref(),
                        span.logical_ns,
                        span.microstep,
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
                    .with_component_from_data("boomerang.trace.value_size", u64_array(*value_size));
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
                    if let Some(outcome) = outcome {
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
            let measure = match &kind {
                SpanKind::Send { value_size, .. } => *value_size as f64,
                SpanKind::Tag { terminal, .. } => u8::from(*terminal) as f64,
                SpanKind::Scheduler { .. } | SpanKind::Reaction { .. } | SpanKind::Wait { .. } => {
                    duration as f64
                }
            };
            {
                let scalar = rerun::Scalars::new([measure]);
                self.write_measure(&span.path, span.time, &Combined::new([&payload, &scalar]));
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
            if let SpanKind::Send {
                destination,
                destination_federate,
                action_key,
                outcome,
                ..
            } = kind
            {
                if let (Some(logical_ns), Some(microstep)) = (span.logical_ns, span.microstep) {
                    let key = RouteKey {
                        federate: destination_federate.or_else(|| span.federate.clone()),
                        enclave: destination,
                        action_key,
                        logical_ns,
                        microstep,
                    };
                    let mut correlation = self
                        .adapter
                        .correlation
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    correlation.push_send(
                        key.clone(),
                        PendingSend {
                            id: span.id.clone(),
                            accepted: outcome.as_deref() == Some("accepted"),
                        },
                    );
                    drop(correlation);
                    self.correlate(key);
                }
            }
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
            .or_else(|| parent.and_then(|p| p.federate.as_deref()));
        let enclave = f
            .text("enclave")
            .or_else(|| parent.and_then(|p| p.enclave.as_deref()));
        let logical = f
            .u64("logical_ns")
            .or_else(|| parent.and_then(|p| p.logical_ns));
        let microstep = f
            .u64("microstep")
            .or_else(|| parent.and_then(|p| p.microstep));
        let id = self.next_id(enclave).ok_or("trace ID sequence exhausted")?;
        let parent_id = parent.map(|p| &p.id);
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
                    let path = self.path(federate, Some(enclave), "action", &action_key, name);
                    let archetype = if kind == "logical" {
                        "boomerang.LogicalIngress"
                    } else {
                        "boomerang.PhysicalIngress"
                    };
                    let payload = common(
                        archetype,
                        &id,
                        parent_id,
                        name,
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
                    if kind == "logical" && outcome == "accepted" {
                        let key = RouteKey {
                            federate: federate.map(str::to_owned),
                            enclave: Some(enclave.to_owned()),
                            action_key: action_key.clone(),
                            logical_ns,
                            microstep,
                        };
                        let pending = PendingIngress {
                            id: id.clone(),
                            path: path.replace("/async_ingress", "/propagation_receive"),
                            time,
                            federate: federate.map(str::to_owned),
                            enclave: enclave.to_owned(),
                            action_key,
                            action,
                            logical_ns,
                            microstep,
                            destination_logical_ns,
                            destination_microstep,
                            value_type,
                            value_size,
                        };
                        {
                            let mut correlation = self
                                .adapter
                                .correlation
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            correlation.push_ingress(key.clone(), pending.clone());
                        }
                        let federated_key = RouteKey {
                            enclave: None,
                            ..key.clone()
                        };
                        {
                            let mut correlation = self
                                .adapter
                                .correlation
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            correlation.push_ingress(federated_key.clone(), pending);
                        }
                        self.correlate(key);
                        self.correlate(federated_key);
                    }
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
                            &id,
                            parent_id,
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
                let mut payload = common(
                    archetype,
                    &id,
                    parent_id,
                    name,
                    federate,
                    Some(enclave),
                    logical,
                    microstep,
                )
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
                    parent.map(|parent| {
                        parent
                            .kind
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                    }),
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
                    &id,
                    parent_id,
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
                }) = parent.map(|p| {
                    p.kind
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone()
                }) {
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
                let payload = common(
                    archetype,
                    &id,
                    parent_id,
                    name,
                    federate,
                    Some(enclave),
                    logical,
                    microstep,
                )
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
                let mut payload = common(
                    archetype,
                    &id,
                    parent_id,
                    name,
                    federate,
                    Some(enclave),
                    logical,
                    microstep,
                );
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
                    &id,
                    parent_id,
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
            self.write_measure(&path, time, &Combined::new([&payload, &measure]));
        } else {
            self.write(&path, time, &payload);
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

#[allow(clippy::too_many_arguments)]
fn common(
    archetype: &'static str,
    id: &TraceId,
    parent: Option<&TraceId>,
    event: &str,
    federate: Option<&str>,
    enclave: Option<&str>,
    logical: Option<u64>,
    microstep: Option<u64>,
) -> rerun::DynamicArchetype {
    let mut value = rerun::DynamicArchetype::new(archetype)
        .with_component::<rerun::components::Text>("boomerang.trace.id", [id.0.as_str()])
        .with_component::<rerun::components::Text>("boomerang.trace.event", [event]);
    if let Some(parent) = parent {
        value = value.with_component::<rerun::components::Text>(
            "boomerang.trace.parent_id",
            [parent.0.as_str()],
        );
    }
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
fn span_state<S>(id: &Id, ctx: &Context<'_, S>) -> Option<Arc<SpanState>>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    ctx.span(id)?.extensions().get::<Arc<SpanState>>().cloned()
}
