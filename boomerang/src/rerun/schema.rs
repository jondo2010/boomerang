use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use tracing::field::{Field, Visit};

use super::entities::{TraceId, TraceTimePoint};

/// A complete logical tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TraceTag {
    pub logical_ns: u64,
    pub microstep: u64,
}

/// One validated trace record, independent of its eventual recording format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRecord {
    pub id: TraceId,
    pub parent_id: Option<TraceId>,
    pub entity_path: String,
    pub timepoint: TraceTimePoint,
    pub duration_ns: Option<u64>,
    pub event: TraceEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    SchedulerRunning(SchedulerRunning),
    TagProcessing(TagProcessing),
    ReactionExecution(ReactionExecution),
    CoordinationWait(CoordinationWait),
    LogicalIngress(LogicalIngress),
    PhysicalIngress(PhysicalIngress),
    ControlIngress(ControlIngress),
    ActionScheduled(ActionScheduled),
    ActionStartup(ActionStartup),
    ActionRebased(ActionRebased),
    PortWrite(PortWrite),
    PropagationLogicalSend(PropagationLogicalSend),
    PropagationPhysicalSend(PropagationPhysicalSend),
    PropagationSerializedSend(PropagationSerializedSend),
    PropagationReceive(PropagationReceive),
    FrontierCandidate(FrontierCandidate),
    FrontierState(FrontierState),
    CoordinationGrant(CoordinationGrant),
    TagRelease(TagRelease),
    TagComplete(TagComplete),
    Shutdown(Shutdown),
    RuntimeDiagnostic(RuntimeDiagnostic),
    SchemaDiagnostic(SchemaDiagnostic),
    CausalLink(CausalLink),
}

