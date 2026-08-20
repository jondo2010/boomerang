use std::collections::HashMap;
use std::error::Error;
use std::fmt;

/// Adapter-owned identifier for one dynamic trace record.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraceId(pub String);

impl TraceId {
    pub(crate) fn new(source: &str, enclave: &str, sequence: u64) -> Self {
        Self(format!("{source}:{enclave}:{sequence}"))
    }
}

impl std::ops::Deref for TraceId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Explicit timestamps attached independently to every trace record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceTimePoint {
    pub elapsed_ns: i64,
    pub wall_clock_unix_ns: i64,
    pub logical_ns: Option<i64>,
}

/// Typed values accepted from Boomerang's stable tracing schema.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceFields {
    pub event: Option<String>,
    pub enclave: Option<String>,
    pub kind: Option<String>,
    pub reactor: Option<String>,
    pub reaction_key: Option<String>,
    pub reaction: Option<String>,
    pub action_key: Option<String>,
    pub action: Option<String>,
    pub port_key: Option<String>,
    pub port: Option<String>,
    pub logical_ns: Option<u64>,
    pub microstep: Option<u64>,
    pub destination: Option<String>,
    /// Adapter-owned source identifier for synthesized causal relations.
    pub source: Option<String>,
    pub destination_logical_ns: Option<u64>,
    pub destination_microstep: Option<u64>,
    pub old_logical_ns: Option<u64>,
    pub old_microstep: Option<u64>,
    pub level: Option<String>,
    pub state: Option<String>,
    pub terminal: Option<bool>,
    pub value_type: Option<String>,
    pub value_size: Option<u64>,
    pub outcome: Option<String>,
    pub error: Option<String>,
}

impl TraceFields {
    pub(crate) fn inherit_missing(&mut self, parent: &Self) {
        macro_rules! inherit {
            ($($field:ident),* $(,)?) => {
                $(if self.$field.is_none() { self.$field = parent.$field.clone(); })*
            };
        }
        inherit!(
            enclave,
            reactor,
            reaction_key,
            reaction,
            logical_ns,
            microstep,
        );
    }
}

/// One adapter-normalized dynamic record ready for a recording sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub id: TraceId,
    pub parent_id: Option<TraceId>,
    pub entity_path: String,
    pub event: String,
    pub timepoint: TraceTimePoint,
    /// Microstep is deliberately a component, not a Rerun timeline.
    pub microstep: Option<u64>,
    pub duration_ns: Option<u64>,
    /// Final lifecycle state emitted when a tracked runtime span closes.
    pub terminal_state: Option<String>,
    pub fields: TraceFields,
}

/// Error returned by a dynamic trace writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceWriterError(pub String);

impl fmt::Display for TraceWriterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TraceWriterError {}

impl From<&str> for TraceWriterError {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for TraceWriterError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<rerun::RecordingStreamError> for TraceWriterError {
    fn from(value: rerun::RecordingStreamError) -> Self {
        Self(value.to_string())
    }
}

/// Synchronous writer seam used by the live layer and future recording sinks.
///
/// Implementations must not retain application payloads. Calls may arrive concurrently.
pub trait TraceWriter: Send + Sync + 'static {
    fn write(
        &self,
        recording: &rerun::RecordingStream,
        record: &TraceRecord,
    ) -> Result<(), TraceWriterError>;
}

pub(crate) struct RerunTraceWriter;

enum PrimaryPayload {
    TextLog(Box<rerun::TextLog>),
    Dynamic(rerun::DynamicArchetype),
}

struct TimeContextReset<'a>(&'a rerun::RecordingStream);

impl Drop for TimeContextReset<'_> {
    fn drop(&mut self) {
        self.0.reset_time();
    }
}

