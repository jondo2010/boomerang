#![cfg(feature = "rerun")]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use boomerang::rerun::{
    BlueprintConfig, FlushDriver, RerunSessionBuildError, RerunSessionBuilder, SinkConfig,
    SinkConfigError, TraceRecord, TraceWriter, TraceWriterError,
};
use boomerang::runtime::{Enclave, Port, Reactor};
use rerun::external::arrow::array::Array as _;
use rerun::external::re_log_encoding::Decodable as _;
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
        .take_memory_snapshot_bounded()
        .expect("memory sink")
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

fn decode_chunks(
    messages: Vec<rerun::log::LogMsg>,
) -> Result<Vec<rerun::log::Chunk>, rerun::log::ChunkError> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            rerun::log::LogMsg::ArrowMsg(_, message) => {
                Some(rerun::log::Chunk::from_chunk_record_batch(&message.batch))
            }
            _ => None,
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
struct DecodedShutdown {
    entity_path: String,
    timelines: BTreeMap<String, Vec<i64>>,
    descriptors: Vec<(String, Option<String>)>,
    event: String,
    state: String,
    outcome: String,
}

fn text_component(chunk: &rerun::log::Chunk, suffix: &str) -> String {
    let descriptor = chunk
        .component_descriptors()
        .find(|descriptor| descriptor.component.as_str().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing component {suffix}"));
    let values = chunk
        .component_batch_raw(descriptor.component, 0)
        .unwrap_or_else(|| panic!("missing component batch {suffix}"))
        .unwrap();
    let values = values
        .as_any()
        .downcast_ref::<rerun::external::arrow::array::StringArray>()
        .unwrap_or_else(|| panic!("component {suffix} is not text"));
    assert_eq!(values.len(), 1);
    values.value(0).to_owned()
}

fn decoded_shutdown(chunks: &[rerun::log::Chunk]) -> DecodedShutdown {
    let chunk = chunks
        .iter()
        .find(|chunk| chunk.entity_path().to_string().ends_with("/shutdown"))
        .expect("shutdown trace record");
    let timelines = chunk
        .timelines()
        .values()
        .filter(|column| matches!(column.name(), "elapsed" | "wall_clock" | "logical"))
        .map(|column| (column.name().to_owned(), column.times_raw().to_vec()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(timelines.len(), 3);
    assert_eq!(timelines["logical"], vec![42]);
    assert!(timelines["elapsed"][0] >= 0);
    assert!(timelines["wall_clock"][0] > 0);
    let mut descriptors = chunk
        .component_descriptors()
        .map(|descriptor| {
            (
                descriptor.component.to_string(),
                descriptor.archetype.as_ref().map(ToString::to_string),
            )
        })
        .collect::<Vec<_>>();
    descriptors.sort();
    assert!(descriptors
        .iter()
        .any(|(_, archetype)| { archetype.as_deref() == Some("boomerang.TraceRecord") }));

    DecodedShutdown {
        entity_path: chunk.entity_path().to_string(),
        timelines,
        descriptors,
        event: text_component(chunk, ":boomerang.trace.event"),
        state: text_component(chunk, ":boomerang.trace.state"),
        outcome: text_component(chunk, ":boomerang.trace.outcome"),
    }
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
    let normalized_path = std::env::current_dir().unwrap().join(&path);
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
            SinkConfig::File(normalized_path),
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
fn invalid_sink_topology_is_rejected_before_file_construction() {
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("sentinel.rrd");
    std::fs::write(&sentinel, b"keep-me").unwrap();

    let later_empty = RerunSessionBuilder::new("later-empty")
        .sink(SinkConfig::Tee(vec![
            SinkConfig::File(sentinel.clone()),
            SinkConfig::Tee(Vec::new()),
        ]))
        .build();
    assert!(matches!(
        later_empty,
        Err(RerunSessionBuildError::SinkConfig(
            SinkConfigError::EmptyTee
        ))
    ));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep-me");

    let duplicate = RerunSessionBuilder::new("duplicate-file")
        .sink(SinkConfig::Tee(vec![
            SinkConfig::File(sentinel.clone()),
            SinkConfig::File(directory.path().join("child/../sentinel.rrd")),
        ]))
        .build();
    assert!(matches!(
        duplicate,
        Err(RerunSessionBuildError::SinkConfig(
            SinkConfigError::DuplicateFilePath(_)
        ))
    ));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep-me");

    let grpc = RerunSessionBuilder::new("unsupported-grpc")
        .sink(SinkConfig::Tee(vec![
            SinkConfig::File(sentinel.clone()),
            SinkConfig::Grpc {
                url: "rerun+http://127.0.0.1:9/proxy".to_owned(),
                memory_limit_bytes: 4096,
            },
        ]))
        .build();
    assert!(matches!(grpc, Err(RerunSessionBuildError::UnsupportedGrpc)));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep-me");
}

#[test]
fn file_sink_finish_writes_decodable_footer_bearing_trace() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trace.rrd");
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .sink(SinkConfig::File(path.clone()))
        .blueprint(BlueprintConfig::None)
        .build()
        .unwrap();

    assert!(session.take_memory_snapshot_bounded().is_none());
    emit_shutdown(&session);
    session.finish().unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 0);

    let bytes = std::fs::read(&path).unwrap();
    let footer_size = rerun::external::re_log_encoding::StreamFooter::ENCODED_SIZE_BYTES;
    let footer_bytes = bytes
        .get(bytes.len().saturating_sub(footer_size)..)
        .filter(|footer| footer.len() == footer_size)
        .expect("recorded RRD is missing or has a truncated footer");
    rerun::external::re_log_encoding::StreamFooter::from_rrd_bytes(footer_bytes)
        .expect("finalized RRD footer");

    let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
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

struct FailingFinalization;

impl FlushDriver for FailingFinalization {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        Err(rerun::sink::SinkFlushError::failed(
            "injected finalization failure",
        ))
    }
}

#[test]
fn finish_reports_finalization_failure() {
    let session = RerunSessionBuilder::new("failing-finalization")
        .blueprint(BlueprintConfig::None)
        .flush_driver(Arc::new(FailingFinalization))
        .build()
        .unwrap();

    let error = session.finish().unwrap_err();

    assert_eq!(
        error.to_string(),
        "failed to finalize Rerun recording: injected finalization failure"
    );
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
    let memory_messages = session.take_memory_snapshot_bounded().unwrap();
    drop(session);

    let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    let file_messages = rerun::external::re_log_encoding::DecoderApp::decode_lazy(file)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let memory_shutdown = decoded_shutdown(&decode_chunks(memory_messages).unwrap());
    let file_shutdown = decoded_shutdown(&decode_chunks(file_messages).unwrap());

    assert_eq!(memory_shutdown.event, "shutdown");
    assert_eq!(memory_shutdown.state, "complete");
    assert_eq!(memory_shutdown.outcome, "success");
    assert_eq!(memory_shutdown, file_shutdown);
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

struct GatedFirstFlush {
    first: AtomicBool,
    entered: Barrier,
    release: Barrier,
}

impl GatedFirstFlush {
    fn new() -> Self {
        Self {
            first: AtomicBool::new(true),
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

impl FlushDriver for GatedFirstFlush {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        if self.first.swap(false, Ordering::AcqRel) {
            self.entered.wait();
            self.release.wait();
        }
        Ok(())
    }
}

struct BlockingTeardown;

impl FlushDriver for BlockingTeardown {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        Ok(())
    }

    fn teardown(
        &self,
        recording: rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        drop(recording);
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }
}

struct JoinedLifecycle(Arc<AtomicUsize>);

impl FlushDriver for JoinedLifecycle {
    fn flush(
        &self,
        _recording: &rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        Ok(())
    }

    fn teardown(
        &self,
        recording: rerun::RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        drop(recording);
        self.0.store(1, Ordering::Release);
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
fn in_flight_lifecycle_operation_rejects_additional_requests() {
    let driver = Arc::new(GatedFirstFlush::new());
    let session = Arc::new(
        RerunSessionBuilder::new("bounded-lifecycle-submission")
            .flush_timeout(Duration::from_millis(200))
            .flush_driver(driver.clone())
            .build()
            .unwrap(),
    );
    let flushing = {
        let session = session.clone();
        std::thread::spawn(move || session.flush())
    };
    driver.entered.wait();

    let started = Instant::now();
    assert_eq!(session.take_memory_snapshot_bounded(), None);
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(session.is_enabled());

    driver.release.wait();
    flushing.join().unwrap();
    assert!(session.take_memory_snapshot_bounded().is_some());
}

#[test]
fn disabled_session_rejects_lifecycle_requests_without_queueing() {
    let driver = Arc::new(GatedFirstFlush::new());
    let session = RerunSessionBuilder::new("disabled-lifecycle-submission")
        .flush_timeout(Duration::from_millis(50))
        .flush_driver(driver.clone())
        .build()
        .unwrap();
    let releasing = {
        let driver = driver.clone();
        std::thread::spawn(move || {
            driver.entered.wait();
            std::thread::sleep(Duration::from_millis(150));
            driver.release.wait();
        })
    };

    session.flush();
    assert!(!session.is_enabled());
    let started = Instant::now();
    for _ in 0..4 {
        assert_eq!(session.take_memory_snapshot_bounded(), None);
    }
    assert!(started.elapsed() < Duration::from_millis(100));

    releasing.join().unwrap();
}

#[test]
fn session_drop_bounds_blocking_teardown_and_joins_normal_teardown() {
    let blocking = RerunSessionBuilder::new("blocking-teardown")
        .flush_timeout(Duration::from_millis(10))
        .flush_driver(Arc::new(BlockingTeardown))
        .build()
        .unwrap();
    let started = Instant::now();
    drop(blocking);
    assert!(started.elapsed() < Duration::from_millis(100));

    let joined = Arc::new(AtomicUsize::new(0));
    let normal = RerunSessionBuilder::new("joined-teardown")
        .flush_timeout(Duration::from_secs(1))
        .flush_driver(Arc::new(JoinedLifecycle(joined.clone())))
        .build()
        .unwrap();
    drop(normal);
    assert_eq!(joined.load(Ordering::Acquire), 1);
}

#[test]
fn default_blueprint_contains_timeline_first_views() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let messages = session.take_memory_snapshot_bounded().unwrap();
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
        .take_memory_snapshot_bounded()
        .unwrap()
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
    let debug = format!("{:#?}", custom.take_memory_snapshot_bounded().unwrap());
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
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let subscriber = tracing_subscriber::registry().with(session.layer());
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

struct CountingDebug<'a>(&'a AtomicUsize);

impl std::fmt::Debug for CountingDebug<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fetch_add(1, Ordering::Relaxed);
        formatter.write_str("e0")
    }
}

fn emit_counted_shutdown(evaluations: &AtomicUsize) {
    tracing::trace!(
        target: "boomerang::trace",
        event = "shutdown",
        enclave = ?CountingDebug(evaluations),
        state = "complete",
        outcome = "success",
    );
}

fn emit_counted_unrelated(evaluations: &AtomicUsize) {
    tracing::trace!(
        target: "unrelated",
        value = ?CountingDebug(evaluations),
    );
}

#[derive(Clone)]
struct EventCounter {
    target: &'static str,
    observed: Arc<AtomicUsize>,
}

struct DebugEvaluatingVisitor;

impl tracing::field::Visit for DebugEvaluatingVisitor {
    fn record_debug(&mut self, _field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = format!("{value:?}");
    }
}

impl<S> tracing_subscriber::Layer<S> for EventCounter
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() == self.target {
            self.observed.fetch_add(1, Ordering::Relaxed);
            event.record(&mut DebugEvaluatingVisitor);
        }
    }
}

fn emit_composed_counted_shutdown(evaluations: &AtomicUsize) {
    tracing::trace!(
        target: "boomerang::trace",
        event = "shutdown",
        enclave = ?CountingDebug(evaluations),
        state = "complete",
        outcome = "success",
    );
}

#[test]
fn first_write_failure_dynamically_disables_trace_callsites() {
    const CHILD_MARKER: &str = "BOOMERANG_RERUN_INTEREST_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "first_write_failure_dynamically_disables_trace_callsites",
            ])
            .env(CHILD_MARKER, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .trace_writer(Arc::new(FailingWriter))
        .build()
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(session.layer());

    let evaluations = AtomicUsize::new(0);
    let unrelated_evaluations = AtomicUsize::new(0);
    tracing::subscriber::with_default(subscriber, || {
        assert!(tracing::enabled!(
            target: "boomerang::trace",
            tracing::Level::TRACE
        ));
        emit_counted_shutdown(&evaluations);
        assert!(!tracing::enabled!(
            target: "boomerang::trace",
            tracing::Level::TRACE
        ));
        assert!(!tracing::enabled!(
            target: "unrelated",
            tracing::Level::TRACE
        ));
        emit_counted_unrelated(&unrelated_evaluations);
        emit_counted_shutdown(&evaluations);
    });

    assert!(!session.is_enabled());
    assert_eq!(session.error_count(), 1);
    assert_eq!(session.skipped_count(), 0);
    assert_eq!(evaluations.load(Ordering::Relaxed), 1);
    assert_eq!(unrelated_evaluations.load(Ordering::Relaxed), 0);
}

