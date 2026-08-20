#![cfg(feature = "rerun")]

use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use boomerang::rerun::{
    BlueprintConfig, FlushDriver, RerunSessionBuilder, SinkConfig, TraceRecord, TraceWriter,
    TraceWriterError,
};
use boomerang::runtime::{Enclave, Port, Reactor};
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

fn memory_paths(session: &boomerang::rerun::RerunSession) -> Vec<String> {
    session
        .memory_sink()
        .expect("memory sink")
        .take()
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .map(|chunk| chunk.entity_path().to_string())
        .collect()
}

fn emit_shutdown(session: &boomerang::rerun::RerunSession) {
    let subscriber = tracing_subscriber::registry().with(session.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            logical_ns = 42_u64,
            state = "complete",
            outcome = "success",
        );
    });
}

#[test]
fn sink_config_rejects_empty_tee_and_flattens_nested_tees() {
    let empty = SinkConfig::Tee(Vec::new()).normalized();
    assert!(empty.is_err());
    assert!(RerunSessionBuilder::new("empty-tee")
        .sink(SinkConfig::Tee(Vec::new()))
        .build()
        .is_err());

    let path = std::path::PathBuf::from("trace.rrd");
    let nested = SinkConfig::Tee(vec![
        SinkConfig::Memory,
        SinkConfig::Tee(vec![
            SinkConfig::File(path.clone()),
            SinkConfig::Grpc {
                url: "rerun+http://127.0.0.1:9876/proxy".to_owned(),
                memory_limit_bytes: 4096,
            },
        ]),
    ])
    .normalized()
    .unwrap();

    assert_eq!(
        nested,
        SinkConfig::Tee(vec![
            SinkConfig::Memory,
            SinkConfig::File(path),
            SinkConfig::Grpc {
                url: "rerun+http://127.0.0.1:9876/proxy".to_owned(),
                memory_limit_bytes: 4096,
            },
        ])
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("must-not-be-created.rrd");
    assert!(RerunSessionBuilder::new("unsupported-grpc-tee")
        .sink(SinkConfig::Tee(vec![
            SinkConfig::File(path.clone()),
            SinkConfig::Grpc {
                url: "rerun+http://127.0.0.1:9/proxy".to_owned(),
                memory_limit_bytes: 4096,
            },
        ]))
        .build()
        .is_err());
    assert!(!path.exists());
}

#[test]
fn file_sink_writes_decodable_trace_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trace.rrd");
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();

    assert!(session.memory_sink().is_none());
    emit_shutdown(&session);
    session.flush();
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    drop(session);

    let file = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let messages = rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let chunks = messages
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch).unwrap())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let trace = chunks
        .iter()
        .find(|chunk| chunk.entity_path().to_string().ends_with("/shutdown"))
        .expect("dynamic trace chunk in .rrd");
    let timelines = trace
        .timelines()
        .keys()
        .map(|timeline| timeline.as_str())
        .collect::<Vec<_>>();
    assert!(timelines.contains(&"elapsed"));
    assert!(timelines.contains(&"wall_clock"));
    assert!(timelines.contains(&"logical"));
    assert!(trace.component_descriptors().any(|descriptor| {
        descriptor
            .archetype
            .as_ref()
            .is_some_and(|name| name == "boomerang.TraceRecord")
    }));
}

#[test]
fn tee_writes_the_same_trace_to_memory_and_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tee.rrd");
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .sink(SinkConfig::Tee(vec![
            SinkConfig::Memory,
            SinkConfig::File(path.clone()),
        ]))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();

    emit_shutdown(&session);
    session.flush();
    assert!(memory_paths(&session)
        .iter()
        .any(|path| path.ends_with("/shutdown")));
    drop(session);

    let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    let has_trace = rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
        .filter_map(Result::ok)
        .any(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                rerun::log::Chunk::from_chunk_record_batch(&message.batch)
                    .is_ok_and(|chunk| chunk.entity_path().to_string().ends_with("/shutdown"))
            }
            _ => false,
        });
    assert!(has_trace);
}

struct SlowFlush;

impl FlushDriver for SlowFlush {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}

#[test]
fn blocking_flush_driver_is_bounded_and_observational() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .flush_timeout(Duration::from_millis(10))
        .flush_driver(Arc::new(SlowFlush))
        .build()
        .unwrap();
    let started = Instant::now();

    session.flush();

    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(!session.is_enabled());
    assert_eq!(session.error_count(), 1);
    session.flush();
    assert_eq!(session.error_count(), 1);
}

#[test]
fn default_blueprint_contains_timeline_first_views() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let messages = session.memory_sink().unwrap().take();
    let blueprint_chunks = messages
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                rerun::log::Chunk::from_chunk_record_batch(&message.batch).ok()
            }
            _ => None,
        })
        .filter(|chunk| {
            chunk.entity_path().to_string().starts_with("/view/")
                || chunk.entity_path().to_string() == "/time_panel"
        })
        .collect::<Vec<_>>();
    let debug = format!("{blueprint_chunks:#?}");
    for name in [
        "Scheduler timeline",
        "Event streams",
        "Ownership and propagation",
        "Selected records",
        "Diagnostics",
        "Operational measures",
    ] {
        assert!(
            debug.contains(name),
            "missing blueprint view {name}: {debug}"
        );
    }
    assert!(
        debug.contains("logical"),
        "logical timeline is not selected"
    );
    assert!(debug.contains("/enclaves/**"));
    assert!(debug.contains("/federates/**"));
}