impl TraceWriter for RerunTraceWriter {
    fn write(
        &self,
        recording: &rerun::RecordingStream,
        record: &TraceRecord,
    ) -> Result<(), TraceWriterError> {
        let mut timepoint = rerun::TimePoint::default();
        timepoint.insert_cell(
            "elapsed",
            rerun::TimeCell::from_duration_nanos(record.timepoint.elapsed_ns),
        );
        timepoint.insert_cell(
            "wall_clock",
            rerun::TimeCell::from_timestamp_nanos_since_epoch(record.timepoint.wall_clock_unix_ns),
        );
        if let Some(logical_ns) = record.timepoint.logical_ns {
            timepoint.insert_cell("logical", rerun::TimeCell::from_duration_nanos(logical_ns));
        }

        // Rerun's time context is thread-local. Reset before and after each write so a record
        // without logical time cannot inherit it from an earlier record on the same thread. The
        // guard also resets the context during panic unwinding.
        recording.reset_time();
        let _reset = TimeContextReset(recording);
        recording.set_timepoint(timepoint);
        match record.primary_payload() {
            PrimaryPayload::TextLog(payload) => {
                recording.log(record.entity_path.clone(), payload.as_ref())?;
            }
            PrimaryPayload::Dynamic(payload) => {
                recording.log(record.entity_path.clone(), &payload)?;
            }
        }
        if record.event == "causal_link" {
            if let (Some(source), Some(destination)) = (
                record.fields.source.as_deref(),
                record.fields.destination.as_deref(),
            ) {
                recording.log(
                    record.entity_path.clone(),
                    &rerun::GraphEdges::new([(source, destination)])
                        .with_graph_type(rerun::components::GraphType::Directed),
                )?;
            }
        }
        for (name, value) in record.scalar_series() {
            recording.log(
                format!("{}/metrics/{name}", record.entity_path),
                &rerun::Scalars::new([value]),
            )?;
        }
        Ok(())
    }
}

impl TraceRecord {
    fn primary_payload(&self) -> PrimaryPayload {
        if self.event == "diagnostic" {
            let text = self
                .fields
                .error
                .as_deref()
                .unwrap_or("Boomerang trace diagnostic");
            return PrimaryPayload::TextLog(Box::new(
                rerun::TextLog::new(text).with_level(rerun::TextLogLevel::ERROR),
            ));
        }

        let mut payload = rerun::DynamicArchetype::new("boomerang.TraceRecord");
        for (name, value) in self.string_components() {
            payload = payload.with_component::<rerun::components::Text>(name, [value]);
        }
        for (name, value) in self.u64_components() {
            payload = payload.with_component_from_data(
                name,
                std::sync::Arc::new(rerun::external::arrow::array::UInt64Array::from(vec![
                    value,
                ])),
            );
        }
        if let Some(terminal) = self.fields.terminal {
            payload = payload.with_component_from_data(
                "boomerang.trace.terminal",
                std::sync::Arc::new(rerun::external::arrow::array::BooleanArray::from(vec![
                    terminal,
                ])),
            );
        }
        PrimaryPayload::Dynamic(payload)
    }

    fn string_components(&self) -> Vec<(&'static str, String)> {
        let mut values = vec![
            ("boomerang.trace.id", self.id.0.clone()),
            ("boomerang.trace.event", self.event.clone()),
        ];
        if let Some(parent_id) = &self.parent_id {
            values.push(("boomerang.trace.parent_id", parent_id.0.clone()));
        }
        if let Some(terminal_state) = &self.terminal_state {
            values.push(("boomerang.trace.terminal_state", terminal_state.clone()));
        }

        macro_rules! string_fields {
            ($($field:ident),* $(,)?) => {
                $(if let Some(value) = &self.fields.$field {
                    values.push((concat!("boomerang.trace.", stringify!($field)), value.clone()));
                })*
            };
        }
        string_fields!(
            enclave,
            kind,
            reactor,
            reaction_key,
            reaction,
            action_key,
            action,
            port_key,
            port,
            destination,
            source,
            level,
            state,
            value_type,
            outcome,
            error,
        );
        values
    }

    fn u64_components(&self) -> Vec<(&'static str, u64)> {
        let mut values = Vec::new();
        macro_rules! component {
            ($name:literal, $value:expr) => {
                if let Some(value) = $value {
                    values.push(($name, value));
                }
            };
        }
        component!("boomerang.trace.microstep", self.microstep);
        component!("boomerang.trace.duration_ns", self.duration_ns);
        component!("boomerang.trace.logical_ns", self.fields.logical_ns);
        component!(
            "boomerang.trace.destination_logical_ns",
            self.fields.destination_logical_ns
        );
        component!(
            "boomerang.trace.destination_microstep",
            self.fields.destination_microstep
        );
        component!("boomerang.trace.old_logical_ns", self.fields.old_logical_ns);
        component!("boomerang.trace.old_microstep", self.fields.old_microstep);
        component!("boomerang.trace.value_size", self.fields.value_size);
        values
    }

