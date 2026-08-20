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
        inherit!(enclave, reactor, reaction, logical_ns, microstep,);
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

        let mut values = rerun::DynamicArchetype::new("boomerang.TraceRecord");
        for (name, value) in record.component_values() {
            values = values.with_component::<rerun::components::Text>(name, [value]);
        }

        // Rerun's time context is thread-local. Reset before and after each write so a record
        // without logical time cannot inherit it from an earlier record on the same thread. The
        // guard also resets the context during panic unwinding.
        recording.reset_time();
        let _reset = TimeContextReset(recording);
        recording.set_timepoint(timepoint);
        let result = recording.log(record.entity_path.clone(), &values);
        result.map_err(Into::into)
    }
}

impl TraceRecord {
    fn component_values(&self) -> Vec<(&'static str, String)> {
        let mut values = vec![
            ("boomerang.trace.id", self.id.0.clone()),
            ("boomerang.trace.event", self.event.clone()),
        ];
        if let Some(parent_id) = &self.parent_id {
            values.push(("boomerang.trace.parent_id", parent_id.0.clone()));
        }
        if let Some(microstep) = self.microstep {
            values.push(("boomerang.trace.microstep", microstep.to_string()));
        }
        if let Some(duration_ns) = self.duration_ns {
            values.push(("boomerang.trace.duration_ns", duration_ns.to_string()));
        }

        macro_rules! string_fields {
            ($($field:ident),* $(,)?) => {
                $(if let Some(value) = &self.fields.$field {
                    values.push((concat!("boomerang.trace.", stringify!($field)), value.clone()));
                })*
            };
        }
        macro_rules! display_fields {
            ($($field:ident),* $(,)?) => {
                $(if let Some(value) = self.fields.$field {
                    values.push((concat!("boomerang.trace.", stringify!($field)), value.to_string()));
                })*
            };
        }
        string_fields!(
            enclave,
            kind,
            reactor,
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
        display_fields!(
            logical_ns,
            destination_logical_ns,
            destination_microstep,
            old_logical_ns,
            old_microstep,
            terminal,
            value_size,
        );
        values
    }
}

pub(crate) fn escape_entity_segment(segment: &str) -> String {
    segment.replace('\\', "\\\\").replace('/', "\\/")
}

pub(crate) fn entity_path(fields: &TraceFields, event: &str) -> String {
    let enclave = escape_entity_segment(fields.enclave.as_deref().unwrap_or("unknown"));
    if let (Some(reactor), Some(reaction)) = (&fields.reactor, &fields.reaction) {
        return format!(
            "/enclaves/{enclave}/reactors/{}/reactions/{}",
            escape_entity_segment(reactor),
            escape_entity_segment(reaction),
        );
    }
    if let Some(action) = &fields.action {
        return format!(
            "/enclaves/{enclave}/actions/{}/{}",
            escape_entity_segment(action),
            escape_entity_segment(event),
        );
    }
    if let Some(port) = &fields.port {
        return format!(
            "/enclaves/{enclave}/ports/{}/{}",
            escape_entity_segment(port),
            escape_entity_segment(event),
        );
    }
    format!(
        "/enclaves/{enclave}/scheduler/{}",
        escape_entity_segment(event)
    )
}
