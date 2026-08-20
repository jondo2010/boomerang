use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use rerun::{RecordingStream, RecordingStreamBuilder};

#[cfg(feature = "federated")]
use super::entities::{escape_entity_segment, log_runtime_relation, runtime_enclave_root};
use super::entities::{log_runtime_enclaves, RegistrationIndex, RerunTraceWriter, TraceWriter};
use super::layer::RerunLayer;

const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Destination for a Rerun recording.
///
/// Active memory, file, and tee sinks use Rerun 0.36.1's bounded batching pipeline. When that
/// pipeline is saturated, logging from [`RerunLayer`] may apply backpressure to the scheduler
/// callback that emitted the trace. Boomerang does not add a second dynamic-record queue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SinkConfig {
    /// Retain the recording in memory.
    #[default]
    Memory,
    /// Write a recording to an RRD file.
    File(std::path::PathBuf),
    /// Request streaming to a Rerun data proxy.
    ///
    /// Rerun 0.36.1 only exposes a blocking, bounded-channel client sink, and does not expose the
    /// requested client memory limit. Building this configuration therefore returns
    /// [`RerunSessionBuildError::UnsupportedGrpc`] instead of risking scheduler backpressure.
    Grpc {
        url: String,
        memory_limit_bytes: usize,
    },
    /// Send every record to all configured destinations.
    Tee(Vec<Self>),
}

impl SinkConfig {
    /// Returns a deterministic, one-level sink configuration.
    pub fn normalized(self) -> Result<Self, SinkConfigError> {
        match self {
            Self::Tee(sinks) if sinks.is_empty() => Err(SinkConfigError::EmptyTee),
            Self::Tee(sinks) => {
                let mut flattened = Vec::new();
                for sink in sinks {
                    match sink.normalized()? {
                        Self::Tee(children) => flattened.extend(children),
                        leaf => flattened.push(leaf),
                    }
                }
                if flattened.is_empty() {
                    Err(SinkConfigError::EmptyTee)
                } else {
                    Ok(Self::Tee(flattened))
                }
            }
            leaf => Ok(leaf),
        }
    }
}

/// Invalid sink topology.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SinkConfigError {
    #[error("a Rerun tee sink must contain at least one destination")]
    EmptyTee,
}

/// Viewer layout attached to a recording.
#[derive(Default)]
pub enum BlueprintConfig {
    /// Boomerang's timeline-first debugging layout.
    #[default]
    Default,
    /// Do not attach a blueprint.
    None,
    /// Attach a caller-provided Rerun blueprint.
    Custom(Box<rerun::blueprint::Blueprint>),
}

/// Errors produced while configuring an observational recording session.
#[derive(Debug, thiserror::Error)]
pub enum RerunSessionBuildError {
    #[error(transparent)]
    SinkConfig(#[from] SinkConfigError),
    #[error(transparent)]
    Recording(#[from] rerun::RecordingStreamError),
    #[error(transparent)]
    FileSink(#[from] rerun::sink::FileSinkError),
    #[error(
        "Rerun 0.36.1 gRPC uses blocking backpressure and cannot be isolated from the scheduler"
    )]
    UnsupportedGrpc,
}

/// Adapter-local flush seam used to isolate sink behavior from application execution.
pub trait FlushDriver: Send + Sync + 'static {
    fn flush(
        &self,
        recording: &RecordingStream,
        timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError>;
}

struct SdkFlushDriver;

impl FlushDriver for SdkFlushDriver {
    fn flush(
        &self,
        recording: &RecordingStream,
        timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        recording.flush_with_timeout(timeout)
    }
}

/// Configures a [`RerunSession`].
///
/// Enabled sessions inherit Rerun 0.36.1's bounded batching and may backpressure trace callbacks
/// under saturation. Disabled tracing performs no trace metadata collection and does not alter
/// runtime object layouts.
pub struct RerunSessionBuilder {
    application_id: String,
    source_id: Option<String>,
    sink: SinkConfig,
    blueprint: BlueprintConfig,
    flush_timeout: Duration,
    flush_driver: Arc<dyn FlushDriver>,
    trace_writer: Arc<dyn TraceWriter>,
}

impl RerunSessionBuilder {
    /// Creates a builder for the given Rerun application ID.
    pub fn new(application_id: impl Into<String>) -> Self {
        Self {
            application_id: application_id.into(),
            source_id: None,
            sink: SinkConfig::default(),
            blueprint: BlueprintConfig::default(),
            flush_timeout: DEFAULT_FLUSH_TIMEOUT,
            flush_driver: Arc::new(SdkFlushDriver),
            trace_writer: Arc::new(RerunTraceWriter),
        }
    }