#[test]
fn another_interested_layer_keeps_disabled_trace_callsites_enabled() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .trace_writer(Arc::new(FailingWriter))
        .build()
        .unwrap();
    let observed = Arc::new(AtomicUsize::new(0));
    let observer = EventCounter {
        target: "boomerang::trace",
        observed: observed.clone(),
    }
    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
        metadata.target() == "boomerang::trace"
    }));
    let subscriber = tracing_subscriber::registry()
        .with(session.layer())
        .with(observer);

    let evaluations = AtomicUsize::new(0);
    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(
            target: "boomerang::trace",
            event = "shutdown",
            enclave = "e0",
            state = "complete",
            outcome = "success",
        );
        assert!(!session.is_enabled());
        assert!(tracing::enabled!(
            target: "boomerang::trace",
            tracing::Level::TRACE
        ));
        emit_composed_counted_shutdown(&evaluations);
    });

    assert_eq!(session.error_count(), 1);
    assert_eq!(session.skipped_count(), 0);
    assert_eq!(
        (
            evaluations.load(Ordering::Relaxed),
            observed.load(Ordering::Relaxed)
        ),
        (1, 2)
    );
}

#[test]
fn another_interested_layer_can_enable_an_unrelated_target() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let observed = Arc::new(AtomicUsize::new(0));
    let observer = EventCounter {
        target: "unrelated",
        observed: observed.clone(),
    }
    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
        metadata.target() == "unrelated"
    }));
    let subscriber = tracing_subscriber::registry()
        .with(session.layer())
        .with(observer);

    let evaluations = AtomicUsize::new(0);
    tracing::subscriber::with_default(subscriber, || {
        assert!(tracing::enabled!(
            target: "unrelated",
            tracing::Level::TRACE
        ));
        emit_counted_unrelated(&evaluations);
    });

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.skipped_count(), 0);
    assert_eq!(evaluations.load(Ordering::Relaxed), 1);
    assert_eq!(observed.load(Ordering::Relaxed), 1);
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
    assert_eq!(session.skipped_count(), 0);
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
