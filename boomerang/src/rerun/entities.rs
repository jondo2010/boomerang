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

pub(crate) fn entity_path(fields: &TraceFields, event: &str) -> String {
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
    use rerun::AsComponents as _;

    use super::*;

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