    /// Identifies the process or adapter producing this recording.
    pub fn source_id(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }

    /// Selects the recording destination.
    pub fn sink(mut self, sink: SinkConfig) -> Self {
        self.sink = sink;
        self
    }

    /// Selects the viewer blueprint attached to the recording.
    pub fn blueprint(mut self, blueprint: BlueprintConfig) -> Self {
        self.blueprint = blueprint;
        self
    }

    /// Sets the maximum time spent flushing on an explicit flush or drop.
    pub fn flush_timeout(mut self, flush_timeout: Duration) -> Self {
        self.flush_timeout = flush_timeout;
        self
    }

    /// Overrides the adapter-local flush operation.
    ///
    /// This is primarily useful for verifying timeout isolation. The adapter always invokes the
    /// driver on a detached, bounded-wait thread so a broken primitive cannot block application
    /// shutdown. A permanently blocked driver can retain one detached thread per session.
    pub fn flush_driver(mut self, flush_driver: Arc<dyn FlushDriver>) -> Self {
        self.flush_driver = flush_driver;
        self
    }

    /// Overrides the synchronous dynamic-record writer.
    ///
    /// This seam supports deterministic testing and future file/live/tee sinks without adding a
    /// second trace queue.
    pub fn trace_writer(mut self, trace_writer: Arc<dyn TraceWriter>) -> Self {
        self.trace_writer = trace_writer;
        self
    }

    /// Builds the recording session.
    pub fn build(self) -> Result<RerunSession, RerunSessionBuildError> {
        let sink = self.sink.normalized()?;
        let mut builder =
            RecordingStreamBuilder::new(rerun::ApplicationId::new_or_unknown(self.application_id));
        builder = match self.blueprint {
            BlueprintConfig::Default => builder.with_default_blueprint(default_blueprint()),
            BlueprintConfig::None => builder,
            BlueprintConfig::Custom(blueprint) => builder.with_default_blueprint(*blueprint),
        };
        let (recording, initial_memory) = builder.memory()?;
        let memory = configure_sinks(&recording, sink, initial_memory)?;
        let source_id = self.source_id.unwrap_or_else(|| {
            recording
                .store_info()
                .map(|info| info.recording_id().to_string())
                .unwrap_or_else(|| {
                    let sequence = SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
                    format!("process-{}-{sequence}", std::process::id())
                })
        });
        let enabled = recording.is_enabled();

        Ok(RerunSession {
            recording,
            memory,
            source_id,
            flush_timeout: self.flush_timeout,
            flush_driver: self.flush_driver,
            state: SessionState::new(enabled),
            trace_writer: self.trace_writer,
            started: Instant::now(),
            next_trace_id: Arc::new(AtomicU64::new(0)),
            registration: Arc::new(RwLock::new(RegistrationIndex::default())),
        })
    }
}

/// An observational Rerun recording session.
///
/// An active memory, file, or tee sink is not nonblocking: Rerun's bounded batching may
/// backpressure scheduler callbacks under saturation. Once disabled, the tracing annotations
/// retain their zero-metadata-work path.
pub struct RerunSession {
    recording: RecordingStream,
    memory: Option<rerun::sink::MemorySinkStorage>,
    source_id: String,
    flush_timeout: Duration,
    flush_driver: Arc<dyn FlushDriver>,
    state: SessionState,
    trace_writer: Arc<dyn TraceWriter>,
    started: Instant,
    next_trace_id: Arc<AtomicU64>,
    registration: Arc<RwLock<RegistrationIndex>>,
}

impl RerunSession {
    /// Whether recording attempts are still enabled.
    pub fn is_enabled(&self) -> bool {
        self.state.is_enabled()
    }