#[test]
fn blueprint_none_and_custom_control_blueprint_emission() {
    let none = RerunSessionBuilder::new("boomerang-rerun-test")
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();
    assert!(none
        .memory_sink()
        .unwrap()
        .take()
        .iter()
        .all(|message| message.store_id().kind() != rerun::StoreKind::Blueprint));

    let custom = rerun::blueprint::Blueprint::new(
        rerun::blueprint::TextLogView::new("Caller-defined diagnostics")
            .with_origin("/diagnostics"),
    );
    let custom = RerunSessionBuilder::new("boomerang-rerun-test")
        .blueprint(BlueprintConfig::Custom(Box::new(custom)))
        .build()
        .unwrap();
    let debug = format!("{:#?}", custom.memory_sink().unwrap().take());
    assert!(debug.contains("Caller-defined diagnostics"));
}

#[test]
fn local_registration_aligns_after_the_runtime_graph_is_dropped() {
    let (session, capture) = session_with_capture("local-source");
    let mut enclaves = boomerang_tinymap::TinyMap::new();
    let enclave_key = enclaves.insert(Enclave::default());
    let enclave = &mut enclaves[enclave_key];
    let reactor_key = enclave.insert_reactor(Reactor::new("local", ()).boxed(), None);
    let scope = enclave.root_scope(reactor_key);
    let port_key = enclave.insert_port(|key| Port::<u32>::new("input", key).boxed());
    enclave.insert_port_scope(port_key, scope);

    session.register_enclaves(None, &enclaves);
    let static_paths = memory_paths(&session);
    assert!(static_paths
        .iter()
        .any(|path| path == "/enclaves/EnclaveKey\\(0\\)"));
    assert!(static_paths.iter().any(|path| {
        path == "/enclaves/EnclaveKey\\(0\\)/reactors/local\\@ReactorKey\\(0\\)/ports/PortKey\\(0\\)"
    }));
    drop(enclaves);

    let subscriber = tracing_subscriber::registry().with(session.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(
            target: "boomerang::trace",
            event = "port_write",
            enclave = %enclave_key,
            port_key = %port_key,
            outcome = "test",
        );
    });

    let records = capture.records();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].entity_path,
        "/enclaves/EnclaveKey(0)/reactors/local@ReactorKey(0)/ports/PortKey(0)/port_write"
    );
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
fn explicit_root_span_does_not_inherit_current_trace_parent() {
    let (session, capture) = session_with_capture("explicit-root");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let parent = tracing::trace_span!(
            target: "boomerang::trace",
            "tag_process",
            event = "tag_process",
            enclave = "e0",
            state = "processing",
        );
        let _entered = parent.enter();
        let root = tracing::trace_span!(
            target: "boomerang::trace",
            parent: None,
            "reaction_execute",
            event = "reaction_execute",
            enclave = "e0",
            reactor = "r0",
            reaction = "root",
            state = "begin",
        );
        let _root_entered = root.enter();
    });

    let records = capture.records();
    let root = records
        .iter()
        .find(|record| record.fields.reaction.as_deref() == Some("root"))
        .unwrap();
    assert_eq!(root.parent_id, None);
}

#[test]
fn action_and_port_facts_use_their_own_keyed_entity_paths() {
    let (session, capture) = session_with_capture("primary-entity");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let reaction = tracing::trace_span!(
            target: "boomerang::trace",
            "reaction_execute",
            event = "reaction_execute",
            enclave = "e0",
            reactor = "reactor-label",
            reaction_key = "rk/4",
            reaction = "reaction-label",
            state = "begin",
        );
        let _entered = reaction.enter();
        tracing::trace!(
            target: "boomerang::trace",
            event = "action_schedule",
            action_key = "ak/2",
            action = "action-label",
            outcome = "scheduled",
        );
        tracing::trace!(
            target: "boomerang::trace",
            event = "port_write",
            port_key = "pk/3",
            port = "port-label",
            outcome = "mutable_access",
        );
    });

    let records = capture.records();
    let action = records
        .iter()
        .find(|record| record.event == "action_schedule")
        .unwrap();
    let port = records
        .iter()
        .find(|record| record.event == "port_write")
        .unwrap();
    assert_eq!(
        action.entity_path,
        "/enclaves/e0/actions/ak\\/2/action_schedule"
    );
    assert_eq!(port.entity_path, "/enclaves/e0/ports/pk\\/3/port_write");
}

#[test]
fn closed_runtime_span_has_duration_and_terminal_state() {
    let (session, capture) = session_with_capture("terminal-state");
    let subscriber = tracing_subscriber::registry().with(session.layer());

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::trace_span!(
            target: "boomerang::trace",
            "tag_process",
            event = "tag_process",
            enclave = "e0",
            terminal = true,
            state = "processing",
        );
        let _entered = span.enter();
    });

    let records = capture.records();
    assert_eq!(records.len(), 1);
    assert!(records[0].duration_ns.is_some());
    assert_eq!(records[0].terminal_state.as_deref(), Some("terminal"));
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

struct PanickingDebug;

impl std::fmt::Debug for PanickingDebug {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("injected Debug panic")
    }
}

#[test]
fn normalization_panic_isolated_and_disables_subsequent_callbacks() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let application_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "boomerang::trace",
                event = "shutdown",
                enclave = ?PanickingDebug,
                state = "complete",
                outcome = "success",
            );
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
    assert_eq!(session.skipped_count(), 1);
}

#[test]
fn poisoned_span_fields_are_recovered_without_application_panic() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let application_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::trace_span!(
                target: "boomerang::trace",
                "reaction_execute",
                event = "reaction_execute",
                enclave = "e0",
                reactor = "r0",
                reaction = "react",
                state = tracing::field::Empty,
            );
            span.record("state", tracing::field::debug(PanickingDebug));
            span.record("state", "recovered");
            let _entered = span.enter();
        });
    }));

    assert!(application_result.is_ok());
    assert!(!session.is_enabled());
    assert_eq!(session.error_count(), 1);
    assert!(session.skipped_count() >= 1);
}