    fn scalar_series(&self) -> Vec<(&'static str, f64)> {
        let mut values = Vec::new();
        if let Some(duration_ns) = self.duration_ns {
            values.push(("duration_ns", duration_ns as f64));
        }
        if let Some(value_size) = self.fields.value_size {
            values.push(("value_size", value_size as f64));
        }
        if let Some(terminal) = self.fields.terminal {
            values.push(("terminal", if terminal { 1.0 } else { 0.0 }));
        }
        values
    }
}

pub(crate) fn escape_entity_segment(segment: &str) -> String {
    segment.replace('\\', "\\\\").replace('/', "\\/")
}

/// Compact adapter-owned lookup derived synchronously from static registration.
#[derive(Clone, Debug, Default)]
pub(super) struct RegistrationIndex {
    entities: HashMap<RegistrationLookup, RegistrationResolution>,
    action_triggers: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RegistrationLookup {
    enclave: String,
    kind: &'static str,
    identity: String,
}

#[derive(Clone, Debug)]
enum RegistrationResolution {
    Unique(String),
    Ambiguous,
}

impl RegistrationIndex {
    fn register(
        &mut self,
        enclave: &str,
        kind: &'static str,
        stable_key: &str,
        display_name: &str,
        path: &str,
    ) {
        self.register_identity(enclave, kind, stable_key, path);
        if display_name != stable_key {
            self.register_identity(enclave, kind, display_name, path);
        }
    }

    fn register_identity(&mut self, enclave: &str, kind: &'static str, identity: &str, path: &str) {
        let lookup = RegistrationLookup {
            enclave: enclave.to_owned(),
            kind,
            identity: identity.to_owned(),
        };
        self.entities
            .entry(lookup)
            .and_modify(|resolution| {
                if !matches!(resolution, RegistrationResolution::Unique(existing) if existing == path)
                {
                    *resolution = RegistrationResolution::Ambiguous;
                }
            })
            .or_insert_with(|| RegistrationResolution::Unique(path.to_owned()));
    }

    pub(super) fn entity_path(&self, fields: &TraceFields, event: &str) -> Option<String> {
        self.resolve_entity(fields, event)
            .map(|path| format!("{}/{}", path, escape_entity_segment(event)))
    }

    pub(super) fn resolve_entity(&self, fields: &TraceFields, event: &str) -> Option<String> {
        let enclave = if event == "propagation_send" {
            fields.destination.as_deref()?
        } else {
            fields.enclave.as_deref()?
        };
        let (kind, identity) = if let Some(identity) = fields.action_key.as_ref() {
            ("action", identity.as_str())
        } else if let Some(identity) = fields.action.as_ref() {
            ("action", identity.as_str())
        } else if let Some(identity) = fields.port_key.as_ref() {
            ("port", identity.as_str())
        } else if let Some(identity) = fields.port.as_ref() {
            ("port", identity.as_str())
        } else if let Some(identity) = fields.reaction_key.as_ref() {
            ("reaction", identity.as_str())
        } else if let Some(identity) = fields.reaction.as_ref() {
            ("reaction", identity.as_str())
        } else {
            ("scheduler", "scheduler")
        };
        let lookup = RegistrationLookup {
            enclave: enclave.to_owned(),
            kind,
            identity: identity.to_owned(),
        };
        match self.entities.get(&lookup) {
            Some(RegistrationResolution::Unique(path)) => Some(path.clone()),
            Some(RegistrationResolution::Ambiguous) | None => None,
        }
    }

