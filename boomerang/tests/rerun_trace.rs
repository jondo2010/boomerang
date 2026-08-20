#![cfg(feature = "rerun")]

use std::sync::{Arc, Barrier, Mutex};

use boomerang::rerun::{
    RerunSessionBuilder, SinkConfig, TraceRecord, TraceWriter, TraceWriterError,
};
use tracing_subscriber::prelude::*;

#[derive(Default)]
struct CapturingWriter {
    records: Mutex<Vec<TraceRecord>>,
}

impl TraceWriter for CapturingWriter {
    fn write(
        &self,
        _recording: &rerun::RecordingStream,
        record: &TraceRecord,
    ) -> Result<(), TraceWriterError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
}

impl CapturingWriter {
    fn records(&self) -> Vec<TraceRecord> {
        self.records.lock().unwrap().clone()
    }
}

fn session_with_capture(source_id: &str) -> (boomerang::rerun::RerunSession, Arc<CapturingWriter>) {
    let capture = Arc::new(CapturingWriter::default());
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .source_id(source_id)
        .trace_writer(capture.clone())
        .build()
        .unwrap();
    (session, capture)
}

#[test]
fn memory_session_starts_enabled_with_empty_failure_counts() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .source_id("test-process")
        .sink(SinkConfig::Memory)
        .build()
        .unwrap();

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.skipped_count(), 0);
    assert_eq!(session.source_id(), "test-process");
}

#[test]
fn default_source_ids_are_generated_per_session() {
    let first = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let second = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();

    assert_ne!(first.source_id(), second.source_id());
}

#[test]
fn memory_session_flush_is_idempotent_smoke_test() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();

    session.flush();
    session.flush();

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.skipped_count(), 0);
}

#[test]
fn layer_composes_and_maps_reaction_span_with_explicit_timepoint() {
    let (session, capture) = session_with_capture("source/a");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            enclave = "e/0",
            logical_ns = 42_u64,
            microstep = 3_u64,
            reactor = "r/0",
            reaction = "react\\0",
            state = "begin",
        );
        let _entered = span.enter();
    });

    let records = capture.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.event, "reaction_execute");
    assert_eq!(
        record.entity_path,
        "/enclaves/e\\/0/reactors/r\\/0/reactions/react\\\\0"
    );
    assert_eq!(record.timepoint.logical_ns, Some(42));
    assert_eq!(record.microstep, Some(3));
    assert!(record.timepoint.elapsed_ns >= 0);
    assert!(record.timepoint.wall_clock_unix_ns > 0);
    assert!(record.duration_ns.is_some());
}

#[test]
fn span_records_updates_and_accounts_for_cross_thread_entries() {
    let (session, capture) = session_with_capture("cross-thread");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::trace_span!(
            target: "boomerang::trace",
            "coordination_wait",
            event = "coordination_wait",
            enclave = "e0",
            state = tracing::field::Empty,
        );
        span.record("state", "waiting");
        std::thread::scope(|scope| {
            let span = span.clone();
            scope.spawn(move || {
                let _entered = span.enter();
                std::thread::yield_now();
            });
        });
    });

    let records = capture.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].fields.state.as_deref(), Some("waiting"));
    assert!(records[0].duration_ns.is_some());
}

#[test]
fn default_memory_writer_accepts_trace_record_smoke_test() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            state = "complete",
            outcome = "success",
        );
    });
    session.flush();

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
}

