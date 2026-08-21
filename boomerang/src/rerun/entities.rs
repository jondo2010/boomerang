use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use super::schema::{TraceEvent, TraceRecord, TraceTag, ValueDescriptor};

/// Adapter-owned identifier for one dynamic trace record.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraceId(pub String);

impl TraceId {
    pub(crate) fn new(source: &str, enclave: &str, sequence: u64) -> Self {
        Self(format!("{source}:{enclave}:{sequence}"))
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use crate::rerun::schema::*;
    use rerun::AsComponents as _;

    fn record(event: TraceEvent) -> TraceRecord {
        TraceRecord {
            id: TraceId("source:e0:1".into()),
            parent_id: None,
            entity_path: "/event".into(),
            timepoint: TraceTimePoint {
                elapsed_ns: 1,
                wall_clock_unix_ns: 2,
                logical_ns: Some(3),
            },
            duration_ns: None,
            terminal_state: None,
            event,
        }
    }

    fn descriptors(record: &TraceRecord) -> Vec<(String, String)> {
        let mut values = record
            .dynamic_archetype()
            .as_serialized_batches()
            .into_iter()
            .map(|batch| {
                (
                    batch.descriptor.archetype.unwrap().to_string(),
                    batch.descriptor.component.to_string(),
                )
            })
            .collect::<Vec<_>>();
        values.sort();
        values
    }

    fn component_names(record: &TraceRecord) -> Vec<String> {
        let mut values = descriptors(record)
            .into_iter()
            .map(|(_, component)| match component.rsplit_once(':') {
                Some((_, name)) => name.to_owned(),
                None => component,
            })
            .collect::<Vec<_>>();
        values.sort();
        values
    }

    fn expected_components(names: &[&str]) -> Vec<String> {
        let mut values = names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        values.sort();
        values
    }

    #[test]
    fn dense_archetype_descriptor_sets_are_exact() {
        let tag = TraceTag {
            logical_ns: 3,
            microstep: 1,
        };
        let action = record(TraceEvent::ActionScheduled(ActionScheduled {
            federate: None,
            enclave: "e0".into(),
            source_tag: tag,
            action_key: "a0".into(),
            action: "tick".into(),
            destination_tag: TraceTag {
                logical_ns: 4,
                microstep: 0,
            },
            value: ValueDescriptor {
                value_type: "u64".into(),
                value_size: u64::MAX,
            },
        }));
        let mut with_parent = action.clone();
        with_parent.parent_id = Some(TraceId("parent".into()));
        assert_eq!(descriptors(&action), descriptors(&with_parent));
        let action_descriptors = descriptors(&action);
        assert!(action_descriptors
            .iter()
            .all(|(archetype, _)| archetype == "boomerang.ActionScheduled"));
        assert_eq!(
            component_names(&action),
            expected_components(&[
                "boomerang.trace.action",
                "boomerang.trace.action_key",
                "boomerang.trace.destination_logical_ns",
                "boomerang.trace.destination_microstep",
                "boomerang.trace.enclave",
                "boomerang.trace.event",
                "boomerang.trace.federate",
                "boomerang.trace.id",
                "boomerang.trace.logical_ns",
                "boomerang.trace.microstep",
                "boomerang.trace.outcome",
                "boomerang.trace.parent_id",
                "boomerang.trace.value_size",
                "boomerang.trace.value_type",
            ])
        );
        let value_size = action
            .dynamic_archetype()
            .as_serialized_batches()
            .into_iter()
            .find(|batch| {
                batch
                    .descriptor
                    .component
                    .as_str()
                    .ends_with(":boomerang.trace.value_size")
            })
            .unwrap();
        assert_eq!(
            value_size.array.data_type(),
            &rerun::external::arrow::datatypes::DataType::UInt64
        );

        let shutdown = record(TraceEvent::Shutdown(Shutdown {
            federate: None,
            enclave: "e0".into(),
            tag,
            state: ShutdownState::Complete,
            outcome: ShutdownOutcome::Success,
        }));
        assert!(descriptors(&shutdown)
            .iter()
            .all(|(archetype, _)| archetype == "boomerang.Shutdown"));
        assert_eq!(
            component_names(&shutdown),
            expected_components(&[
                "boomerang.trace.enclave",
                "boomerang.trace.event",
                "boomerang.trace.federate",
                "boomerang.trace.id",
                "boomerang.trace.logical_ns",
                "boomerang.trace.microstep",
                "boomerang.trace.outcome",
                "boomerang.trace.parent_id",
                "boomerang.trace.state",
            ])
        );
    }

    #[test]
    fn optional_tags_do_not_change_variant_descriptors() {
        let tag = TraceTag {
            logical_ns: 3,
            microstep: 1,
        };
        let rebased = |source_tag| {
            record(TraceEvent::ActionRebased(ActionRebased {
                federate: None,
                enclave: Some("e0".into()),
                source_tag,
                action_key: "a0".into(),
                old_tag: tag,
                destination_tag: tag,
            }))
        };
        assert_eq!(
            descriptors(&rebased(None)),
            descriptors(&rebased(Some(tag)))
        );

        let physical = |source_tag| {
            record(TraceEvent::PropagationPhysicalSend(
                PropagationPhysicalSend {
                    federate: None,
                    enclave: "e0".into(),
                    destination: "e1".into(),
                    source_tag,
                    action_key: "a0".into(),
                    action: "input".into(),
                    value: ValueDescriptor {
                        value_type: "u64".into(),
                        value_size: 8,
                    },
                    outcome: DeliveryOutcome::Accepted,
                },
            ))
        };
        assert_eq!(
            descriptors(&physical(None)),
            descriptors(&physical(Some(tag)))
        );
    }

    #[test]
    fn every_event_variant_has_an_exact_dense_descriptor_set() {
        let t = TraceTag {
            logical_ns: 1,
            microstep: 2,
        };
        let v = || ValueDescriptor {
            value_type: "u8".into(),
            value_size: 1,
        };
        let mut cases: Vec<(TraceEvent, &str, bool, bool, &[&str])> = Vec::new();
        macro_rules! c { ($e:expr,$a:literal,$tag:expr,$dur:expr,[$($x:literal),*]) => {
            cases.push(($e,$a,$tag,$dur,&[$($x),*]));
        }}
        c!(
            TraceEvent::SchedulerRunning(SchedulerRunning {
                federate: "f".into(),
                enclave: "e".into(),
                state: SchedulerState::Running
            }),
            "boomerang.SchedulerRunning",
            false,
            false,
            ["state"]
        );
        c!(
            TraceEvent::TagProcessing(TagProcessing {
                federate: None,
                enclave: "e".into(),
                tag: t,
                terminal: false,
                state: TagState::Processing
            }),
            "boomerang.TagProcessing",
            true,
            true,
            ["terminal", "state"]
        );
        c!(
            TraceEvent::ReactionExecution(ReactionExecution {
                federate: None,
                enclave: "e".into(),
                tag: t,
                reactor: "r".into(),
                reaction_key: None,
                reaction: "rx".into(),
                level: 0,
                state: ReactionState::Begin
            }),
            "boomerang.ReactionExecution",
            true,
            true,
            ["reactor", "reaction_key", "reaction", "level", "state"]
        );
        c!(
            TraceEvent::CoordinationWait(CoordinationWait {
                federate: None,
                enclave: "e".into(),
                tag: t,
                state: WaitState::Waiting
            }),
            "boomerang.CoordinationWait",
            true,
            true,
            ["state"]
        );
        c!(
            TraceEvent::LogicalIngress(LogicalIngress {
                federate: None,
                enclave: "e".into(),
                action_key: "a".into(),
                action: "x".into(),
                tag: t,
                destination_tag: t,
                value: v(),
                outcome: IngressOutcome::Accepted
            }),
            "boomerang.LogicalIngress",
            true,
            false,
            [
                "action_key",
                "action",
                "destination_logical_ns",
                "destination_microstep",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PhysicalIngress(PhysicalIngress {
                federate: None,
                enclave: "e".into(),
                action_key: "a".into(),
                action: "x".into(),
                tag: t,
                destination_tag: t,
                value: v(),
                outcome: IngressOutcome::Accepted
            }),
            "boomerang.PhysicalIngress",
            true,
            false,
            [
                "action_key",
                "action",
                "destination_logical_ns",
                "destination_microstep",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::ControlIngress(ControlIngress {
                federate: None,
                enclave: "e".into(),
                tag: t,
                kind: ControlKind::Shutdown,
                outcome: IngressOutcome::Accepted
            }),
            "boomerang.ControlIngress",
            true,
            false,
            ["kind", "outcome"]
        );
        c!(
            TraceEvent::ActionScheduled(ActionScheduled {
                federate: None,
                enclave: "e".into(),
                source_tag: t,
                action_key: "a".into(),
                action: "x".into(),
                destination_tag: t,
                value: v()
            }),
            "boomerang.ActionScheduled",
            true,
            false,
            [
                "action_key",
                "action",
                "destination_logical_ns",
                "destination_microstep",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::ActionStartup(ActionStartup {
                federate: None,
                enclave: "e".into(),
                source_tag: t,
                action_key: "a".into(),
                action: "x".into(),
                destination_tag: t,
                value: v()
            }),
            "boomerang.ActionStartup",
            true,
            false,
            [
                "action_key",
                "action",
                "destination_logical_ns",
                "destination_microstep",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::ActionRebased(ActionRebased {
                federate: None,
                enclave: None,
                source_tag: None,
                action_key: "a".into(),
                old_tag: t,
                destination_tag: t
            }),
            "boomerang.ActionRebased",
            true,
            false,
            [
                "action_key",
                "old_logical_ns",
                "old_microstep",
                "destination_logical_ns",
                "destination_microstep",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PortWrite(PortWrite {
                federate: None,
                enclave: "e".into(),
                reactor: "r".into(),
                reaction_key: None,
                reaction: "rx".into(),
                tag: t,
                port_key: "p".into(),
                port: "out".into(),
                value_type: "u8".into(),
                outcome: PortWriteOutcome::MutableAccess
            }),
            "boomerang.PortWrite",
            true,
            false,
            [
                "reactor",
                "reaction_key",
                "reaction",
                "port_key",
                "port",
                "value_type",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PropagationLogicalSend(PropagationLogicalSend {
                federate: None,
                enclave: "e".into(),
                destination: "d".into(),
                action_key: "a".into(),
                action: "x".into(),
                tag: t,
                value: v(),
                outcome: DeliveryOutcome::Accepted
            }),
            "boomerang.PropagationLogicalSend",
            true,
            false,
            [
                "destination",
                "action_key",
                "action",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PropagationPhysicalSend(PropagationPhysicalSend {
                federate: None,
                enclave: "e".into(),
                destination: "d".into(),
                source_tag: None,
                action_key: "a".into(),
                action: "x".into(),
                value: v(),
                outcome: DeliveryOutcome::Accepted
            }),
            "boomerang.PropagationPhysicalSend",
            true,
            false,
            [
                "destination",
                "action_key",
                "action",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PropagationSerializedSend(PropagationSerializedSend {
                federate: None,
                enclave: "e".into(),
                destination_federate: None,
                action_key: "a".into(),
                action: "x".into(),
                tag: t,
                value: v(),
                outcome: DeliveryOutcome::Accepted
            }),
            "boomerang.PropagationSerializedSend",
            true,
            false,
            [
                "destination_federate",
                "action_key",
                "action",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::PropagationReceive(PropagationReceive {
                federate: None,
                enclave: "e".into(),
                action_key: "a".into(),
                action: "x".into(),
                tag: t,
                destination_tag: t,
                value: v(),
                outcome: IngressOutcome::Accepted
            }),
            "boomerang.PropagationReceive",
            true,
            false,
            [
                "action_key",
                "action",
                "destination_logical_ns",
                "destination_microstep",
                "value_type",
                "value_size",
                "outcome"
            ]
        );
        c!(
            TraceEvent::FrontierCandidate(FrontierCandidate {
                federate: None,
                enclave: "e".into(),
                tag: t,
                outcome: PublishOutcome::Published
            }),
            "boomerang.FrontierCandidate",
            true,
            false,
            ["outcome"]
        );
        c!(
            TraceEvent::FrontierState(FrontierState {
                federate: None,
                enclave: "e".into(),
                state: FrontierStatus::Idle,
                outcome: PublishOutcome::Published
            }),
            "boomerang.FrontierState",
            false,
            false,
            ["state", "outcome"]
        );
        c!(
            TraceEvent::CoordinationGrant(CoordinationGrant {
                federate: None,
                enclave: "e".into(),
                tag: t,
                outcome: CoordinationGrantOutcome::Granted
            }),
            "boomerang.CoordinationGrant",
            true,
            false,
            ["outcome"]
        );
        c!(
            TraceEvent::TagRelease(TagRelease {
                federate: None,
                enclave: "e".into(),
                destination: "d".into(),
                tag: t,
                outcome: DeliveryOutcome::Accepted
            }),
            "boomerang.TagRelease",
            true,
            false,
            ["destination", "outcome"]
        );
        c!(
            TraceEvent::TagComplete(TagComplete {
                federate: None,
                enclave: "e".into(),
                tag: t,
                terminal: true,
                outcome: CompletionOutcome::Completed
            }),
            "boomerang.TagComplete",
            true,
            false,
            ["terminal", "outcome"]
        );
        c!(
            TraceEvent::Shutdown(Shutdown {
                federate: None,
                enclave: "e".into(),
                tag: t,
                state: ShutdownState::Complete,
                outcome: ShutdownOutcome::Success
            }),
            "boomerang.Shutdown",
            true,
            false,
            ["state", "outcome"]
        );
        c!(
            TraceEvent::RuntimeDiagnostic(RuntimeDiagnostic {
                federate: None,
                enclave: "e".into(),
                error: "x".into()
            }),
            "boomerang.RuntimeDiagnostic",
            false,
            false,
            ["error"]
        );
        c!(
            TraceEvent::SchemaDiagnostic(SchemaDiagnostic { error: "x".into() }),
            "boomerang.SchemaDiagnostic",
            false,
            false,
            ["error"]
        );
        c!(
            TraceEvent::CausalLink(CausalLink {
                enclave: "e".into(),
                source: TraceId("s".into()),
                destination: TraceId("d".into()),
                tag: t,
                state: CausalState::Exact,
                outcome: CausalOutcome::Matched
            }),
            "boomerang.CausalLink",
            true,
            false,
            ["source", "destination", "state", "outcome"]
        );

        for (event, archetype, tagged, duration, extras) in cases {
            let record = record(event);
            assert!(descriptors(&record).iter().all(|(a, _)| a == archetype));
            let mut names = vec!["id", "parent_id", "event", "federate", "enclave"];
            if tagged {
                names.extend(["logical_ns", "microstep"]);
            }
            if duration {
                names.extend(["duration_ns", "terminal_state"]);
            }
            names.extend(extras.iter().copied());
            let expected = names
                .into_iter()
                .map(|name| format!("boomerang.trace.{name}"))
                .collect::<Vec<_>>();
            assert_eq!(
                component_names(&record),
                expected_components(&expected.iter().map(String::as_str).collect::<Vec<_>>()),
                "{archetype}"
            );
        }
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

/// One adapter-normalized state transition ready for a recording sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceStateRecord {
    pub entity_path: String,
    pub timepoint: TraceTimePoint,
    pub change: TraceStateChange,
}

/// A state value to set, or a reset that ends the preceding state interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceStateChange {
    Set(String),
    Reset,
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

    fn write_state(
        &self,
        _recording: &rerun::RecordingStream,
        _record: &TraceStateRecord,
    ) -> Result<(), TraceWriterError> {
        Ok(())
    }
}

pub(crate) struct RerunTraceWriter;

struct TimeContextReset<'a>(&'a rerun::RecordingStream);

impl Drop for TimeContextReset<'_> {
    fn drop(&mut self) {
        self.0.reset_time();
    }
}

fn with_timepoint(
    recording: &rerun::RecordingStream,
    timepoint: &TraceTimePoint,
    write: impl FnOnce() -> Result<(), TraceWriterError>,
) -> Result<(), TraceWriterError> {
    let mut rerun_timepoint = rerun::TimePoint::default();
    rerun_timepoint.insert_cell(
        "elapsed",
        rerun::TimeCell::from_duration_nanos(timepoint.elapsed_ns),
    );
    rerun_timepoint.insert_cell(
        "wall_clock",
        rerun::TimeCell::from_timestamp_nanos_since_epoch(timepoint.wall_clock_unix_ns),
    );
    if let Some(logical_ns) = timepoint.logical_ns {
        rerun_timepoint.insert_cell("logical", rerun::TimeCell::from_duration_nanos(logical_ns));
    }

    // Rerun's time context is thread-local. Reset before and after each write so a record
    // without logical time cannot inherit it from an earlier record on the same thread. The
    // guard also resets the context during panic unwinding.
    recording.reset_time();
    let _reset = TimeContextReset(recording);
    recording.set_timepoint(rerun_timepoint);
    write()
}

impl TraceWriter for RerunTraceWriter {
    fn write(
        &self,
        recording: &rerun::RecordingStream,
        record: &TraceRecord,
    ) -> Result<(), TraceWriterError> {
        with_timepoint(recording, &record.timepoint, || {
            recording.log(record.entity_path.clone(), &record.dynamic_archetype())?;
            if let TraceEvent::RuntimeDiagnostic(value) = &record.event {
                recording.log(
                    record.entity_path.clone(),
                    &rerun::TextLog::new(value.error.as_str())
                        .with_level(rerun::TextLogLevel::ERROR),
                )?;
            } else if let TraceEvent::SchemaDiagnostic(value) = &record.event {
                recording.log(
                    record.entity_path.clone(),
                    &rerun::TextLog::new(value.error.as_str())
                        .with_level(rerun::TextLogLevel::ERROR),
                )?;
            } else if let TraceEvent::CausalLink(value) = &record.event {
                recording.log(
                    record.entity_path.clone(),
                    &rerun::GraphEdges::new([(
                        value.source.0.as_str(),
                        value.destination.0.as_str(),
                    )])
                    .with_graph_type(rerun::components::GraphType::Directed),
                )?;
            }
            for (name, value) in record.scalar_series() {
                recording.log(
                    format!("{}/metrics/{name}", record.entity_path),
                    &rerun::Scalars::new([value]),
                )?;
            }
            Ok(())
        })
    }

    fn write_state(
        &self,
        recording: &rerun::RecordingStream,
        record: &TraceStateRecord,
    ) -> Result<(), TraceWriterError> {
        with_timepoint(recording, &record.timepoint, || {
            match &record.change {
                TraceStateChange::Set(value) => recording.log(
                    record.entity_path.clone(),
                    &rerun::StateChange::single(value.clone()),
                )?,
                TraceStateChange::Reset => recording.log(
                    record.entity_path.clone(),
                    &rerun::StateChange::clear_fields(),
                )?,
            }
            Ok(())
        })
    }
}

impl TraceRecord {
    fn dynamic_archetype(&self) -> rerun::DynamicArchetype {
        dense_archetype(self)
    }

    fn scalar_series(&self) -> Vec<(&'static str, f64)> {
        let mut values = Vec::new();
        if let Some(duration_ns) = self.duration_ns {
            values.push(("duration_ns", duration_ns as f64));
        }
        if let Some(value_size) = event_value(&self.event).map(|value| value.value_size) {
            values.push(("value_size", value_size as f64));
        }
        if let Some(terminal) = self.event.terminal() {
            values.push(("terminal", if terminal { 1.0 } else { 0.0 }));
        }
        values
    }
}

fn text(
    mut payload: rerun::DynamicArchetype,
    name: &'static str,
    value: Option<&str>,
) -> rerun::DynamicArchetype {
    payload = payload.with_component_from_data(
        name,
        std::sync::Arc::new(rerun::external::arrow::array::StringArray::from(vec![
            value,
        ])),
    );
    payload
}

fn uint(
    mut payload: rerun::DynamicArchetype,
    name: &'static str,
    value: Option<u64>,
) -> rerun::DynamicArchetype {
    payload = payload.with_component_from_data(
        name,
        std::sync::Arc::new(rerun::external::arrow::array::UInt64Array::from(vec![
            value,
        ])),
    );
    payload
}

fn boolean(
    mut payload: rerun::DynamicArchetype,
    name: &'static str,
    value: Option<bool>,
) -> rerun::DynamicArchetype {
    payload = payload.with_component_from_data(
        name,
        std::sync::Arc::new(rerun::external::arrow::array::BooleanArray::from(vec![
            value,
        ])),
    );
    payload
}

fn tag(
    mut payload: rerun::DynamicArchetype,
    prefix: &'static str,
    value: Option<TraceTag>,
) -> rerun::DynamicArchetype {
    let (logical, microstep) = value.map_or((None, None), |tag| {
        (Some(tag.logical_ns), Some(tag.microstep))
    });
    let (logical_name, microstep_name) = match prefix {
        "destination" => (
            "boomerang.trace.destination_logical_ns",
            "boomerang.trace.destination_microstep",
        ),
        "old" => (
            "boomerang.trace.old_logical_ns",
            "boomerang.trace.old_microstep",
        ),
        _ => ("boomerang.trace.logical_ns", "boomerang.trace.microstep"),
    };
    payload = uint(payload, logical_name, logical);
    uint(payload, microstep_name, microstep)
}

fn value(
    mut payload: rerun::DynamicArchetype,
    descriptor: &ValueDescriptor,
) -> rerun::DynamicArchetype {
    payload = text(
        payload,
        "boomerang.trace.value_type",
        Some(&descriptor.value_type),
    );
    uint(
        payload,
        "boomerang.trace.value_size",
        Some(descriptor.value_size),
    )
}

fn common(record: &TraceRecord, archetype: &'static str) -> rerun::DynamicArchetype {
    let mut payload = rerun::DynamicArchetype::new(archetype);
    payload = text(payload, "boomerang.trace.id", Some(&record.id.0));
    payload = text(
        payload,
        "boomerang.trace.parent_id",
        record.parent_id.as_ref().map(|id| id.0.as_str()),
    );
    payload = text(payload, "boomerang.trace.event", Some(record.event.name()));
    payload = text(payload, "boomerang.trace.federate", record.event.federate());
    payload = text(payload, "boomerang.trace.enclave", record.event.enclave());
    if !matches!(
        &record.event,
        TraceEvent::SchedulerRunning(_)
            | TraceEvent::FrontierState(_)
            | TraceEvent::RuntimeDiagnostic(_)
            | TraceEvent::SchemaDiagnostic(_)
    ) {
        payload = tag(payload, "", record.event.tag());
    }
    if record.event.duration_phase().is_some() {
        payload = uint(payload, "boomerang.trace.duration_ns", record.duration_ns);
        payload = text(
            payload,
            "boomerang.trace.terminal_state",
            record.terminal_state.as_deref(),
        );
    }
    payload
}

fn action(
    mut p: rerun::DynamicArchetype,
    key: &str,
    name: &str,
    destination: TraceTag,
    descriptor: &ValueDescriptor,
    outcome: &str,
) -> rerun::DynamicArchetype {
    p = text(p, "boomerang.trace.action_key", Some(key));
    p = text(p, "boomerang.trace.action", Some(name));
    p = tag(p, "destination", Some(destination));
    p = value(p, descriptor);
    text(p, "boomerang.trace.outcome", Some(outcome))
}

fn ingress(
    mut p: rerun::DynamicArchetype,
    key: &str,
    name: &str,
    destination: TraceTag,
    descriptor: &ValueDescriptor,
    outcome: &str,
) -> rerun::DynamicArchetype {
    p = text(p, "boomerang.trace.action_key", Some(key));
    p = text(p, "boomerang.trace.action", Some(name));
    p = tag(p, "destination", Some(destination));
    p = value(p, descriptor);
    text(p, "boomerang.trace.outcome", Some(outcome))
}

fn send(
    mut p: rerun::DynamicArchetype,
    key: &str,
    name: &str,
    descriptor: &ValueDescriptor,
    outcome: &str,
) -> rerun::DynamicArchetype {
    p = text(p, "boomerang.trace.action_key", Some(key));
    p = text(p, "boomerang.trace.action", Some(name));
    p = value(p, descriptor);
    text(p, "boomerang.trace.outcome", Some(outcome))
}

fn dense_archetype(record: &TraceRecord) -> rerun::DynamicArchetype {
    match &record.event {
        TraceEvent::SchedulerRunning(v) => text(
            common(record, "boomerang.SchedulerRunning"),
            "boomerang.trace.state",
            Some(v.state.as_str()),
        ),
        TraceEvent::TagProcessing(v) => {
            let p = boolean(
                common(record, "boomerang.TagProcessing"),
                "boomerang.trace.terminal",
                Some(v.terminal),
            );
            text(p, "boomerang.trace.state", Some(v.state.as_str()))
        }
        TraceEvent::ReactionExecution(v) => {
            let mut p = common(record, "boomerang.ReactionExecution");
            p = text(p, "boomerang.trace.reactor", Some(&v.reactor));
            p = text(p, "boomerang.trace.reaction_key", v.reaction_key.as_deref());
            p = text(p, "boomerang.trace.reaction", Some(&v.reaction));
            p = uint(p, "boomerang.trace.level", Some(v.level));
            text(p, "boomerang.trace.state", Some(v.state.as_str()))
        }
        TraceEvent::CoordinationWait(v) => text(
            common(record, "boomerang.CoordinationWait"),
            "boomerang.trace.state",
            Some(v.state.as_str()),
        ),
        TraceEvent::LogicalIngress(v) => ingress(
            common(record, "boomerang.LogicalIngress"),
            &v.action_key,
            &v.action,
            v.destination_tag,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::PhysicalIngress(v) => ingress(
            common(record, "boomerang.PhysicalIngress"),
            &v.action_key,
            &v.action,
            v.destination_tag,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::ControlIngress(v) => {
            let p = text(
                common(record, "boomerang.ControlIngress"),
                "boomerang.trace.kind",
                Some(v.kind.as_str()),
            );
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::ActionScheduled(v) => action(
            common(record, "boomerang.ActionScheduled"),
            &v.action_key,
            &v.action,
            v.destination_tag,
            &v.value,
            "scheduled",
        ),
        TraceEvent::ActionStartup(v) => action(
            common(record, "boomerang.ActionStartup"),
            &v.action_key,
            &v.action,
            v.destination_tag,
            &v.value,
            "startup",
        ),
        TraceEvent::ActionRebased(v) => {
            let mut p = text(
                common(record, "boomerang.ActionRebased"),
                "boomerang.trace.action_key",
                Some(&v.action_key),
            );
            p = tag(p, "old", Some(v.old_tag));
            p = tag(p, "destination", Some(v.destination_tag));
            text(p, "boomerang.trace.outcome", Some("rebased"))
        }
        TraceEvent::PortWrite(v) => {
            let mut p = common(record, "boomerang.PortWrite");
            p = text(p, "boomerang.trace.reactor", Some(&v.reactor));
            p = text(p, "boomerang.trace.reaction_key", v.reaction_key.as_deref());
            p = text(p, "boomerang.trace.reaction", Some(&v.reaction));
            p = text(p, "boomerang.trace.port_key", Some(&v.port_key));
            p = text(p, "boomerang.trace.port", Some(&v.port));
            p = text(p, "boomerang.trace.value_type", Some(&v.value_type));
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::PropagationLogicalSend(v) => send(
            text(
                common(record, "boomerang.PropagationLogicalSend"),
                "boomerang.trace.destination",
                Some(&v.destination),
            ),
            &v.action_key,
            &v.action,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::PropagationPhysicalSend(v) => send(
            text(
                common(record, "boomerang.PropagationPhysicalSend"),
                "boomerang.trace.destination",
                Some(&v.destination),
            ),
            &v.action_key,
            &v.action,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::PropagationSerializedSend(v) => send(
            text(
                common(record, "boomerang.PropagationSerializedSend"),
                "boomerang.trace.destination_federate",
                v.destination_federate.as_deref(),
            ),
            &v.action_key,
            &v.action,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::PropagationReceive(v) => ingress(
            common(record, "boomerang.PropagationReceive"),
            &v.action_key,
            &v.action,
            v.destination_tag,
            &v.value,
            v.outcome.as_str(),
        ),
        TraceEvent::FrontierCandidate(v) => text(
            common(record, "boomerang.FrontierCandidate"),
            "boomerang.trace.outcome",
            Some(v.outcome.as_str()),
        ),
        TraceEvent::FrontierState(v) => {
            let p = text(
                common(record, "boomerang.FrontierState"),
                "boomerang.trace.state",
                Some(v.state.as_str()),
            );
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::CoordinationGrant(v) => text(
            common(record, "boomerang.CoordinationGrant"),
            "boomerang.trace.outcome",
            Some(v.outcome.as_str()),
        ),
        TraceEvent::TagRelease(v) => {
            let p = text(
                common(record, "boomerang.TagRelease"),
                "boomerang.trace.destination",
                Some(&v.destination),
            );
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::TagComplete(v) => {
            let p = boolean(
                common(record, "boomerang.TagComplete"),
                "boomerang.trace.terminal",
                Some(v.terminal),
            );
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::Shutdown(v) => {
            let p = text(
                common(record, "boomerang.Shutdown"),
                "boomerang.trace.state",
                Some(v.state.as_str()),
            );
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
        TraceEvent::RuntimeDiagnostic(v) => text(
            common(record, "boomerang.RuntimeDiagnostic"),
            "boomerang.trace.error",
            Some(&v.error),
        ),
        TraceEvent::SchemaDiagnostic(v) => text(
            common(record, "boomerang.SchemaDiagnostic"),
            "boomerang.trace.error",
            Some(&v.error),
        ),
        TraceEvent::CausalLink(v) => {
            let mut p = text(
                common(record, "boomerang.CausalLink"),
                "boomerang.trace.source",
                Some(&v.source.0),
            );
            p = text(p, "boomerang.trace.destination", Some(&v.destination.0));
            p = text(p, "boomerang.trace.state", Some(v.state.as_str()));
            text(p, "boomerang.trace.outcome", Some(v.outcome.as_str()))
        }
    }
}

fn event_value(event: &TraceEvent) -> Option<&ValueDescriptor> {
    match event {
        TraceEvent::LogicalIngress(v) => Some(&v.value),
        TraceEvent::PhysicalIngress(v) => Some(&v.value),
        TraceEvent::ActionScheduled(v) => Some(&v.value),
        TraceEvent::ActionStartup(v) => Some(&v.value),
        TraceEvent::PropagationLogicalSend(v) => Some(&v.value),
        TraceEvent::PropagationPhysicalSend(v) => Some(&v.value),
        TraceEvent::PropagationSerializedSend(v) => Some(&v.value),
        TraceEvent::PropagationReceive(v) => Some(&v.value),
        _ => None,
    }
}

pub(crate) fn escape_entity_segment(segment: &str) -> String {
    segment.replace('\\', "\\\\").replace('/', "\\/")
}

/// Compact adapter-owned lookup derived synchronously from static registration.
#[derive(Clone, Debug, Default)]
pub(super) struct RegistrationIndex {
    entities: HashMap<RegistrationLookup, RegistrationResolution>,
    federated_entities: HashMap<FederatedRegistrationLookup, RegistrationResolution>,
    action_triggers: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RegistrationLookup {
    federate: Option<String>,
    enclave: String,
    kind: &'static str,
    identity: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FederatedRegistrationLookup {
    federate: String,
    kind: &'static str,
    identity: String,
}

#[derive(Clone, Debug)]
enum RegistrationResolution {
    Unique(String),
    Ambiguous,
}

impl RegistrationIndex {
    #[cfg(test)]
    pub(super) fn register(
        &mut self,
        enclave: &str,
        kind: &'static str,
        stable_key: &str,
        display_name: &str,
        path: &str,
    ) {
        self.register_in_federate(None, enclave, kind, stable_key, display_name, path);
    }

    pub(super) fn register_in_federate(
        &mut self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        stable_key: &str,
        display_name: &str,
        path: &str,
    ) {
        self.register_identity(federate, enclave, kind, stable_key, path);
        if display_name != stable_key {
            self.register_identity(federate, enclave, kind, display_name, path);
        }
    }

    pub(super) fn register_action_trigger(&mut self, action_path: &str, reaction_path: &str) {
        let reactions = self
            .action_triggers
            .entry(action_path.to_owned())
            .or_default();
        if !reactions.iter().any(|reaction| reaction == reaction_path) {
            reactions.push(reaction_path.to_owned());
        }
    }

    fn register_identity(
        &mut self,
        federate: Option<&str>,
        enclave: &str,
        kind: &'static str,
        identity: &str,
        path: &str,
    ) {
        let lookup = RegistrationLookup {
            federate: federate.map(str::to_owned),
            enclave: enclave.to_owned(),
            kind,
            identity: identity.to_owned(),
        };
        register_resolution(&mut self.entities, lookup, path);
        if let Some(federate) = federate {
            register_resolution(
                &mut self.federated_entities,
                FederatedRegistrationLookup {
                    federate: federate.to_owned(),
                    kind,
                    identity: identity.to_owned(),
                },
                path,
            );
        }
    }

    pub(super) fn entity_path(&self, event: &TraceEvent) -> Option<String> {
        self.resolve_entity(event)
            .map(|path| format!("{}/{}", path, escape_entity_segment(event.name())))
    }

    pub(super) fn resolve_entity(&self, event: &TraceEvent) -> Option<String> {
        let (kind, identity) = event_identity(event);
        if matches!(
            event,
            TraceEvent::PropagationLogicalSend(_)
                | TraceEvent::PropagationPhysicalSend(_)
                | TraceEvent::PropagationSerializedSend(_)
        ) {
            let (federate, destination) = propagation_destination(event);
            if let Some(enclave) = destination {
                return resolve_registration(
                    &self.entities,
                    &RegistrationLookup {
                        federate: federate.map(str::to_owned),
                        enclave: enclave.to_owned(),
                        kind,
                        identity: identity.to_owned(),
                    },
                );
            }
            return resolve_registration(
                &self.federated_entities,
                &FederatedRegistrationLookup {
                    federate: federate?.to_owned(),
                    kind,
                    identity: identity.to_owned(),
                },
            );
        }
        let enclave = event.enclave()?;
        let lookup = RegistrationLookup {
            federate: event.federate().map(str::to_owned),
            enclave: enclave.to_owned(),
            kind,
            identity: identity.to_owned(),
        };
        resolve_registration(&self.entities, &lookup)
    }

    pub(super) fn triggered_reactions(&self, action_path: &str) -> &[String] {
        self.action_triggers
            .get(action_path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn merge(&mut self, other: Self) {
        for (lookup, incoming) in other.entities {
            merge_resolution(&mut self.entities, lookup, incoming);
        }
        for (lookup, incoming) in other.federated_entities {
            merge_resolution(&mut self.federated_entities, lookup, incoming);
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

fn event_identity(event: &TraceEvent) -> (&'static str, &str) {
    match event {
        TraceEvent::LogicalIngress(v) => ("action", &v.action_key),
        TraceEvent::PhysicalIngress(v) => ("action", &v.action_key),
        TraceEvent::ActionScheduled(v) => ("action", &v.action_key),
        TraceEvent::ActionStartup(v) => ("action", &v.action_key),
        TraceEvent::ActionRebased(v) => ("action", &v.action_key),
        TraceEvent::PropagationLogicalSend(v) => ("action", &v.action_key),
        TraceEvent::PropagationPhysicalSend(v) => ("action", &v.action_key),
        TraceEvent::PropagationSerializedSend(v) => ("action", &v.action_key),
        TraceEvent::PropagationReceive(v) => ("action", &v.action_key),
        TraceEvent::PortWrite(v) => ("port", &v.port_key),
        TraceEvent::ReactionExecution(v) => {
            ("reaction", v.reaction_key.as_deref().unwrap_or(&v.reaction))
        }
        _ => ("scheduler", "scheduler"),
    }
}

fn propagation_destination(event: &TraceEvent) -> (Option<&str>, Option<&str>) {
    match event {
        TraceEvent::PropagationLogicalSend(v) => (v.federate.as_deref(), Some(&v.destination)),
        TraceEvent::PropagationPhysicalSend(v) => (v.federate.as_deref(), Some(&v.destination)),
        TraceEvent::PropagationSerializedSend(v) => (
            v.destination_federate.as_deref().or(v.federate.as_deref()),
            None,
        ),
        _ => (event.federate(), None),
    }
}

fn register_resolution<K: Eq + std::hash::Hash>(
    map: &mut HashMap<K, RegistrationResolution>,
    key: K,
    path: &str,
) {
    merge_resolution(map, key, RegistrationResolution::Unique(path.to_owned()));
}

fn merge_resolution<K: Eq + std::hash::Hash>(
    map: &mut HashMap<K, RegistrationResolution>,
    key: K,
    incoming: RegistrationResolution,
) {
    map.entry(key)
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

fn resolve_registration<K: Eq + std::hash::Hash>(
    map: &HashMap<K, RegistrationResolution>,
    key: &K,
) -> Option<String> {
    match map.get(key) {
        Some(RegistrationResolution::Unique(path)) => Some(path.clone()),
        Some(RegistrationResolution::Ambiguous) | None => None,
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
        index.register_in_federate(
            federate,
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
            index.register_in_federate(
                federate,
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
            index.register_in_federate(
                federate,
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
            index.register_in_federate(
                federate,
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
            for (_, reaction) in reactions {
                let reaction = reaction_path(*reaction);
                index.register_action_trigger(&action_path, &reaction);
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

pub(crate) fn entity_path(event: &TraceEvent) -> String {
    if matches!(
        event,
        TraceEvent::PropagationLogicalSend(_)
            | TraceEvent::PropagationPhysicalSend(_)
            | TraceEvent::PropagationSerializedSend(_)
            | TraceEvent::PropagationReceive(_)
    ) {
        return format!(
            "/propagation/unresolved/{}",
            escape_entity_segment(event.name())
        );
    }
    let enclave = escape_entity_segment(event.enclave().unwrap_or("unknown"));
    let root = event.federate().map_or_else(
        || format!("/enclaves/{enclave}"),
        |federate| {
            format!(
                "/federates/{}/enclaves/{enclave}",
                escape_entity_segment(federate)
            )
        },
    );
    let (kind, identity) = event_identity(event);
    if kind == "action" {
        return format!(
            "{root}/actions/{}/{}",
            escape_entity_segment(identity),
            escape_entity_segment(event.name()),
        );
    }
    if kind == "port" {
        return format!(
            "{root}/ports/{}/{}",
            escape_entity_segment(identity),
            escape_entity_segment(event.name()),
        );
    }
    if let TraceEvent::ReactionExecution(value) = event {
        return format!(
            "{root}/reactors/{}/reactions/{}",
            escape_entity_segment(&value.reactor),
            escape_entity_segment(identity),
        );
    }
    format!("{root}/scheduler/{}", escape_entity_segment(event.name()))
}

#[cfg(test)]
mod tests {
    use boomerang_runtime::{Enclave, Reactor};
    use rerun::AsComponents as _;

    use super::*;
    use crate::rerun::schema::{
        ActionScheduled, CausalLink, CausalOutcome, CausalState, CompletionOutcome,
        DeliveryOutcome, IngressOutcome, LogicalIngress, PropagationLogicalSend,
        PropagationReceive, ReactionExecution, ReactionState, RuntimeDiagnostic, SchemaDiagnostic,
        Shutdown, ShutdownOutcome, ShutdownState, TagComplete, ValueDescriptor,
    };

    const TAG: TraceTag = TraceTag {
        logical_ns: 3,
        microstep: 1,
    };

    fn value() -> ValueDescriptor {
        ValueDescriptor {
            value_type: "u64".to_owned(),
            value_size: u64::MAX,
        }
    }

    fn ingress(federate: Option<&str>, enclave: &str, action_key: &str) -> TraceEvent {
        TraceEvent::LogicalIngress(LogicalIngress {
            federate: federate.map(str::to_owned),
            enclave: enclave.to_owned(),
            action_key: action_key.to_owned(),
            action: "input".to_owned(),
            tag: TAG,
            destination_tag: TAG,
            value: value(),
            outcome: IngressOutcome::Accepted,
        })
    }

    fn logical_send(
        federate: Option<&str>,
        enclave: &str,
        destination: &str,
        action_key: &str,
    ) -> TraceEvent {
        TraceEvent::PropagationLogicalSend(PropagationLogicalSend {
            federate: federate.map(str::to_owned),
            enclave: enclave.to_owned(),
            destination: destination.to_owned(),
            action_key: action_key.to_owned(),
            action: "input".to_owned(),
            tag: TAG,
            value: value(),
            outcome: DeliveryOutcome::Accepted,
        })
    }

    fn record(event: TraceEvent) -> TraceRecord {
        TraceRecord {
            id: TraceId("source:e0:1".to_owned()),
            parent_id: None,
            entity_path: "/diagnostics/schema".to_owned(),
            timepoint: TraceTimePoint {
                elapsed_ns: 1,
                wall_clock_unix_ns: 2,
                logical_ns: Some(3),
            },
            duration_ns: None,
            terminal_state: None,
            event,
        }
    }

    fn action_scheduled() -> TraceEvent {
        TraceEvent::ActionScheduled(ActionScheduled {
            federate: None,
            enclave: "e0".to_owned(),
            source_tag: TAG,
            action_key: "a0".to_owned(),
            action: "tick".to_owned(),
            destination_tag: TraceTag {
                logical_ns: 4,
                microstep: 0,
            },
            value: value(),
        })
    }

    #[test]
    fn federate_qualifies_overlapping_runtime_entity_keys() {
        let mut index = RegistrationIndex::default();
        index.register_in_federate(
            Some("a"),
            "0",
            "action",
            "0",
            "input",
            "/federates/a/enclaves/0/actions/input",
        );
        index.register_in_federate(
            Some("b"),
            "0",
            "action",
            "0",
            "input",
            "/federates/b/enclaves/0/actions/input",
        );

        let ingress = ingress(Some("b"), "0", "0");
        assert_eq!(
            index.resolve_entity(&ingress).as_deref(),
            Some("/federates/b/enclaves/0/actions/input")
        );

        let send = TraceEvent::PropagationSerializedSend(
            crate::rerun::schema::PropagationSerializedSend {
                federate: None,
                enclave: "source".to_owned(),
                destination_federate: Some("b".to_owned()),
                action_key: "0".to_owned(),
                action: "input".to_owned(),
                tag: TAG,
                value: value(),
                outcome: DeliveryOutcome::Accepted,
            },
        );
        assert_eq!(
            index.resolve_entity(&send).as_deref(),
            Some("/federates/b/enclaves/0/actions/input")
        );

        index.register_in_federate(
            Some("b"),
            "1",
            "action",
            "0",
            "input",
            "/federates/b/enclaves/1/actions/input",
        );
        assert_eq!(index.resolve_entity(&send), None);
    }

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
        let send = logical_send(None, "EnclaveKey(1)", "EnclaveKey(0)", "ActionKey(0)");
        let receive = TraceEvent::PropagationReceive(PropagationReceive {
            federate: None,
            enclave: "EnclaveKey(1)".to_owned(),
            action_key: "ActionKey(0)".to_owned(),
            action: "input".to_owned(),
            tag: TAG,
            destination_tag: TAG,
            value: value(),
            outcome: IngressOutcome::Accepted,
        });

        assert_eq!(
            entity_path(&send),
            "/propagation/unresolved/propagation_send"
        );
        assert_eq!(
            entity_path(&receive),
            "/propagation/unresolved/propagation_receive"
        );
    }

    #[test]
    fn operational_payload_is_dense_and_preserves_typed_numeric_components() {
        let payload = record(action_scheduled()).dynamic_archetype();
        let batches = payload.as_serialized_batches();
        assert!(batches.iter().all(|batch| {
            batch
                .descriptor
                .archetype
                .as_ref()
                .is_some_and(|name| name.as_str() == "boomerang.ActionScheduled")
        }));
        assert!(batches.iter().all(|batch| {
            let component = batch.descriptor.component.as_str();
            !component.contains("port") && !component.contains("reaction")
        }));
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

        let reaction = record(TraceEvent::ReactionExecution(ReactionExecution {
            federate: None,
            enclave: "e0".to_owned(),
            tag: TAG,
            reactor: "main".to_owned(),
            reaction_key: Some("r0".to_owned()),
            reaction: "react".to_owned(),
            level: 7,
            state: ReactionState::Begin,
        }))
        .dynamic_archetype();
        let level = reaction
            .as_serialized_batches()
            .into_iter()
            .find(|batch| {
                batch
                    .descriptor
                    .component
                    .as_str()
                    .ends_with(":boomerang.trace.level")
            })
            .expect("reaction level component");
        assert_eq!(
            level.array.data_type(),
            &rerun::external::arrow::datatypes::DataType::UInt64
        );
    }

    #[test]
    fn duration_is_exposed_as_builtin_scalar_series() {
        let mut reaction = record(TraceEvent::ReactionExecution(ReactionExecution {
            federate: None,
            enclave: "e0".to_owned(),
            tag: TAG,
            reactor: "main".to_owned(),
            reaction_key: Some("r0".to_owned()),
            reaction: "react".to_owned(),
            level: 0,
            state: ReactionState::Begin,
        }));
        reaction.duration_ns = Some(5);
        let series = reaction.scalar_series();
        assert!(series.iter().any(|(name, _)| *name == "duration_ns"));
    }

    #[test]
    fn registration_merge_is_idempotent_and_conflicts_become_ambiguous() {
        let event = ingress(None, "e0", "a0");
        let mut registration = RegistrationIndex::default();
        registration.register_in_federate(None, "e0", "action", "a0", "input", "/first/action");
        let mut repeated = RegistrationIndex::default();
        repeated.register_in_federate(None, "e0", "action", "a0", "input", "/first/action");
        registration.merge(repeated);
        assert_eq!(
            registration.resolve_entity(&event),
            Some("/first/action".to_owned())
        );

        let mut conflicting = RegistrationIndex::default();
        conflicting.register_in_federate(None, "e0", "action", "a0", "input", "/second/action");
        registration.merge(conflicting);
        assert_eq!(registration.resolve_entity(&event), None);
    }

    #[test]
    fn memory_sink_encodes_timelines_typed_components_and_builtin_archetypes() {
        let (recording, memory) = rerun::RecordingStreamBuilder::new("boomerang-memory-behavior")
            .memory()
            .unwrap();
        let writer = RerunTraceWriter;

        let mut logical = record(action_scheduled());
        logical.entity_path = "/records/logical".to_owned();
        writer.write(&recording, &logical).unwrap();

        let mut terminal = record(TraceEvent::TagComplete(TagComplete {
            federate: None,
            enclave: "e0".to_owned(),
            tag: TAG,
            terminal: true,
            outcome: CompletionOutcome::Completed,
        }));
        terminal.entity_path = "/records/terminal".to_owned();
        writer.write(&recording, &terminal).unwrap();

        let mut non_logical = record(TraceEvent::Shutdown(Shutdown {
            federate: None,
            enclave: "e0".to_owned(),
            tag: TAG,
            state: ShutdownState::Complete,
            outcome: ShutdownOutcome::Success,
        }));
        non_logical.entity_path = "/records/non_logical".to_owned();
        non_logical.timepoint.logical_ns = None;
        writer.write(&recording, &non_logical).unwrap();

        let diagnostic = record(TraceEvent::SchemaDiagnostic(SchemaDiagnostic {
            error: "bad schema".to_owned(),
        }));
        writer.write(&recording, &diagnostic).unwrap();

        let mut runtime_diagnostic = record(TraceEvent::RuntimeDiagnostic(RuntimeDiagnostic {
            federate: None,
            enclave: "e0".to_owned(),
            error: "runtime failure".to_owned(),
        }));
        runtime_diagnostic.entity_path = "/diagnostics/runtime".to_owned();
        writer.write(&recording, &runtime_diagnostic).unwrap();

        let mut reaction = record(TraceEvent::ReactionExecution(ReactionExecution {
            federate: None,
            enclave: "e0".to_owned(),
            tag: TAG,
            reactor: "main".to_owned(),
            reaction_key: Some("r0".to_owned()),
            reaction: "react".to_owned(),
            level: 0,
            state: ReactionState::Begin,
        }));
        reaction.entity_path = "/records/reaction".to_owned();
        reaction.duration_ns = Some(5);
        writer.write(&recording, &reaction).unwrap();

        let mut causal = record(TraceEvent::CausalLink(CausalLink {
            enclave: "e0".to_owned(),
            source: TraceId("source".to_owned()),
            destination: TraceId("destination".to_owned()),
            tag: TAG,
            state: CausalState::Exact,
            outcome: CausalOutcome::Matched,
        }));
        causal.entity_path = "/causality/source_to_destination".to_owned();
        writer.write(&recording, &causal).unwrap();

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
        let terminal_chunk = chunks
            .iter()
            .find(|chunk| chunk.entity_path().to_string() == "/records/terminal")
            .expect("terminal record chunk");
        let terminal_component = terminal_chunk
            .components()
            .0
            .values()
            .find(|column| {
                column
                    .descriptor
                    .component
                    .as_str()
                    .ends_with(":boomerang.trace.terminal")
            })
            .expect("terminal component");
        assert_eq!(
            terminal_component.list_array.values().data_type(),
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
        assert!(chunks
            .iter()
            .any(
                |chunk| chunk.component_descriptors().any(|descriptor| descriptor
                    .archetype
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "rerun.archetypes.GraphEdges"))
            ));
    }
}