    pub(super) fn triggered_reactions(&self, action_path: &str) -> &[String] {
        self.action_triggers
            .get(action_path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn merge(&mut self, other: Self) {
        for (lookup, incoming) in other.entities {
            self.entities
                .entry(lookup)
                .and_modify(|current| {
                    if !matches!(
                        (&*current, &incoming),
                        (RegistrationResolution::Unique(left), RegistrationResolution::Unique(right))
                            if left == right
                    ) {
                        *current = RegistrationResolution::Ambiguous;
                    }
                })
                .or_insert(incoming);
        }
        for (action, reactions) in other.action_triggers {
            let existing = self.action_triggers.entry(action).or_default();
            for reaction in reactions {
                if !existing.contains(&reaction) {
                    existing.push(reaction);
                }
            }
        }
    }
}

pub(super) fn log_runtime_enclaves(
    recording: &rerun::RecordingStream,
    federate: Option<&str>,
    enclaves: &boomerang_tinymap::TinyMap<
        boomerang_runtime::EnclaveKey,
        boomerang_runtime::Enclave,
    >,
    index: &mut RegistrationIndex,
) -> rerun::RecordingStreamResult<()> {
    use boomerang_runtime::{ActionKey, PortKey, ReactionKey, ReactorKey};

    for (enclave_key, enclave) in enclaves.iter() {
        let root = runtime_enclave_root(federate, enclave_key);
        let scheduler = format!("{root}/scheduler");
        let enclave_path = root.clone();
        let mut nodes = vec![enclave_path.clone(), scheduler.clone()];
        let mut edges = vec![(enclave_path.clone(), scheduler.clone(), "owns_scheduler")];
        let enclave_key_string = enclave_key.to_string();
        index.register(
            &enclave_key_string,
            "scheduler",
            "scheduler",
            "scheduler",
            &scheduler,
        );

        log_runtime_entity(
            recording,
            &enclave_path,
            &enclave_key.to_string(),
            &enclave_key.to_string(),
            "enclave",
            &[
                ("boomerang.runtime.owner_key", federate),
                (
                    "boomerang.runtime.type",
                    Some(std::any::type_name::<boomerang_runtime::Enclave>()),
                ),
            ],
        )?;
        log_runtime_entity(
            recording,
            &scheduler,
            "scheduler",
            "scheduler",
            "scheduler",
            &[
                (
                    "boomerang.runtime.owner_key",
                    Some(&enclave_key.to_string()),
                ),
                (
                    "boomerang.runtime.type",
                    Some(std::any::type_name::<boomerang_runtime::Scheduler>()),
                ),
            ],
        )?;

        let reactor_path = |key: ReactorKey| runtime_reactor_path(&root, enclave, key);

        for (key, reactor) in enclave.env.reactors.iter() {
            let path = reactor_path(key);
            let owner = enclave
                .graph
                .reactor_root_scopes
                .get(key)
                .and_then(|scope| {
                    enclave.graph.scopes[*scope]
                        .parent
                        .map(|parent| enclave.graph.scopes[parent].reactor)
                        .filter(|parent| *parent != key)
                });
            let owner_path = owner
                .map(&reactor_path)
                .unwrap_or_else(|| enclave_path.clone());
            nodes.push(path.clone());
            edges.push((owner_path, path.clone(), "owns_reactor"));
            let owner_key = owner
                .map(|owner| owner.to_string())
                .unwrap_or_else(|| enclave_key.to_string());
            log_runtime_entity(
                recording,
                &path,
                reactor.name().rsplit('/').next().unwrap_or(reactor.name()),
                &key.to_string(),
                "reactor",
                &[("boomerang.runtime.owner_key", Some(&owner_key))],
            )?;
        }

        let reaction_levels = reaction_levels(&enclave.graph);
        for (key, reaction) in enclave.env.reactions.iter() {
            let owner = enclave.graph.reaction_reactors[key];
            let path = format!(
                "{}/reactions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            let owner_path = reactor_path(owner);
            nodes.push(path.clone());
            edges.push((owner_path, path.clone(), "owns_reaction"));
            let owner_key = owner.to_string();
            index.register(
                &enclave_key_string,
                "reaction",
                &key.to_string(),
                reaction.get_name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                reaction.get_name(),
                &key.to_string(),
                "reaction",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    (
                        "boomerang.runtime.reaction_level",
                        reaction_levels
                            .get(&key)
                            .map(ToString::to_string)
                            .as_deref(),
                    ),
                ],
            )?;
        }

        for (key, action) in enclave.env.actions.iter() {
            let owner = owner_reactor_for_action(&enclave.graph, key);
            let path = format!(
                "{}/actions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            nodes.push(path.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_action"));
            let owner_key = owner.to_string();
            index.register(
                &enclave_key_string,
                "action",
                &key.to_string(),
                action.name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                action.name(),
                &key.to_string(),
                "action",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    ("boomerang.runtime.type", Some(action.type_name())),
                    (
                        "boomerang.runtime.action_timing",
                        Some(if action.is_logical() {
                            "logical"
                        } else {
                            "physical"
                        }),
                    ),
                ],
            )?;
        }

        for (key, port) in enclave.env.ports.iter() {
            let owner = owner_reactor_for_port(&enclave.graph, key);
            let path = format!(
                "{}/ports/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            );
            nodes.push(path.clone());
            edges.push((reactor_path(owner), path.clone(), "owns_port"));
            let owner_key = owner.to_string();
            index.register(
                &enclave_key_string,
                "port",
                &key.to_string(),
                port.get_name(),
                &path,
            );
            log_runtime_entity(
                recording,
                &path,
                port.get_name(),
                &key.to_string(),
                "port",
                &[
                    ("boomerang.runtime.owner_key", Some(&owner_key)),
                    ("boomerang.runtime.type", Some(port.type_name())),
                ],
            )?;
        }

        let reaction_path = |key: ReactionKey| {
            let owner = enclave.graph.reaction_reactors[key];
            format!(
                "{}/reactions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };
        let action_path = |key: ActionKey| {
            let owner = owner_reactor_for_action(&enclave.graph, key);
            format!(
                "{}/actions/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };
        let port_path = |key: PortKey| {
            let owner = owner_reactor_for_port(&enclave.graph, key);
            format!(
                "{}/ports/{}",
                reactor_path(owner),
                escape_entity_segment(&key.to_string())
            )
        };

        for (action, reactions) in enclave.graph.action_triggers.iter() {
            let action_path = action_path(action);
            let triggered = index
                .action_triggers
                .entry(action_path.clone())
                .or_default();
            for (_, reaction) in reactions {
                let reaction = reaction_path(*reaction);
                if !triggered.contains(&reaction) {
                    triggered.push(reaction);
                }
            }
            edges.extend(
                reactions.iter().map(|(_, reaction)| {
                    (action_path.clone(), reaction_path(*reaction), "triggers")
                }),
            );
        }
        for (port, reactions) in enclave.graph.port_triggers.iter() {
            edges.extend(
                reactions
                    .iter()
                    .map(|(_, reaction)| (port_path(port), reaction_path(*reaction), "triggers")),
            );
        }
        for (reaction, ports) in enclave.graph.reaction_use_ports.iter() {
            edges.extend(
                ports
                    .iter()
                    .map(|port| (port_path(*port), reaction_path(reaction), "uses")),
            );
        }
        for (reaction, ports) in enclave.graph.reaction_effect_ports.iter() {
            edges.extend(
                ports
                    .iter()
                    .map(|port| (reaction_path(reaction), port_path(*port), "effects")),
            );
        }
        for (reaction, actions) in enclave.graph.reaction_actions.iter() {
            edges.extend(actions.iter().map(|action| {
                (
                    reaction_path(reaction),
                    action_path(*action),
                    "action_use_or_effect",
                )
            }));
        }
        for (downstream, _) in enclave.downstream_enclaves.iter() {
            edges.push((
                scheduler.clone(),
                format!("{}/scheduler", runtime_enclave_root(federate, downstream)),
                "scheduler_coordination",
            ));
        }

        for (index, (source, target, kind)) in edges.iter().enumerate() {
            log_runtime_relation(
                recording,
                &format!("{root}/topology/relations/{index}"),
                source,
                target,
                kind,
                None,
                None,
            )?;
        }

        recording.log_static(
            format!("{root}/topology"),
            &rerun::GraphNodes::new(nodes.clone()).with_labels(nodes),
        )?;
        recording.log_static(
            format!("{root}/topology"),
            &rerun::GraphEdges::new(
                edges
                    .iter()
                    .map(|(source, target, _)| (source.as_str(), target.as_str())),
            )
            .with_graph_type(rerun::components::GraphType::Directed),
        )?;
    }
    Ok(())
}

fn runtime_reactor_path(
    root: &str,
    enclave: &boomerang_runtime::Enclave,
    key: boomerang_runtime::ReactorKey,
) -> String {
    let mut key_chain = Vec::new();
    let mut current = Some(key);
    while let Some(reactor_key) = current {
        key_chain.push(reactor_key);
        current = enclave
            .graph
            .reactor_root_scopes
            .get(reactor_key)
            .and_then(|scope| enclave.graph.scopes[*scope].parent)
            .map(|parent| enclave.graph.scopes[parent].reactor)
            .filter(|parent| *parent != reactor_key);
    }
    key_chain.reverse();

    let first_fqn = enclave.env.reactors[key_chain[0]].name();
    let mut hierarchy = first_fqn
        .split('/')
        .map(escape_entity_segment)
        .collect::<Vec<_>>();
    hierarchy.pop();
    hierarchy.extend(key_chain.into_iter().map(|reactor_key| {
        let reactor = &enclave.env.reactors[reactor_key];
        let display_name = reactor.name().rsplit('/').next().unwrap_or(reactor.name());
        format!(
            "{}@{}",
            escape_entity_segment(display_name),
            escape_entity_segment(&reactor_key.to_string())
        )
    }));
    format!("{root}/reactors/{}", hierarchy.join("/reactors/"))
}

pub(super) fn log_runtime_relation(
    recording: &rerun::RecordingStream,
    path: &str,
    source: &str,
    target: &str,
    kind: &str,
    stable_key: Option<&str>,
    delay_ns: Option<u64>,
) -> rerun::RecordingStreamResult<()> {
    let mut relation = rerun::DynamicArchetype::new("boomerang.RuntimeRelation")
        .with_component::<rerun::components::Text>("boomerang.runtime.source", [source])
        .with_component::<rerun::components::Text>("boomerang.runtime.target", [target])
        .with_component::<rerun::components::Text>("boomerang.runtime.relation_kind", [kind]);
    if let Some(stable_key) = stable_key {
        relation = relation.with_component::<rerun::components::Text>(
            "boomerang.runtime.stable_key",
            [stable_key],
        );
    }
    if let Some(delay_ns) = delay_ns {
        relation = relation.with_component_from_data(
            "boomerang.runtime.delay_ns",
            std::sync::Arc::new(rerun::external::arrow::array::UInt64Array::from(vec![
                delay_ns,
            ])),
        );
    }
    recording.log_static(path, &relation)
}

fn log_runtime_entity(
    recording: &rerun::RecordingStream,
    path: &str,
    display_name: &str,
    stable_key: &str,
    kind: &str,
    optional_components: &[(&'static str, Option<&str>)],
) -> rerun::RecordingStreamResult<()> {
    let mut entity = rerun::DynamicArchetype::new("boomerang.RuntimeEntity")
        .with_component::<rerun::components::Text>("boomerang.runtime.display_name", [display_name])
        .with_component::<rerun::components::Text>("boomerang.runtime.stable_key", [stable_key])
        .with_component::<rerun::components::Text>("boomerang.runtime.kind", [kind]);
    for (name, value) in optional_components {
        if let Some(value) = value {
            entity = entity.with_component::<rerun::components::Text>(*name, [*value]);
        }
    }
    recording.log_static(path, &entity)
}

pub(super) fn runtime_enclave_root(
    federate: Option<&str>,
    enclave: boomerang_runtime::EnclaveKey,
) -> String {
    let enclave = escape_entity_segment(&enclave.to_string());
    match federate {
        Some(federate) => format!(
            "/federates/{}/enclaves/{enclave}",
            escape_entity_segment(federate)
        ),
        None => format!("/enclaves/{enclave}"),
    }
}

fn owner_reactor_for_action(
    graph: &boomerang_runtime::ReactionGraph,
    action: boomerang_runtime::ActionKey,
) -> boomerang_runtime::ReactorKey {
    graph.scopes[graph.action_scopes[action]].reactor
}

fn owner_reactor_for_port(
    graph: &boomerang_runtime::ReactionGraph,
    port: boomerang_runtime::PortKey,
) -> boomerang_runtime::ReactorKey {
    graph.scopes[graph.port_scopes[port]].reactor
}

fn reaction_levels(
    graph: &boomerang_runtime::ReactionGraph,
) -> std::collections::BTreeMap<boomerang_runtime::ReactionKey, boomerang_runtime::Level> {
    let mut levels = std::collections::BTreeMap::new();
    for (level, reaction) in graph
        .action_triggers
        .values()
        .flatten()
        .chain(graph.port_triggers.values().flatten())
        .chain(graph.reset_reactions.values().flatten())
        .chain(
            graph
                .startup_reactions
                .values()
                .flatten()
                .map(|entry| &entry.reaction),
        )
        .chain(
            graph
                .shutdown_reactions_by_scope
                .values()
                .flatten()
                .map(|entry| &entry.reaction),
        )
    {
        levels.insert(*reaction, *level);
    }
    levels
}

pub(crate) fn entity_path(fields: &TraceFields, event: &str) -> String {
    if matches!(event, "propagation_send" | "propagation_receive") {
        return format!("/propagation/unresolved/{}", escape_entity_segment(event));
    }
    let enclave = escape_entity_segment(fields.enclave.as_deref().unwrap_or("unknown"));
    if let Some(action) = fields.action_key.as_ref().or(fields.action.as_ref()) {
        return format!(
            "/enclaves/{enclave}/actions/{}/{}",
            escape_entity_segment(action),
            escape_entity_segment(event),
        );
    }
    if let Some(port) = fields.port_key.as_ref().or(fields.port.as_ref()) {
        return format!(
            "/enclaves/{enclave}/ports/{}/{}",
            escape_entity_segment(port),
            escape_entity_segment(event),
        );
    }
    if let (Some(reactor), Some(reaction)) = (
        &fields.reactor,
        fields.reaction_key.as_ref().or(fields.reaction.as_ref()),
    ) {
        return format!(
            "/enclaves/{enclave}/reactors/{}/reactions/{}",
            escape_entity_segment(reactor),
            escape_entity_segment(reaction),
        );
    }
    format!(
        "/enclaves/{enclave}/scheduler/{}",
        escape_entity_segment(event)
    )
}

#[cfg(test)]
mod tests {
    use boomerang_runtime::{Enclave, Reactor};
    use rerun::AsComponents as _;

    use super::*;

    #[test]
    fn duplicate_reactor_names_use_the_supplied_stable_key() {
        let mut enclave = Enclave::default();
        let root = enclave.insert_reactor(Reactor::new("main", ()).boxed(), None);
        let root_scope = enclave.root_scope(root);
        let first = enclave.insert_reactor(Reactor::new("main/duplicate", ()).boxed(), None);
        enclave.set_reactor_scope_parent(first, root_scope);
        let second = enclave.insert_reactor(Reactor::new("main/duplicate", ()).boxed(), None);
        enclave.set_reactor_scope_parent(second, root_scope);

        let first_path = runtime_reactor_path("/enclaves/local", &enclave, first);
        let second_path = runtime_reactor_path("/enclaves/local", &enclave, second);

        assert_eq!(
            first_path,
            "/enclaves/local/reactors/main@ReactorKey(0)/reactors/duplicate@ReactorKey(1)"
        );
        assert_eq!(
            second_path,
            "/enclaves/local/reactors/main@ReactorKey(0)/reactors/duplicate@ReactorKey(2)"
        );
        assert_ne!(first_path, second_path);
    }

    #[test]
    fn unresolved_propagation_never_fabricates_an_action_path() {
        let fields = TraceFields {
            enclave: Some("EnclaveKey(1)".to_owned()),
            destination: Some("EnclaveKey(0)".to_owned()),
            action_key: Some("ActionKey(0)".to_owned()),
            ..TraceFields::default()
        };

        assert_eq!(
            entity_path(&fields, "propagation_send"),
            "/propagation/unresolved/propagation_send"
        );
        assert_eq!(
            entity_path(&fields, "propagation_receive"),
            "/propagation/unresolved/propagation_receive"
        );
    }

    fn record(event: &str) -> TraceRecord {
        TraceRecord {
            id: TraceId("source:e0:1".to_owned()),
            parent_id: None,
            entity_path: "/diagnostics/schema".to_owned(),
            event: event.to_owned(),
            timepoint: TraceTimePoint {
                elapsed_ns: 1,
                wall_clock_unix_ns: 2,
                logical_ns: Some(3),
            },
            microstep: Some(u64::MAX),
            duration_ns: Some(5),
            terminal_state: None,
            fields: TraceFields {
                event: Some(event.to_owned()),
                enclave: Some("e0".to_owned()),
                value_size: Some(u64::MAX),
                error: Some("bad schema".to_owned()),
                ..TraceFields::default()
            },
        }
    }

    #[test]
    fn diagnostics_use_builtin_text_log() {
        assert!(matches!(
            record("diagnostic").primary_payload(),
            PrimaryPayload::TextLog(_)
        ));
    }

    #[test]
    fn operational_payload_preserves_typed_numeric_components() {
        let PrimaryPayload::Dynamic(payload) = record("action_schedule").primary_payload() else {
            panic!("operational records use dynamic components")
        };
        let batches = payload.as_serialized_batches();
        let value_size = batches
            .iter()
            .find(|batch| {
                batch
                    .descriptor
                    .component
                    .as_str()
                    .ends_with(":boomerang.trace.value_size")
            })
            .expect("value_size component");
        assert_eq!(
            value_size.array.data_type(),
            &rerun::external::arrow::datatypes::DataType::UInt64
        );
    }

    #[test]
    fn duration_is_exposed_as_builtin_scalar_series() {
        let series = record("reaction_execute").scalar_series();
        assert!(series.iter().any(|(name, _)| *name == "duration_ns"));
    }

    #[test]
    fn memory_sink_encodes_timelines_typed_components_and_builtin_archetypes() {
        let (recording, memory) = rerun::RecordingStreamBuilder::new("boomerang-memory-behavior")
            .memory()
            .unwrap();
        let writer = RerunTraceWriter;

        let mut logical = record("action_schedule");
        logical.entity_path = "/records/logical".to_owned();
        logical.fields.terminal = Some(true);
        writer.write(&recording, &logical).unwrap();

        let mut non_logical = record("shutdown");
        non_logical.entity_path = "/records/non_logical".to_owned();
        non_logical.timepoint.logical_ns = None;
        writer.write(&recording, &non_logical).unwrap();

        let diagnostic = record("diagnostic");
        writer.write(&recording, &diagnostic).unwrap();

        let chunks = memory
            .take()
            .into_iter()
            .filter_map(|message| match message {
                rerun::log::LogMsg::ArrowMsg(_, message) => {
                    Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let non_logical_chunk = chunks
            .iter()
            .find(|chunk| chunk.entity_path().to_string() == "/records/non_logical")
            .expect("non-logical record chunk");
        assert!(non_logical_chunk
            .timelines()
            .keys()
            .all(|timeline| timeline.as_str() != "logical"));

        let logical_chunk = chunks
            .iter()
            .find(|chunk| chunk.entity_path().to_string() == "/records/logical")
            .expect("logical record chunk");
        assert!(logical_chunk
            .timelines()
            .keys()
            .any(|timeline| timeline.as_str() == "logical"));
        let component = |suffix: &str| {
            logical_chunk
                .components()
                .0
                .values()
                .find(|column| column.descriptor.component.as_str().ends_with(suffix))
                .unwrap_or_else(|| panic!("missing {suffix}"))
        };
        assert_eq!(
            component(":boomerang.trace.value_size")
                .list_array
                .values()
                .data_type(),
            &rerun::external::arrow::datatypes::DataType::UInt64
        );
        assert_eq!(
            component(":boomerang.trace.terminal")
                .list_array
                .values()
                .data_type(),
            &rerun::external::arrow::datatypes::DataType::Boolean
        );
        assert!(chunks
            .iter()
            .any(
                |chunk| chunk.component_descriptors().any(|descriptor| descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "rerun.archetypes.TextLog"))
            ));
        assert!(chunks
            .iter()
            .any(
                |chunk| chunk.component_descriptors().any(|descriptor| descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "rerun.archetypes.Scalars"))
            ));
    }
}