#[test]
fn child_event_uses_adapter_parent_id_and_neutral_ingress_has_none() {
    let (session, capture) = session_with_capture("parentage");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let reaction = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            enclave = "e0",
            reactor = "r0",
            reaction = "react",
            state = "begin",
        );
        let entered = reaction.enter();
        tracing::trace!(
            target: "boomerang::trace",
            event = "action_schedule",
            enclave = "e0",
            action = "tick",
            logical_ns = 1_u64,
            microstep = 0_u64,
            outcome = "scheduled",
        );
        drop(entered);
        tracing::trace!(
            target: "boomerang::trace",
            event = "async_ingress",
            enclave = "e0",
            kind = "logical",
            action = "tick",
            logical_ns = 1_u64,
            microstep = 0_u64,
            outcome = "accepted",
        );
    });

    let records = capture.records();
    let reaction = records
        .iter()
        .find(|record| record.event == "reaction_execute")
        .unwrap();
    let scheduled = records
        .iter()
        .find(|record| record.event == "action_schedule")
        .unwrap();
    let ingress = records
        .iter()
        .find(|record| record.event == "async_ingress")
        .unwrap();
    assert_eq!(scheduled.parent_id.as_ref(), Some(&reaction.id));
    assert_eq!(ingress.parent_id, None, "ambiguous ingress stays neutral");
}

#[test]
fn simultaneous_events_receive_unique_adapter_ids() {
    let (session, capture) = session_with_capture("concurrent");
    let layer = session.layer();
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let subscriber = tracing_subscriber::registry().with(layer.clone());
            let barrier = barrier.clone();
            scope.spawn(move || {
                tracing::subscriber::with_default(subscriber, || {
                    barrier.wait();
                    tracing::trace!(
                        target: "boomerang::trace",
                        event = "shutdown",
                        enclave = "e0",
                        state = "complete",
                        outcome = "success",
                        worker,
                    );
                });
            });
        }
        barrier.wait();
    });

    let records = capture.records();
    assert_eq!(records.len(), 8);
    let mut ids = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8);
    assert!(ids.iter().all(|id| id.starts_with("concurrent:e0:")));
}

#[test]
fn independently_created_layers_share_the_session_id_sequence() {
    let (session, capture) = session_with_capture("shared-session");

    for layer in [session.layer(), session.layer()] {
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "boomerang::trace",
                event = "shutdown",
                enclave = "e0",
                state = "complete",
                outcome = "success",
            );
        });
    }

    let records = capture.records();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0].id, records[1].id);
}

#[test]
fn malformed_trace_emits_one_non_recursive_schema_diagnostic() {
    let (session, capture) = session_with_capture("diagnostic");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(target: "boomerang::trace", event = "shutdown", outcome = "success");
        tracing::trace!(target: "boomerang::trace", enclave = "e0", outcome = "success");
        tracing::trace!(target: "unrelated", event = "shutdown", enclave = "e0");
    });

    let records = capture.records();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.event == "diagnostic"));
    assert!(records
        .iter()
        .all(|record| record.entity_path == "/diagnostics/schema"));
}

struct FailingWriter;

impl TraceWriter for FailingWriter {
    fn write(
        &self,
        _recording: &rerun::RecordingStream,
        _record: &TraceRecord,
    ) -> Result<(), TraceWriterError> {
        Err("injected writer failure".into())
    }
}

#[test]
fn first_write_failure_disables_layer_and_later_records_are_skipped() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .trace_writer(Arc::new(FailingWriter))
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..3 {
            tracing::trace!(
                target: "boomerang::trace",
                event = "shutdown",
                enclave = "e0",
                state = "complete",
                outcome = "success",
            );
        }
    });

    assert!(!session.is_enabled());
    assert_eq!(session.error_count(), 1);
    assert_eq!(session.skipped_count(), 2);
}

struct PanickingWriter;

impl TraceWriter for PanickingWriter {
    fn write(
        &self,
        _recording: &rerun::RecordingStream,
        _record: &TraceRecord,
    ) -> Result<(), TraceWriterError> {
        panic!("injected writer panic")
    }
}

#[test]
fn writer_panic_isolated_from_traced_application() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .trace_writer(Arc::new(PanickingWriter))
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let application_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "boomerang::trace",
                event = "shutdown",
                enclave = "e0",
                state = "complete",
                outcome = "success",
            );
        });
    }));

    assert!(application_result.is_ok());
    assert!(!session.is_enabled());
    assert_eq!(session.error_count(), 1);
}