    /// Number of recording errors observed by this session.
    pub fn error_count(&self) -> usize {
        self.state.error_count()
    }

    /// Number of recording attempts skipped after the session was disabled.
    pub fn skipped_count(&self) -> usize {
        self.state.skipped_count()
    }

    /// Identifier for the process or adapter producing this recording.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Creates a composable tracing layer without installing a global subscriber.
    pub fn layer(&self) -> RerunLayer {
        RerunLayer::new(
            self.recording.clone_weak(),
            self.state.clone(),
            Arc::from(self.source_id.as_str()),
            self.trace_writer.clone(),
            self.started,
            self.next_trace_id.clone(),
            self.registration.clone(),
        )
    }

    /// Access the backing memory sink for recording inspection or serialization.
    pub fn memory_sink(&self) -> Option<&rerun::sink::MemorySinkStorage> {
        self.memory.as_ref()
    }

    /// Registers the immutable hierarchy already produced by builder lowering.
    ///
    /// Registration is synchronous and retains no reference to, or copy of, the runtime graph.
    pub fn register_runtime(&self, runtime: &boomerang_builder::RuntimeAssembly) {
        match &runtime.execution {
            boomerang_builder::RuntimeExecution::Local(enclaves) => {
                self.register_enclaves(None, enclaves);
            }
            #[cfg(feature = "federated")]
            boomerang_builder::RuntimeExecution::Federated(federation) => {
                if let Some(index) = self.observe_registration(|| {
                    let mut index = RegistrationIndex::default();
                    let mut federation_nodes = Vec::new();
                    let mut federation_edges = Vec::new();
                    for (id, federate) in federation.federates() {
                        let path = format!("/federates/{}", escape_entity_segment(id.as_str()));
                        federation_nodes.push(path.clone());
                        let entity = rerun::DynamicArchetype::new("boomerang.RuntimeEntity")
                            .with_component::<rerun::components::Text>(
                                "boomerang.runtime.display_name",
                                [id.as_str()],
                            )
                            .with_component::<rerun::components::Text>(
                                "boomerang.runtime.stable_key",
                                [id.as_str()],
                            )
                            .with_component::<rerun::components::Text>(
                                "boomerang.runtime.kind",
                                ["federate"],
                            );
                        self.recording.log_static(path.as_str(), &entity)?;
                        for enclave in federate.enclaves().keys() {
                            let enclave_path = runtime_enclave_root(Some(id.as_str()), enclave);
                            let relation_index = federation_edges.len();
                            log_runtime_relation(
                                &self.recording,
                                &format!("/federation/topology/ownership/{relation_index}"),
                                &path,
                                &enclave_path,
                                "owns_enclave",
                                None,
                                None,
                            )?;
                            federation_nodes.push(enclave_path.clone());
                            federation_edges.push((path.clone(), enclave_path));
                        }
                        log_runtime_enclaves(
                            &self.recording,
                            Some(id.as_str()),
                            federate.enclaves(),
                            &mut index,
                        )?;
                    }

                    for (endpoint, source, target, delay) in federation.graph().endpoint_routes() {
                        let source =
                            format!("/federates/{}", escape_entity_segment(source.as_str()));
                        let target =
                            format!("/federates/{}", escape_entity_segment(target.as_str()));
                        log_runtime_relation(
                            &self.recording,
                            &format!(
                                "/federation/topology/endpoints/{}",
                                escape_entity_segment(endpoint.as_str())
                            ),
                            &source,
                            &target,
                            "federated_endpoint",
                            Some(endpoint.as_str()),
                            Some(delay.as_nanos()),
                        )?;
                        federation_edges.push((source, target));
                    }
                    self.recording.log_static(
                        "/federation/topology",
                        &rerun::GraphNodes::new(federation_nodes.clone())
                            .with_labels(federation_nodes),
                    )?;
                    self.recording.log_static(
                        "/federation/topology",
                        &rerun::GraphEdges::new(federation_edges)
                            .with_graph_type(rerun::components::GraphType::Directed),
                    )?;
                    Ok(index)
                }) {
                    *self
                        .registration
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = index;
                }
            }
        }
    }