macro_rules! text_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $($variant),+ }

        impl $name {
            #[allow(dead_code)] // Task 2 wires the migration-only raw parser into the tracing layer.
            fn parse(field: &'static str, value: &str) -> Result<Self, SchemaErrorKind> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(SchemaErrorKind::InvalidValue {
                        field,
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

text_enum!(SchedulerState { Running => "running" });
text_enum!(TagState { Processing => "processing" });
text_enum!(ReactionState { Begin => "begin" });
text_enum!(WaitState { Waiting => "waiting" });
text_enum!(ControlKind {
    ProvisionalRelease => "provisional_release",
    Shutdown => "shutdown",
});
text_enum!(IngressOutcome {
    Accepted => "accepted",
    IgnoredPast => "ignored_past",
});
text_enum!(DeliveryOutcome {
    Accepted => "accepted",
    Failed => "failed",
});
text_enum!(PublishOutcome {
    Published => "published",
    Failed => "failed",
});
text_enum!(CoordinationGrantOutcome {
    Granted => "granted",
    InterruptedLocal => "interrupted_local",
    InterruptedExternal => "interrupted_external",
});
text_enum!(CompletionOutcome {
    Completed => "completed",
    Failed => "failed",
});
text_enum!(FrontierStatus {
    Idle => "idle",
    Finished => "finished",
});
text_enum!(ShutdownState { Complete => "complete" });
text_enum!(ShutdownOutcome { Success => "success" });
text_enum!(CausalState { Exact => "exact" });
text_enum!(CausalOutcome { Matched => "matched" });
text_enum!(PortWriteOutcome {
    MutableAccess => "mutable_access",
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueDescriptor {
    pub value_type: String,
    pub value_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerRunning {
    pub federate: String,
    pub enclave: String,
    pub state: SchedulerState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagProcessing {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub terminal: bool,
    pub state: TagState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionExecution {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub reactor: String,
    pub reaction_key: Option<String>,
    pub reaction: String,
    pub level: String,
    pub state: ReactionState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinationWait {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub state: WaitState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalIngress {
    pub federate: Option<String>,
    pub enclave: String,
    pub action_key: String,
    pub action: String,
    pub tag: TraceTag,
    pub destination_tag: TraceTag,
    pub value: ValueDescriptor,
    pub outcome: IngressOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalIngress {
    pub federate: Option<String>,
    pub enclave: String,
    pub action_key: String,
    pub action: String,
    pub tag: TraceTag,
    pub destination_tag: TraceTag,
    pub value: ValueDescriptor,
    pub outcome: IngressOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlIngress {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub kind: ControlKind,
    pub outcome: IngressOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionScheduled {
    pub federate: Option<String>,
    pub enclave: String,
    pub source_tag: TraceTag,
    pub action_key: String,
    pub action: String,
    pub destination_tag: TraceTag,
    pub value: ValueDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionStartup {
    pub federate: Option<String>,
    pub enclave: String,
    pub source_tag: TraceTag,
    pub action_key: String,
    pub action: String,
    pub destination_tag: TraceTag,
    pub value: ValueDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRebased {
    pub federate: Option<String>,
    pub enclave: Option<String>,
    pub source_tag: Option<TraceTag>,
    pub action_key: String,
    pub old_tag: TraceTag,
    pub destination_tag: TraceTag,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortWrite {
    pub federate: Option<String>,
    pub enclave: String,
    pub reactor: String,
    pub reaction_key: Option<String>,
    pub reaction: String,
    pub tag: TraceTag,
    pub port_key: String,
    pub port: String,
    pub value_type: String,
    pub outcome: PortWriteOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropagationLogicalSend {
    pub federate: Option<String>,
    pub enclave: String,
    pub destination: String,
    pub action_key: String,
    pub action: String,
    pub tag: TraceTag,
    pub value: ValueDescriptor,
    pub outcome: DeliveryOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropagationPhysicalSend {
    pub federate: Option<String>,
    pub enclave: String,
    pub destination: String,
    pub source_tag: Option<TraceTag>,
    pub action_key: String,
    pub action: String,
    pub value: ValueDescriptor,
    pub outcome: DeliveryOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropagationSerializedSend {
    pub federate: Option<String>,
    pub enclave: String,
    pub destination_federate: Option<String>,
    pub action_key: String,
    pub action: String,
    pub tag: TraceTag,
    pub value: ValueDescriptor,
    pub outcome: DeliveryOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropagationReceive {
    pub federate: Option<String>,
    pub enclave: String,
    pub action_key: String,
    pub action: String,
    pub tag: TraceTag,
    pub destination_tag: TraceTag,
    pub value: ValueDescriptor,
    pub outcome: IngressOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontierCandidate {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub outcome: PublishOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontierState {
    pub federate: Option<String>,
    pub enclave: String,
    pub state: FrontierStatus,
    pub outcome: PublishOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinationGrant {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub outcome: CoordinationGrantOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagRelease {
    pub federate: Option<String>,
    pub enclave: String,
    pub destination: String,
    pub tag: TraceTag,
    pub outcome: DeliveryOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagComplete {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub terminal: bool,
    pub outcome: CompletionOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shutdown {
    pub federate: Option<String>,
    pub enclave: String,
    pub tag: TraceTag,
    pub state: ShutdownState,
    pub outcome: ShutdownOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDiagnostic {
    pub federate: Option<String>,
    pub enclave: String,
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaDiagnostic {
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CausalLink {
    pub enclave: String,
    pub source: TraceId,
    pub destination: TraceId,
    pub tag: TraceTag,
    pub state: CausalState,
    pub outcome: CausalOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaError {
    pub event: Option<String>,
    pub kind: SchemaErrorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchemaErrorKind {
    UnknownField(String),
    UnexpectedField {
        event: String,
        field: String,
    },
    MissingField(&'static str),
    WrongType {
        field: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    NegativeUnsignedValue(&'static str),
    UnknownEvent(String),
    AdapterOwnedEvent(String),
    InvalidValue {
        field: &'static str,
        value: String,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(event) = &self.event {
            write!(formatter, "invalid trace event `{event}`: ")?;
        } else {
            formatter.write_str("invalid trace record: ")?;
        }
        match &self.kind {
            SchemaErrorKind::UnknownField(field) => write!(formatter, "unknown field `{field}`"),
            SchemaErrorKind::UnexpectedField { event, field } => {
                write!(formatter, "field `{field}` does not belong to `{event}`")
            }
            SchemaErrorKind::MissingField(field) => write!(formatter, "missing field `{field}`"),
            SchemaErrorKind::WrongType {
                field,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "field `{field}` must be {expected}, not {actual}"
                )
            }
            SchemaErrorKind::NegativeUnsignedValue(field) => {
                write!(formatter, "unsigned field `{field}` cannot be negative")
            }
            SchemaErrorKind::UnknownEvent(event) => write!(formatter, "unknown event `{event}`"),
            SchemaErrorKind::AdapterOwnedEvent(event) => {
                write!(formatter, "event `{event}` is owned by the trace adapter")
            }
            SchemaErrorKind::InvalidValue { field, value } => {
                write!(formatter, "invalid `{field}` value `{value}`")
            }
        }
    }
}

impl Error for SchemaError {}

#[allow(dead_code)] // Task 2 wires this migration-only visitor state into the tracing layer.
#[derive(Clone, Debug, PartialEq)]
enum RawValue {
    Text(String),
    U64(u64),
    I64(i64),
    F64(f64),
    I128(i128),
    U128(u128),
    Bool(bool),
    Bytes(Vec<u8>),
    Error(String),
}

#[allow(dead_code)] // Task 2 wires this migration-only visitor state into the tracing layer.
impl RawValue {
    fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::U64(_) => "u64",
            Self::I64(_) => "i64",
            Self::F64(_) => "f64",
            Self::I128(_) => "i128",
            Self::U128(_) => "u128",
            Self::Bool(_) => "bool",
            Self::Bytes(_) => "bytes",
            Self::Error(_) => "error",
        }
    }
}

/// Transient tracing visitor state. Values retain the primitive type supplied by `tracing`.
#[allow(dead_code)] // Task 2 replaces the existing dynamic layer visitor with this state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawTraceFields {
    values: BTreeMap<String, RawValue>,
}

#[allow(dead_code)] // Task 2 wires this migration-only parser into the tracing layer.
impl RawTraceFields {
    fn from_iter<'a>(fields: impl IntoIterator<Item = (&'a str, RawValue)>) -> Self {
        let mut raw = Self::default();
        for (name, value) in fields {
            raw.insert(name, value);
        }
        raw
    }

    fn insert(&mut self, name: &str, value: RawValue) {
        self.values.insert(name.to_owned(), value);
    }

    fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub(crate) fn inherit_missing(&mut self, parent: &Self) {
        for name in [
            "federate",
            "enclave",
            "reactor",
            "reaction_key",
            "reaction",
            "logical_ns",
            "microstep",
        ] {
            if !self.values.contains_key(name) {
                if let Some(value) = parent.values.get(name) {
                    self.values.insert(name.to_owned(), value.clone());
                }
            }
        }
    }

    pub(crate) fn parse(&self) -> Result<TraceEvent, SchemaError> {
        let event = match self.values.get("event") {
            Some(RawValue::Text(value)) => value.clone(),
            Some(value) => {
                return Err(SchemaError {
                    event: None,
                    kind: wrong_type("event", "text", value),
                });
            }
            None => {
                return Err(SchemaError {
                    event: None,
                    kind: SchemaErrorKind::MissingField("event"),
                });
            }
        };
        let reader = Reader {
            raw: self,
            event: &event,
        };
        reader.validate_known().map_err(|kind| SchemaError {
            event: Some(event.clone()),
            kind,
        })?;
        self.parse_event(&reader).map_err(|kind| SchemaError {
            event: Some(event),
            kind,
        })
    }

    fn parse_event(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        match r.event {
            "scheduler_thread" => {
                r.allow(&["federate", "enclave", "state"])?;
                Ok(TraceEvent::SchedulerRunning(SchedulerRunning {
                    federate: r.text("federate")?,
                    enclave: r.text("enclave")?,
                    state: SchedulerState::parse("state", &r.text("state")?)?,
                }))
            }
            "tag_process" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "terminal",
                    "state",
                ])?;
                Ok(TraceEvent::TagProcessing(TagProcessing {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    terminal: r.boolean("terminal")?,
                    state: TagState::parse("state", &r.text("state")?)?,
                }))
            }
            "reaction_execute" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "reactor",
                    "reaction_key",
                    "reaction",
                    "level",
                    "state",
                ])?;
                Ok(TraceEvent::ReactionExecution(ReactionExecution {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    reactor: r.text("reactor")?,
                    reaction_key: r.optional_text("reaction_key")?,
                    reaction: r.text("reaction")?,
                    level: r.text("level")?,
                    state: ReactionState::parse("state", &r.text("state")?)?,
                }))
            }
            "coordination_wait" => {
                r.allow(&["federate", "enclave", "logical_ns", "microstep", "state"])?;
                Ok(TraceEvent::CoordinationWait(CoordinationWait {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    state: WaitState::parse("state", &r.text("state")?)?,
                }))
            }
            "async_ingress" => self.parse_ingress(r),
            "action_schedule" => self.parse_action(r),
            "port_write" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "reactor",
                    "reaction_key",
                    "reaction",
                    "logical_ns",
                    "microstep",
                    "port_key",
                    "port",
                    "value_type",
                    "outcome",
                ])?;
                Ok(TraceEvent::PortWrite(PortWrite {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    reactor: r.text("reactor")?,
                    reaction_key: r.optional_text("reaction_key")?,
                    reaction: r.text("reaction")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    port_key: r.text("port_key")?,
                    port: r.text("port")?,
                    value_type: r.text("value_type")?,
                    outcome: PortWriteOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            "propagation_send" => self.parse_send(r),
            "propagation_receive" | "causal_link" => {
                Err(SchemaErrorKind::AdapterOwnedEvent(r.event.to_owned()))
            }
            "frontier_publish" => self.parse_frontier(r),
            "coordination_grant" => {
                r.allow(&["federate", "enclave", "logical_ns", "microstep", "outcome"])?;
                Ok(TraceEvent::CoordinationGrant(CoordinationGrant {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    outcome: CoordinationGrantOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            "tag_release" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "destination",
                    "logical_ns",
                    "microstep",
                    "outcome",
                ])?;
                Ok(TraceEvent::TagRelease(TagRelease {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    destination: r.text("destination")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    outcome: DeliveryOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            "tag_complete" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "terminal",
                    "outcome",
                ])?;
                Ok(TraceEvent::TagComplete(TagComplete {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    terminal: r.boolean("terminal")?,
                    outcome: CompletionOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            "shutdown" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "state",
                    "outcome",
                ])?;
                Ok(TraceEvent::Shutdown(Shutdown {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    state: ShutdownState::parse("state", &r.text("state")?)?,
                    outcome: ShutdownOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            "diagnostic" => self.parse_diagnostic(r),
            other => Err(SchemaErrorKind::UnknownEvent(other.to_owned())),
        }
    }

    fn parse_ingress(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        let kind = r.text("kind")?;
        match kind.as_str() {
            "logical" | "physical" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "kind",
                    "action_key",
                    "action",
                    "logical_ns",
                    "microstep",
                    "destination_logical_ns",
                    "destination_microstep",
                    "value_type",
                    "value_size",
                    "outcome",
                ])?;
                let federate = r.optional_text("federate")?;
                let enclave = r.text("enclave")?;
                let action_key = r.text("action_key")?;
                let action = r.text("action")?;
                let tag = r.tag("logical_ns", "microstep")?;
                let destination_tag = r.tag("destination_logical_ns", "destination_microstep")?;
                let value = r.value()?;
                let outcome = IngressOutcome::parse("outcome", &r.text("outcome")?)?;
                if kind == "logical" {
                    Ok(TraceEvent::LogicalIngress(LogicalIngress {
                        federate,
                        enclave,
                        action_key,
                        action,
                        tag,
                        destination_tag,
                        value,
                        outcome,
                    }))
                } else {
                    Ok(TraceEvent::PhysicalIngress(PhysicalIngress {
                        federate,
                        enclave,
                        action_key,
                        action,
                        tag,
                        destination_tag,
                        value,
                        outcome,
                    }))
                }
            }
            "provisional_release" | "shutdown" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "kind",
                    "logical_ns",
                    "microstep",
                    "outcome",
                ])?;
                Ok(TraceEvent::ControlIngress(ControlIngress {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    kind: ControlKind::parse("kind", &kind)?,
                    outcome: IngressOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            _ => Err(SchemaErrorKind::InvalidValue {
                field: "kind",
                value: kind,
            }),
        }
    }

    fn parse_action(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        let outcome = r.text("outcome")?;
        match outcome.as_str() {
            "scheduled" | "startup" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "action_key",
                    "action",
                    "destination_logical_ns",
                    "destination_microstep",
                    "value_type",
                    "value_size",
                    "outcome",
                ])?;
                let federate = r.optional_text("federate")?;
                let enclave = r.text("enclave")?;
                let source_tag = r.tag("logical_ns", "microstep")?;
                let action_key = r.text("action_key")?;
                let action = r.text("action")?;
                let destination_tag = r.tag("destination_logical_ns", "destination_microstep")?;
                let value = r.value()?;
                if outcome == "scheduled" {
                    Ok(TraceEvent::ActionScheduled(ActionScheduled {
                        federate,
                        enclave,
                        source_tag,
                        action_key,
                        action,
                        destination_tag,
                        value,
                    }))
                } else {
                    Ok(TraceEvent::ActionStartup(ActionStartup {
                        federate,
                        enclave,
                        source_tag,
                        action_key,
                        action,
                        destination_tag,
                        value,
                    }))
                }
            }
            "rebased" => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "logical_ns",
                    "microstep",
                    "action_key",
                    "old_logical_ns",
                    "old_microstep",
                    "destination_logical_ns",
                    "destination_microstep",
                    "outcome",
                ])?;
                Ok(TraceEvent::ActionRebased(ActionRebased {
                    federate: r.optional_text("federate")?,
                    enclave: r.optional_text("enclave")?,
                    source_tag: r.optional_tag("logical_ns", "microstep")?,
                    action_key: r.text("action_key")?,
                    old_tag: r.tag("old_logical_ns", "old_microstep")?,
                    destination_tag: r.tag("destination_logical_ns", "destination_microstep")?,
                }))
            }
            _ => Err(SchemaErrorKind::InvalidValue {
                field: "outcome",
                value: outcome,
            }),
        }
    }

    fn parse_send(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        let kind = r.text("kind")?;
        let local = r.raw.values.contains_key("destination");
        match (kind.as_str(), local) {
            ("logical", true) => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "kind",
                    "destination",
                    "action_key",
                    "action",
                    "logical_ns",
                    "microstep",
                    "value_type",
                    "value_size",
                    "outcome",
                ])?;
                Ok(TraceEvent::PropagationLogicalSend(PropagationLogicalSend {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    destination: r.text("destination")?,
                    action_key: r.text("action_key")?,
                    action: r.text("action")?,
                    tag: r.tag("logical_ns", "microstep")?,
                    value: r.value()?,
                    outcome: DeliveryOutcome::parse("outcome", &r.text("outcome")?)?,
                }))
            }
            ("physical", true) => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "kind",
                    "destination",
                    "action_key",
                    "action",
                    "logical_ns",
                    "microstep",
                    "value_type",
                    "value_size",
                    "outcome",
                ])?;
                Ok(TraceEvent::PropagationPhysicalSend(
                    PropagationPhysicalSend {
                        federate: r.optional_text("federate")?,
                        enclave: r.text("enclave")?,
                        destination: r.text("destination")?,
                        source_tag: r.optional_tag("logical_ns", "microstep")?,
                        action_key: r.text("action_key")?,
                        action: r.text("action")?,
                        value: r.value()?,
                        outcome: DeliveryOutcome::parse("outcome", &r.text("outcome")?)?,
                    },
                ))
            }
            ("logical", false) => {
                r.allow(&[
                    "federate",
                    "enclave",
                    "kind",
                    "destination_federate",
                    "action_key",
                    "action",
                    "logical_ns",
                    "microstep",
                    "value_type",
                    "value_size",
                    "outcome",
                ])?;
                Ok(TraceEvent::PropagationSerializedSend(
                    PropagationSerializedSend {
                        federate: r.optional_text("federate")?,
                        enclave: r.text("enclave")?,
                        destination_federate: r.optional_text("destination_federate")?,
                        action_key: r.text("action_key")?,
                        action: r.text("action")?,
                        tag: r.tag("logical_ns", "microstep")?,
                        value: r.value()?,
                        outcome: DeliveryOutcome::parse("outcome", &r.text("outcome")?)?,
                    },
                ))
            }
            _ => Err(SchemaErrorKind::InvalidValue {
                field: "kind",
                value: kind,
            }),
        }
    }

    fn parse_frontier(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        let state = r.text("state")?;
        let outcome = PublishOutcome::parse("outcome", &r.text("outcome")?)?;
        if state == "candidate" {
            r.allow(&[
                "federate",
                "enclave",
                "state",
                "logical_ns",
                "microstep",
                "outcome",
            ])?;
            Ok(TraceEvent::FrontierCandidate(FrontierCandidate {
                federate: r.optional_text("federate")?,
                enclave: r.text("enclave")?,
                tag: r.tag("logical_ns", "microstep")?,
                outcome,
            }))
        } else {
            r.allow(&["federate", "enclave", "state", "outcome"])?;
            Ok(TraceEvent::FrontierState(FrontierState {
                federate: r.optional_text("federate")?,
                enclave: r.text("enclave")?,
                state: FrontierStatus::parse("state", &state)?,
                outcome,
            }))
        }
    }

    fn parse_diagnostic(&self, r: &Reader<'_>) -> Result<TraceEvent, SchemaErrorKind> {
        let state = r.text("state")?;
        match state.as_str() {
            "runtime_error" => {
                r.allow(&["federate", "enclave", "state", "outcome", "error"])?;
                r.expect("outcome", "failed")?;
                Ok(TraceEvent::RuntimeDiagnostic(RuntimeDiagnostic {
                    federate: r.optional_text("federate")?,
                    enclave: r.text("enclave")?,
                    error: r.text("error")?,
                }))
            }
            "schema_error" => {
                r.allow(&["state", "outcome", "error"])?;
                r.expect("outcome", "ignored")?;
                Ok(TraceEvent::SchemaDiagnostic(SchemaDiagnostic {
                    error: r.text("error")?,
                }))
            }
            _ => Err(SchemaErrorKind::InvalidValue {
                field: "state",
                value: state,
            }),
        }
    }
}

impl Visit for RawTraceFields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field.name(), RawValue::U64(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field.name(), RawValue::I64(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field.name(), RawValue::F64(value));
    }
    fn record_i128(&mut self, field: &Field, value: i128) {
        self.insert(field.name(), RawValue::I128(value));
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.insert(field.name(), RawValue::U128(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field.name(), RawValue::Bool(value));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field.name(), RawValue::Text(value.to_owned()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut text = format!("{value:?}");
        if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
            text = text[1..text.len() - 1].to_owned();
        }
        self.insert(field.name(), RawValue::Text(text));
    }
    fn record_bytes(&mut self, field: &Field, value: &[u8]) {
        self.insert(field.name(), RawValue::Bytes(value.to_vec()));
    }
    fn record_error(&mut self, field: &Field, value: &(dyn Error + 'static)) {
        self.insert(field.name(), RawValue::Error(value.to_string()));
    }
}

#[allow(dead_code)] // Task 2 wires this migration-only parser into the tracing layer.
struct Reader<'a> {
    raw: &'a RawTraceFields,
    event: &'a str,
}

#[allow(dead_code)] // Task 2 wires this migration-only parser into the tracing layer.
impl Reader<'_> {
    fn validate_known(&self) -> Result<(), SchemaErrorKind> {
        const KNOWN: &[&str] = &[
            "event",
            "federate",
            "enclave",
            "kind",
            "reactor",
            "reaction_key",
            "reaction",
            "action_key",
            "action",
            "port_key",
            "port",
            "logical_ns",
            "microstep",
            "destination",
            "destination_federate",
            "source",
            "destination_logical_ns",
            "destination_microstep",
            "old_logical_ns",
            "old_microstep",
            "level",
            "state",
            "terminal",
            "value_type",
            "value_size",
            "outcome",
            "error",
        ];
        if let Some(field) = self
            .raw
            .values
            .keys()
            .find(|name| !KNOWN.contains(&name.as_str()))
        {
            return Err(SchemaErrorKind::UnknownField(field.clone()));
        }
        Ok(())
    }

    fn allow(&self, allowed: &[&str]) -> Result<(), SchemaErrorKind> {
        if let Some(field) = self
            .raw
            .values
            .keys()
            .find(|name| name.as_str() != "event" && !allowed.contains(&name.as_str()))
        {
            return Err(SchemaErrorKind::UnexpectedField {
                event: self.event.to_owned(),
                field: field.clone(),
            });
        }
        Ok(())
    }

    fn text(&self, field: &'static str) -> Result<String, SchemaErrorKind> {
        match self.raw.values.get(field) {
            Some(RawValue::Text(value)) => Ok(value.clone()),
            Some(value) => Err(wrong_type(field, "text", value)),
            None => Err(SchemaErrorKind::MissingField(field)),
        }
    }

    fn optional_text(&self, field: &'static str) -> Result<Option<String>, SchemaErrorKind> {
        self.raw
            .values
            .get(field)
            .map(|_| self.text(field))
            .transpose()
    }

    fn u64(&self, field: &'static str) -> Result<u64, SchemaErrorKind> {
        match self.raw.values.get(field) {
            Some(RawValue::U64(value)) => Ok(*value),
            Some(RawValue::I64(value)) if *value < 0 => {
                Err(SchemaErrorKind::NegativeUnsignedValue(field))
            }
            Some(value) => Err(wrong_type(field, "u64", value)),
            None => Err(SchemaErrorKind::MissingField(field)),
        }
    }

    fn boolean(&self, field: &'static str) -> Result<bool, SchemaErrorKind> {
        match self.raw.values.get(field) {
            Some(RawValue::Bool(value)) => Ok(*value),
            Some(value) => Err(wrong_type(field, "bool", value)),
            None => Err(SchemaErrorKind::MissingField(field)),
        }
    }

    fn tag(
        &self,
        logical: &'static str,
        microstep: &'static str,
    ) -> Result<TraceTag, SchemaErrorKind> {
        Ok(TraceTag {
            logical_ns: self.u64(logical)?,
            microstep: self.u64(microstep)?,
        })
    }

    fn optional_tag(
        &self,
        logical: &'static str,
        microstep: &'static str,
    ) -> Result<Option<TraceTag>, SchemaErrorKind> {
        match (
            self.raw.values.contains_key(logical),
            self.raw.values.contains_key(microstep),
        ) {
            (false, false) => Ok(None),
            _ => self.tag(logical, microstep).map(Some),
        }
    }

    fn value(&self) -> Result<ValueDescriptor, SchemaErrorKind> {
        Ok(ValueDescriptor {
            value_type: self.text("value_type")?,
            value_size: self.u64("value_size")?,
        })
    }

    fn expect(&self, field: &'static str, expected: &'static str) -> Result<(), SchemaErrorKind> {
        let value = self.text(field)?;
        if value == expected {
            Ok(())
        } else {
            Err(SchemaErrorKind::InvalidValue { field, value })
        }
    }
}

#[allow(dead_code)] // Task 2 wires this migration-only parser into the tracing layer.
fn wrong_type(field: &'static str, expected: &'static str, value: &RawValue) -> SchemaErrorKind {
    SchemaErrorKind::WrongType {
        field,
        expected,
        actual: value.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::Layer;

    #[derive(Clone, Default)]
    struct RawCapture(Arc<Mutex<Vec<RawTraceFields>>>);

    impl<S: Subscriber> Layer<S> for RawCapture {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = RawTraceFields::default();
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields);
        }
    }

    struct Text(&'static str);

    impl fmt::Display for Text {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    fn text(value: &str) -> RawValue {
        RawValue::Text(value.to_owned())
    }

    fn raw(fields: &[(&str, RawValue)]) -> RawTraceFields {
        RawTraceFields::from_iter(fields.iter().cloned())
    }

    #[test]
    fn visitor_preserves_explicit_primitive_kinds_and_formatted_text_semantics() {
        let capture = RawCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(event = 1.5_f64);
            tracing::trace!(event = 7_i128);
            tracing::trace!(event = 7_u128);
            tracing::trace!(event = &b"bytes"[..]);
            let error = std::io::Error::other("boom");
            tracing::trace!(event = &error as &(dyn Error + 'static));
            tracing::trace!(
                event = %Text("shutdown"),
                enclave = %Text("e0"),
                logical_ns = 1_u64,
                microstep = 0_u64,
                state = %Text("complete"),
                outcome = %Text("success"),
            );
        });

        let records = capture.0.lock().unwrap();
        assert_eq!(records.len(), 6);
        for (record, expected_kind) in records[..5]
            .iter()
            .zip(["f64", "i128", "u128", "bytes", "error"])
        {
            assert!(matches!(
                record.parse(),
                Err(SchemaError {
                    kind: SchemaErrorKind::WrongType {
                        field: "event",
                        expected: "text",
                        actual,
                    },
                    ..
                }) if actual == expected_kind
            ));
        }
        assert!(matches!(records[5].parse(), Ok(TraceEvent::Shutdown(_))));
    }

    fn common(event: &str) -> Vec<(&str, RawValue)> {
        vec![
            ("event", text(event)),
            ("federate", text("f0")),
            ("enclave", text("e0")),
        ]
    }

    fn tagged(event: &str) -> Vec<(&str, RawValue)> {
        let mut fields = common(event);
        fields.extend([
            ("logical_ns", RawValue::U64(10)),
            ("microstep", RawValue::U64(2)),
        ]);
        fields
    }

    #[test]
    fn parses_reaction_execution() {
        let mut fields = tagged("reaction_execute");
        fields.extend([
            ("reactor", text("root")),
            ("reaction_key", text("r0")),
            ("reaction", text("respond")),
            ("level", text("3")),
            ("state", text("begin")),
        ]);

        assert!(matches!(
            raw(&fields).parse(),
            Ok(TraceEvent::ReactionExecution(ReactionExecution {
                tag: TraceTag {
                    logical_ns: 10,
                    microstep: 2
                },
                ..
            }))
        ));
    }

    #[test]
    fn parses_logical_and_physical_ingress() {
        let mut logical = tagged("async_ingress");
        logical.extend([
            ("kind", text("logical")),
            ("action_key", text("a0")),
            ("action", text("input")),
            ("destination_logical_ns", RawValue::U64(10)),
            ("destination_microstep", RawValue::U64(2)),
            ("value_type", text("u32")),
            ("value_size", RawValue::U64(4)),
            ("outcome", text("accepted")),
        ]);
        let mut physical = logical.clone();
        physical
            .iter_mut()
            .find(|(name, _)| *name == "kind")
            .unwrap()
            .1 = text("physical");

        assert!(matches!(
            raw(&logical).parse(),
            Ok(TraceEvent::LogicalIngress(_))
        ));
        assert!(matches!(
            raw(&physical).parse(),
            Ok(TraceEvent::PhysicalIngress(_))
        ));
    }

    #[test]
    fn parses_scheduled_startup_and_rebased_actions() {
        let mut scheduled = tagged("action_schedule");
        scheduled.extend([
            ("action_key", text("a0")),
            ("action", text("tick")),
            ("destination_logical_ns", RawValue::U64(12)),
            ("destination_microstep", RawValue::U64(0)),
            ("value_type", text("u32")),
            ("value_size", RawValue::U64(4)),
            ("outcome", text("scheduled")),
        ]);
        let mut startup = scheduled.clone();
        startup
            .iter_mut()
            .find(|(name, _)| *name == "outcome")
            .unwrap()
            .1 = text("startup");
        let mut rebased = vec![
            ("event", text("action_schedule")),
            ("action_key", text("a0")),
            ("old_logical_ns", RawValue::U64(10)),
            ("old_microstep", RawValue::U64(1)),
            ("destination_logical_ns", RawValue::U64(11)),
            ("destination_microstep", RawValue::U64(0)),
            ("outcome", text("rebased")),
        ];

        assert!(matches!(
            raw(&scheduled).parse(),
            Ok(TraceEvent::ActionScheduled(_))
        ));
        assert!(matches!(
            raw(&startup).parse(),
            Ok(TraceEvent::ActionStartup(_))
        ));
        assert!(matches!(
            raw(&rebased).parse(),
            Ok(TraceEvent::ActionRebased(_))
        ));
        rebased.push(("value_size", RawValue::U64(4)));
        assert!(matches!(
            raw(&rebased).parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::UnexpectedField { .. },
                ..
            })
        ));
    }

    fn propagation(kind: &str) -> Vec<(&str, RawValue)> {
        let mut fields = tagged("propagation_send");
        fields.extend([
            ("kind", text(kind)),
            ("action_key", text("a0")),
            ("action", text("input")),
            ("value_type", text("u32")),
            ("value_size", RawValue::U64(4)),
            ("outcome", text("accepted")),
        ]);
        fields
    }

    #[test]
    fn splits_local_logical_local_physical_and_serialized_propagation() {
        let mut logical = propagation("logical");
        logical.push(("destination", text("e1")));
        let mut physical = propagation("physical");
        physical.retain(|(name, _)| !matches!(*name, "logical_ns" | "microstep"));
        physical.push(("destination", text("e1")));
        let mut serialized_some = propagation("logical");
        serialized_some.push(("destination_federate", text("f1")));
        let serialized_none = propagation("logical");
        let mut conflicting = propagation("logical");
        conflicting.extend([
            ("destination", text("e1")),
            ("destination_federate", text("f1")),
        ]);

        assert!(matches!(
            raw(&logical).parse(),
            Ok(TraceEvent::PropagationLogicalSend(_))
        ));
        assert!(matches!(
            raw(&physical).parse(),
            Ok(TraceEvent::PropagationPhysicalSend(_))
        ));
        assert!(matches!(
            raw(&serialized_some).parse(),
            Ok(TraceEvent::PropagationSerializedSend(_))
        ));
        assert!(matches!(
            raw(&serialized_none).parse(),
            Ok(TraceEvent::PropagationSerializedSend(
                PropagationSerializedSend {
                    destination_federate: None,
                    ..
                }
            ))
        ));
        assert!(matches!(
            raw(&conflicting).parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::UnexpectedField { .. },
                ..
            })
        ));
    }

    #[test]
    fn parses_direct_runtime_event_families_and_typed_outcomes() {
        type EventCheck = fn(&TraceEvent) -> bool;
        let cases: Vec<(RawTraceFields, EventCheck)> = vec![
            (
                raw(&[
                    ("event", text("scheduler_thread")),
                    ("federate", text("f0")),
                    ("enclave", text("e0")),
                    ("state", text("running")),
                ]),
                |event| matches!(event, TraceEvent::SchedulerRunning(_)),
            ),
            (
                raw(&[
                    ("event", text("tag_process")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("terminal", RawValue::Bool(false)),
                    ("state", text("processing")),
                ]),
                |event| matches!(event, TraceEvent::TagProcessing(_)),
            ),
            (
                raw(&[
                    ("event", text("coordination_wait")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("state", text("waiting")),
                ]),
                |event| matches!(event, TraceEvent::CoordinationWait(_)),
            ),
            (
                raw(&[
                    ("event", text("async_ingress")),
                    ("enclave", text("e0")),
                    ("kind", text("provisional_release")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("accepted")),
                ]),
                |event| matches!(event, TraceEvent::ControlIngress(_)),
            ),
            (
                raw(&[
                    ("event", text("port_write")),
                    ("enclave", text("e0")),
                    ("reactor", text("root")),
                    ("reaction", text("respond")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("port_key", text("p0")),
                    ("port", text("output")),
                    ("value_type", text("u32")),
                    ("outcome", text("mutable_access")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::PortWrite(PortWrite {
                            outcome: PortWriteOutcome::MutableAccess,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("frontier_publish")),
                    ("enclave", text("e0")),
                    ("state", text("candidate")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("published")),
                ]),
                |event| matches!(event, TraceEvent::FrontierCandidate(_)),
            ),
            (
                raw(&[
                    ("event", text("frontier_publish")),
                    ("enclave", text("e0")),
                    ("state", text("idle")),
                    ("outcome", text("published")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::FrontierState(FrontierState {
                            state: FrontierStatus::Idle,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("frontier_publish")),
                    ("enclave", text("e0")),
                    ("state", text("finished")),
                    ("outcome", text("failed")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::FrontierState(FrontierState {
                            state: FrontierStatus::Finished,
                            outcome: PublishOutcome::Failed,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("coordination_grant")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("granted")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::CoordinationGrant(CoordinationGrant {
                            outcome: CoordinationGrantOutcome::Granted,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("coordination_grant")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("interrupted_local")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::CoordinationGrant(CoordinationGrant {
                            outcome: CoordinationGrantOutcome::InterruptedLocal,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("coordination_grant")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("interrupted_external")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::CoordinationGrant(CoordinationGrant {
                            outcome: CoordinationGrantOutcome::InterruptedExternal,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("tag_release")),
                    ("enclave", text("e0")),
                    ("destination", text("e1")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("accepted")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::TagRelease(TagRelease {
                            outcome: DeliveryOutcome::Accepted,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("tag_release")),
                    ("enclave", text("e0")),
                    ("destination", text("e1")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("outcome", text("failed")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::TagRelease(TagRelease {
                            outcome: DeliveryOutcome::Failed,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("tag_complete")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("terminal", RawValue::Bool(false)),
                    ("outcome", text("completed")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::TagComplete(TagComplete {
                            terminal: false,
                            outcome: CompletionOutcome::Completed,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("tag_complete")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("terminal", RawValue::Bool(true)),
                    ("outcome", text("failed")),
                ]),
                |event| {
                    matches!(
                        event,
                        TraceEvent::TagComplete(TagComplete {
                            terminal: true,
                            outcome: CompletionOutcome::Failed,
                            ..
                        })
                    )
                },
            ),
            (
                raw(&[
                    ("event", text("shutdown")),
                    ("enclave", text("e0")),
                    ("logical_ns", RawValue::U64(1)),
                    ("microstep", RawValue::U64(0)),
                    ("state", text("complete")),
                    ("outcome", text("success")),
                ]),
                |event| matches!(event, TraceEvent::Shutdown(_)),
            ),
        ];

        for (raw, check) in cases {
            let event = raw.parse().expect("direct runtime event parses");
            assert!(check(&event), "unexpected parsed event: {event:?}");
        }
    }

    #[test]
    fn parses_runtime_and_schema_diagnostics() {
        let runtime = raw(&[
            ("event", text("diagnostic")),
            ("enclave", text("e0")),
            ("state", text("runtime_error")),
            ("outcome", text("failed")),
            ("error", text("boom")),
        ]);
        let schema = raw(&[
            ("event", text("diagnostic")),
            ("state", text("schema_error")),
            ("outcome", text("ignored")),
            ("error", text("bad record")),
        ]);

        assert!(matches!(
            runtime.parse(),
            Ok(TraceEvent::RuntimeDiagnostic(_))
        ));
        assert!(matches!(
            schema.parse(),
            Ok(TraceEvent::SchemaDiagnostic(_))
        ));
    }

    #[test]
    fn adapter_owned_events_are_constructible_but_rejected_from_raw_runtime_fields() {
        let receive = TraceEvent::PropagationReceive(PropagationReceive {
            federate: Some("f0".to_owned()),
            enclave: "e0".to_owned(),
            action_key: "a0".to_owned(),
            action: "input".to_owned(),
            tag: TraceTag {
                logical_ns: 10,
                microstep: 2,
            },
            destination_tag: TraceTag {
                logical_ns: 10,
                microstep: 2,
            },
            value: ValueDescriptor {
                value_type: "u32".to_owned(),
                value_size: 4,
            },
            outcome: IngressOutcome::Accepted,
        });
        let link = TraceEvent::CausalLink(CausalLink {
            enclave: "e0".to_owned(),
            source: TraceId("send-1".to_owned()),
            destination: TraceId("receive-1".to_owned()),
            tag: TraceTag {
                logical_ns: 10,
                microstep: 2,
            },
            state: CausalState::Exact,
            outcome: CausalOutcome::Matched,
        });
        assert!(matches!(receive, TraceEvent::PropagationReceive(_)));
        assert!(matches!(link, TraceEvent::CausalLink(_)));

        let raw_receive = raw(&[
            ("event", text("propagation_receive")),
            ("enclave", text("e0")),
            ("action_key", text("a0")),
            ("logical_ns", RawValue::U64(10)),
            ("microstep", RawValue::U64(2)),
        ]);
        let raw_link = raw(&[
            ("event", text("causal_link")),
            ("source", text("forged-send")),
            ("destination", text("forged-receive")),
        ]);
        for raw in [raw_receive, raw_link] {
            assert!(matches!(
                raw.parse(),
                Err(SchemaError {
                    kind: SchemaErrorKind::AdapterOwnedEvent(_),
                    ..
                })
            ));
        }
    }

    #[test]
    fn rejects_unknown_missing_wrong_typed_and_negative_fields() {
        let cases = [
            raw(&[("event", text("future_event"))]),
            raw(&[("event", text("reaction_execute"))]),
            raw(&[
                ("event", text("tag_process")),
                ("enclave", text("e0")),
                ("logical_ns", text("10")),
                ("microstep", RawValue::U64(0)),
                ("terminal", RawValue::Bool(false)),
                ("state", text("processing")),
            ]),
            raw(&[
                ("event", text("tag_process")),
                ("enclave", text("e0")),
                ("logical_ns", RawValue::I64(-1)),
                ("microstep", RawValue::U64(0)),
                ("terminal", RawValue::Bool(false)),
                ("state", text("processing")),
            ]),
        ];

        assert!(matches!(
            cases[0].parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::UnknownEvent(_),
                ..
            })
        ));
        assert!(matches!(
            cases[1].parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::MissingField(_),
                ..
            })
        ));
        assert!(matches!(
            cases[2].parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::WrongType { .. },
                ..
            })
        ));
        assert!(matches!(
            cases[3].parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::NegativeUnsignedValue("logical_ns"),
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_enum_values_unknown_fields_and_variant_extras() {
        let invalid_outcome = raw(&[
            ("event", text("tag_release")),
            ("enclave", text("e0")),
            ("destination", text("e1")),
            ("logical_ns", RawValue::U64(1)),
            ("microstep", RawValue::U64(0)),
            ("outcome", text("maybe")),
        ]);
        let invalid_state = raw(&[
            ("event", text("frontier_publish")),
            ("enclave", text("e0")),
            ("state", text("paused")),
            ("outcome", text("published")),
        ]);
        let unknown = raw(&[("event", text("shutdown")), ("mystery", text("x"))]);
        let extra = raw(&[
            ("event", text("shutdown")),
            ("enclave", text("e0")),
            ("logical_ns", RawValue::U64(1)),
            ("microstep", RawValue::U64(0)),
            ("state", text("complete")),
            ("outcome", text("success")),
            ("action_key", text("a0")),
        ]);

        assert!(matches!(
            invalid_outcome.parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::InvalidValue { .. },
                ..
            })
        ));
        assert!(matches!(
            invalid_state.parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::InvalidValue { .. },
                ..
            })
        ));
        assert!(matches!(
            unknown.parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::UnknownField(_),
                ..
            })
        ));
        assert!(matches!(
            extra.parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::UnexpectedField { .. },
                ..
            })
        ));
    }

    #[test]
    fn parsing_can_be_retried_after_late_span_updates_and_inheritance_is_narrow() {
        let mut fields = raw(&[
            ("event", text("propagation_send")),
            ("enclave", text("e0")),
            ("kind", text("logical")),
            ("destination", text("e1")),
            ("action_key", text("a0")),
            ("action", text("input")),
            ("logical_ns", RawValue::U64(1)),
            ("microstep", RawValue::U64(0)),
            ("value_type", text("u32")),
            ("value_size", RawValue::U64(4)),
        ]);
        assert!(matches!(
            fields.parse(),
            Err(SchemaError {
                kind: SchemaErrorKind::MissingField(_),
                ..
            })
        ));
        fields.insert("outcome", text("accepted"));
        assert!(matches!(
            fields.parse(),
            Ok(TraceEvent::PropagationLogicalSend(_))
        ));

        let parent = raw(&[
            ("federate", text("f0")),
            ("enclave", text("e0")),
            ("reactor", text("root")),
            ("reaction_key", text("r0")),
            ("reaction", text("respond")),
            ("logical_ns", RawValue::U64(1)),
            ("microstep", RawValue::U64(0)),
            ("action_key", text("must-not-inherit")),
        ]);
        let mut child = RawTraceFields::default();
        child.inherit_missing(&parent);
        assert!(child.contains("reaction"));
        assert!(!child.contains("action_key"));
    }
}