    /// Registers one already-lowered Enclave map without retaining it.
    pub fn register_enclaves(
        &self,
        federate: Option<&str>,
        enclaves: &boomerang_tinymap::TinyMap<
            boomerang_runtime::EnclaveKey,
            boomerang_runtime::Enclave,
        >,
    ) {
        if let Some(index) = self.observe_registration(|| {
            let mut index = RegistrationIndex::default();
            log_runtime_enclaves(&self.recording, federate, enclaves, &mut index)?;
            Ok(index)
        }) {
            *self
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = index;
        }
    }

    fn observe_registration<T>(
        &self,
        registration: impl FnOnce() -> rerun::RecordingStreamResult<T>,
    ) -> Option<T> {
        if !self.state.try_begin_attempt() {
            return None;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(registration)) {
            Ok(Ok(value)) => Some(value),
            Ok(Err(error)) => {
                self.state.disable_on_error(&error);
                None
            }
            Err(_) => {
                self.state
                    .disable_on_error(&"runtime registration panicked");
                None
            }
        }
    }

    /// Flushes pending data once, bounded by the configured timeout.
    pub fn flush(&self) {
        let recording = self.recording.clone();
        let driver = self.flush_driver.clone();
        let timeout = self.flush_timeout;
        self.state
            .flush_once(|| bounded_flush(recording, driver, timeout));
    }
}

fn configure_sinks(
    recording: &RecordingStream,
    config: SinkConfig,
    initial_memory: rerun::sink::MemorySinkStorage,
) -> Result<Option<rerun::sink::MemorySinkStorage>, RerunSessionBuildError> {
    if config == SinkConfig::Memory {
        return Ok(Some(initial_memory));
    }

    let leaves = match config {
        SinkConfig::Tee(leaves) => leaves,
        leaf => vec![leaf],
    };
    if leaves
        .iter()
        .any(|leaf| matches!(leaf, SinkConfig::Grpc { .. }))
    {
        return Err(RerunSessionBuildError::UnsupportedGrpc);
    }
    let mut memory = None;
    let mut sinks: Vec<Box<dyn rerun::sink::LogSink>> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        match leaf {
            SinkConfig::Memory => {
                let sink = rerun::sink::MemorySink::new(recording.clone());
                memory = Some(sink.buffer());
                sinks.push(Box::new(sink));
            }
            SinkConfig::File(path) => sinks.push(Box::new(rerun::sink::FileSink::new(path)?)),
            SinkConfig::Grpc {
                url: _,
                memory_limit_bytes: _,
            } => unreachable!("unsupported gRPC sinks were rejected before sink construction"),
            SinkConfig::Tee(_) => unreachable!("sink configuration was normalized"),
        }
    }
    recording.set_sink(Box::new(rerun::sink::MultiSink::new(sinks)));
    Ok(memory)
}

#[derive(Debug, thiserror::Error)]
enum BoundedFlushError {
    #[error("failed to spawn bounded Rerun flush worker: {0}")]
    Spawn(std::io::Error),
    #[error("Rerun flush exceeded {0:?}")]
    Timeout(Duration),
    #[error("Rerun flush worker disconnected")]
    Disconnected,
    #[error(transparent)]
    Sink(#[from] rerun::sink::SinkFlushError),
}

fn bounded_flush(
    recording: RecordingStream,
    driver: Arc<dyn FlushDriver>,
    timeout: Duration,
) -> Result<(), BoundedFlushError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("boomerang-rerun-flush".to_owned())
        .spawn(move || {
            let _ = sender.send(driver.flush(&recording, timeout));
        })
        .map_err(BoundedFlushError::Spawn)?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(Into::into),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(BoundedFlushError::Timeout(timeout)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(BoundedFlushError::Disconnected)
        }
    }
}

fn default_blueprint() -> rerun::blueprint::Blueprint {
    use rerun::blueprint::{
        Blueprint, DataframeView, GraphView, Grid, StateTimelineView, TextLogView, TimePanel,
        TimeSeriesView,
    };

    let roots = ["/enclaves/**", "/federates/**"];
    let scheduler = StateTimelineView::new("Scheduler timeline")
        .with_origin("/")
        .with_contents([
            "/enclaves/**/scheduler/**",
            "/federates/**/enclaves/**/scheduler/**",
        ]);
    let events = StateTimelineView::new("Event streams")
        .with_origin("/")
        .with_contents(roots);
    let topology = GraphView::new("Ownership and propagation")
        .with_origin("/")
        .with_contents([
            "/enclaves/**/topology",
            "/federates/**/enclaves/**/topology",
            "/federation/topology",
            "/propagation/**",
        ]);
    let selected = DataframeView::new("Selected records")
        .with_origin("/")
        .with_contents(roots);
    let diagnostics = TextLogView::new("Diagnostics")
        .with_origin("/diagnostics")
        .with_contents(["/diagnostics/**"]);
    let measures = TimeSeriesView::new("Operational measures")
        .with_origin("/")
        .with_contents(roots);

    Blueprint::new(
        Grid::new([
            scheduler.into(),
            events.into(),
            topology.into(),
            selected.into(),
            diagnostics.into(),
            measures.into(),
        ])
        .with_name("Boomerang timeline-first debugger")
        .with_grid_columns(2),
    )
    .with_auto_views(false)
    .with_auto_layout(false)
    .with_time_panel(TimePanel::new().with_timeline("logical"))
}

impl Drop for RerunSession {
    fn drop(&mut self) {
        self.flush();
    }
}

#[derive(Clone)]
pub(super) struct SessionState {
    inner: Arc<SessionStateInner>,
}

struct SessionStateInner {
    enabled: AtomicBool,
    error_count: AtomicUsize,
    skipped_count: AtomicUsize,
    warned: AtomicBool,
    flushed: AtomicBool,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new(true)
    }
}

impl SessionState {
    fn new(enabled: bool) -> Self {
        Self {
            inner: Arc::new(SessionStateInner {
                enabled: AtomicBool::new(enabled),
                error_count: AtomicUsize::new(0),
                skipped_count: AtomicUsize::new(0),
                warned: AtomicBool::new(false),
                flushed: AtomicBool::new(false),
            }),
        }
    }

    fn is_enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Acquire)
    }

    fn error_count(&self) -> usize {
        self.inner.error_count.load(Ordering::Relaxed)
    }

    fn skipped_count(&self) -> usize {
        self.inner.skipped_count.load(Ordering::Relaxed)
    }

    pub(super) fn try_begin_attempt(&self) -> bool {
        if self.is_enabled() {
            true
        } else {
            self.inner.skipped_count.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn flush_once<E>(&self, flush: impl FnOnce() -> Result<(), E>)
    where
        E: std::fmt::Display,
    {
        if self.inner.flushed.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Err(error) = flush() {
            self.disable_on_error(&error);
        }
    }

    pub(super) fn disable_on_error(&self, error: &dyn std::fmt::Display) {
        if self.inner.enabled.swap(false, Ordering::AcqRel) {
            self.inner.error_count.fetch_add(1, Ordering::Relaxed);
            if !self.inner.warned.swap(true, Ordering::AcqRel) {
                tracing::warn!(
                    target: "boomerang::rerun_internal",
                    %error,
                    "disabling Rerun recording after an observational failure"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionState;

    #[test]
    fn first_failure_disables_once_and_later_attempts_are_skipped() {
        let state = SessionState::default();
        let shared_state = state.clone();

        assert!(state.try_begin_attempt());
        state.disable_on_error(&"injected recording failure");
        assert!(!state.is_enabled());
        assert_eq!(state.error_count(), 1);
        assert_eq!(state.skipped_count(), 0);

        assert!(!shared_state.try_begin_attempt());
        assert!(!shared_state.try_begin_attempt());
        state.disable_on_error(&"later concurrent failure");
        assert_eq!(state.error_count(), 1);
        assert_eq!(state.skipped_count(), 2);
    }

    #[test]
    fn flush_once_is_idempotent() {
        let state = SessionState::default();
        let mut flushes = 0;

        state.flush_once(|| {
            flushes += 1;
            Ok::<_, &'static str>(())
        });
        state.flush_once(|| {
            flushes += 1;
            Ok::<_, &'static str>(())
        });

        assert_eq!(flushes, 1);
    }
}
