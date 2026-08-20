use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rerun::{RecordingStream, RecordingStreamBuilder};

#[cfg(feature = "federated")]
use super::entities::{escape_entity_segment, log_runtime_relation, runtime_enclave_root};
use super::entities::{log_runtime_enclaves, RegistrationIndex, RerunTraceWriter, TraceWriter};
use super::layer::{AdapterState, RerunLayer, SessionFilter};
use tracing_subscriber::Layer as _;

const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Destination for a Rerun recording.
///
/// Active memory, file, and tee sinks use Rerun 0.36.1's bounded batching pipeline. That does not
/// bound a memory sink, which retains the full recording. File sinks disable the SDK's O(chunks)
/// footer manifest by default. When the pipeline is saturated, logging from [`RerunLayer`] may
/// apply backpressure to the scheduler callback. Boomerang adds no second dynamic-record queue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SinkConfig {
    /// Retain the full recording in memory; active memory use grows with the trace.
    #[default]
    Memory,
    /// Write a recording to a sequentially-decodable RRD file.
    ///
    /// Footer emission is disabled so Rerun 0.36.1 does not retain an O(chunks) manifest for the
    /// lifetime of long recordings. This reduces random-access performance; `rerun rrd optimize`
    /// can add a footer after recording.
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
    /// Returns a deterministic, one-level sink configuration with lexically normalized absolute
    /// file paths. Normalization does not access the filesystem or require paths to exist.
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
            Self::File(path) => lexical_absolute_path(&path).map(Self::File),
            leaf => Ok(leaf),
        }
        .and_then(validate_unique_file_paths)
    }
}

fn validate_unique_file_paths(config: SinkConfig) -> Result<SinkConfig, SinkConfigError> {
    let leaves = match &config {
        SinkConfig::Tee(leaves) => leaves.as_slice(),
        leaf => std::slice::from_ref(leaf),
    };
    let mut files = std::collections::HashSet::new();
    for leaf in leaves {
        if let SinkConfig::File(path) = leaf {
            let key = lexical_absolute_path(path)?;
            if !files.insert(key.clone()) {
                return Err(SinkConfigError::DuplicateFilePath(key));
            }
        }
    }
    Ok(config)
}

fn lexical_absolute_path(path: &std::path::Path) -> Result<std::path::PathBuf, SinkConfigError> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| SinkConfigError::CurrentDirectory(error.to_string()))?
            .join(path)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

/// Invalid sink topology.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SinkConfigError {
    #[error("a Rerun tee sink must contain at least one destination")]
    EmptyTee,
    #[error("multiple Rerun file sinks resolve to the same path: {0}")]
    DuplicateFilePath(std::path::PathBuf),
    #[error("failed to resolve the current directory while validating Rerun sinks: {0}")]
    CurrentDirectory(String),
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
    #[error("failed to spawn the Rerun lifecycle worker: {0}")]
    LifecycleWorker(std::io::Error),
}

/// Adapter-local flush seam used to isolate sink behavior from application execution.
pub trait FlushDriver: Send + Sync + 'static {
    fn flush(
        &self,
        recording: &RecordingStream,
        timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError>;

    /// Performs the last strong recording drop after the worker's final flush and disconnect.
    fn teardown(
        &self,
        recording: RecordingStream,
        _timeout: Duration,
    ) -> Result<(), rerun::sink::SinkFlushError> {
        drop(recording);
        Ok(())
    }
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
    /// This is primarily useful for verifying timeout isolation. The adapter invokes the driver
    /// on one persistent lifecycle worker with at most one admitted operation, so a broken
    /// primitive cannot block the application thread or accumulate lifecycle requests. If the SDK
    /// never returns, that one detached worker and its sink resources may remain for the failed
    /// session; the adapter cannot cancel SDK work safely.
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
    ///
    /// The complete topology is normalized and validated before Rerun creates a memory sink or
    /// opens a file. Once valid sink construction starts, filesystem I/O failures are not
    /// transactional and may leave an earlier file destination created or truncated.
    pub fn build(self) -> Result<RerunSession, RerunSessionBuildError> {
        let sink = self.sink.normalized()?;
        if sink_contains_grpc(&sink) {
            return Err(RerunSessionBuildError::UnsupportedGrpc);
        }
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
        let has_memory = memory.is_some();
        let state = SessionState::new(enabled);
        let lifecycle = LifecycleWorker::spawn(
            recording.clone(),
            memory,
            self.flush_driver,
            self.flush_timeout,
            state.clone(),
        )
        .map_err(RerunSessionBuildError::LifecycleWorker)?;

        Ok(RerunSession {
            recording: recording.clone_weak(),
            has_memory,
            source_id,
            flush_timeout: self.flush_timeout,
            lifecycle: Some(lifecycle),
            state,
            trace_writer: self.trace_writer,
            started: Instant::now(),
            adapter: AdapterState::default(),
        })
    }
}

/// An observational Rerun recording session.
///
/// An active memory, file, or tee sink is not nonblocking: Rerun's bounded batching may
/// backpressure scheduler callbacks under saturation. Once disabled, its trace callsites are
/// cached as uninterested when no other subscriber or layer needs them.
pub struct RerunSession {
    recording: RecordingStream,
    has_memory: bool,
    source_id: String,
    flush_timeout: Duration,
    lifecycle: Option<LifecycleWorker>,
    state: SessionState,
    trace_writer: Arc<dyn TraceWriter>,
    started: Instant,
    pub(super) adapter: AdapterState,
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
    ///
    /// After this session is disabled, its per-layer filter rejects `boomerang::trace` callsites
    /// dynamically. If another composed layer remains interested in those callsites, tracing must
    /// still construct their metadata and fields for that layer; only this adapter's callbacks are
    /// skipped. This adapter expresses no interest in unrelated targets, while other composed
    /// layers remain free to enable them.
    pub fn layer<S>(&self) -> impl tracing_subscriber::Layer<S>
    where
        S: tracing::Subscriber
            + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>
            + 'static,
    {
        let state = self.state.clone();
        RerunLayer::new(
            self.recording.clone_weak(),
            state.clone(),
            Arc::from(self.source_id.as_str()),
            self.trace_writer.clone(),
            self.started,
            self.adapter.clone(),
        )
        .with_filter(SessionFilter::new(state))
    }

    /// Returns and clears a bounded-wait snapshot of the configured memory sink.
    ///
    /// Rerun's raw memory export uses an unbounded SDK flush, so the adapter runs it on the single
    /// lifecycle worker and waits only for `flush_timeout`. A timeout disables the session and
    /// returns `None`. `None` also indicates that this session has no memory sink.
    pub fn take_memory_snapshot_bounded(&self) -> Option<Vec<rerun::log::LogMsg>> {
        if !self.has_memory {
            return None;
        }
        match self
            .lifecycle
            .as_ref()
            .expect("lifecycle worker exists until drop")
            .snapshot(self.flush_timeout)
        {
            Ok(messages) => messages,
            Err(error) => {
                if error.disables_session() {
                    self.state.disable_on_error(&error);
                }
                None
            }
        }
    }

    /// Registers the immutable hierarchy already produced by builder lowering.
    ///
    /// Registration is synchronous and retains no reference to, or copy of, the runtime graph.
    /// Repeated calls merge compact lookup indexes because previously logged static records remain
    /// visible. Re-registering the same identity and path is idempotent; conflicting identities
    /// become ambiguous and are never used for exact causal reconstruction.
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
                    self.adapter
                        .registration
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .merge(index);
                }
            }
        }
    }

    /// Registers one already-lowered Enclave map without retaining it.
    ///
    /// Registrations merge with the same ambiguity rules as [`Self::register_runtime`].
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
            self.adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .merge(index);
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
        let timeout = self.flush_timeout;
        self.state.flush_once(|| {
            match self
                .lifecycle
                .as_ref()
                .expect("lifecycle worker exists until drop")
                .flush(timeout)
            {
                Err(LifecycleError::Busy | LifecycleError::Disabled) => Ok(()),
                result => result,
            }
        });
    }
}

fn sink_contains_grpc(config: &SinkConfig) -> bool {
    match config {
        SinkConfig::Grpc { .. } => true,
        SinkConfig::Tee(leaves) => leaves.iter().any(sink_contains_grpc),
        SinkConfig::Memory | SinkConfig::File(_) => false,
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
    let mut memory = None;
    let mut sinks: Vec<Box<dyn rerun::sink::LogSink>> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        match leaf {
            SinkConfig::Memory => {
                let sink = rerun::sink::MemorySink::new(recording.clone());
                memory = Some(sink.buffer());
                sinks.push(Box::new(sink));
            }
            SinkConfig::File(path) => sinks.push(Box::new(rerun::sink::FileSink::with_options(
                path,
                rerun::sink::FileSinkOptions {
                    write_footer: false,
                },
            )?)),
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
enum LifecycleError {
    #[error("Rerun lifecycle operation exceeded {0:?}")]
    Timeout(Duration),
    #[error("Rerun lifecycle worker disconnected")]
    Disconnected,
    #[error("a Rerun lifecycle operation is already pending")]
    Busy,
    #[error("the Rerun session is disabled")]
    Disabled,
    #[error(transparent)]
    Sink(#[from] rerun::sink::SinkFlushError),
}

impl LifecycleError {
    fn disables_session(&self) -> bool {
        !matches!(self, Self::Busy | Self::Disabled)
    }
}

enum LifecycleCommand {
    Flush {
        reply: std::sync::mpsc::SyncSender<Result<(), rerun::sink::SinkFlushError>>,
    },
    Snapshot {
        reply: std::sync::mpsc::SyncSender<Option<Vec<rerun::log::LogMsg>>>,
    },
    Shutdown {
        reply: std::sync::mpsc::SyncSender<Result<(), rerun::sink::SinkFlushError>>,
    },
}

struct LifecycleWorker {
    commands: std::sync::mpsc::SyncSender<LifecycleCommand>,
    pending: Arc<AtomicBool>,
    state: SessionState,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl LifecycleWorker {
    fn spawn(
        recording: RecordingStream,
        memory: Option<rerun::sink::MemorySinkStorage>,
        driver: Arc<dyn FlushDriver>,
        sdk_timeout: Duration,
        state: SessionState,
    ) -> Result<Self, std::io::Error> {
        let (commands, receiver) = std::sync::mpsc::sync_channel(1);
        let pending = Arc::new(AtomicBool::new(false));
        let worker_pending = pending.clone();
        let handle = std::thread::Builder::new()
            .name("boomerang-rerun-lifecycle".to_owned())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    match command {
                        LifecycleCommand::Flush { reply } => {
                            let _ = reply.send(driver.flush(&recording, sdk_timeout));
                        }
                        LifecycleCommand::Snapshot { reply } => {
                            let _ = reply.send(memory.as_ref().map(|storage| storage.take()));
                        }
                        LifecycleCommand::Shutdown { reply } => {
                            let flush = driver.flush(&recording, sdk_timeout);
                            recording.disconnect();
                            drop(memory);
                            let result = driver.teardown(recording, sdk_timeout).and(flush);
                            let _ = reply.send(result);
                            worker_pending.store(false, Ordering::Release);
                            return;
                        }
                    }
                    worker_pending.store(false, Ordering::Release);
                }
                recording.disconnect();
                drop(memory);
                drop(recording);
            })?;
        Ok(Self {
            commands,
            pending,
            state,
            handle: Some(handle),
        })
    }

    fn begin_submission(&self) -> Result<(), LifecycleError> {
        if !self.state.is_enabled() {
            return Err(LifecycleError::Disabled);
        }
        self.pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| LifecycleError::Busy)?;
        if !self.state.is_enabled() {
            self.pending.store(false, Ordering::Release);
            return Err(LifecycleError::Disabled);
        }
        Ok(())
    }

    fn submit(&self, command: LifecycleCommand) -> Result<(), LifecycleError> {
        self.begin_submission()?;
        self.commands.try_send(command).map_err(|error| {
            self.pending.store(false, Ordering::Release);
            match error {
                std::sync::mpsc::TrySendError::Full(_) => LifecycleError::Busy,
                std::sync::mpsc::TrySendError::Disconnected(_) => LifecycleError::Disconnected,
            }
        })
    }

    fn flush(&self, timeout: Duration) -> Result<(), LifecycleError> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        self.submit(LifecycleCommand::Flush { reply })?;
        receive_lifecycle_result(receiver, timeout)
    }

    fn snapshot(
        &self,
        timeout: Duration,
    ) -> Result<Option<Vec<rerun::log::LogMsg>>, LifecycleError> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        self.submit(LifecycleCommand::Snapshot { reply })?;
        receiver.recv_timeout(timeout).map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => LifecycleError::Timeout(timeout),
            std::sync::mpsc::RecvTimeoutError::Disconnected => LifecycleError::Disconnected,
        })
    }

    fn shutdown(mut self, timeout: Duration) -> Result<(), LifecycleError> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        self.submit(LifecycleCommand::Shutdown { reply })?;
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                // A reply is sent only after final sink teardown and the last strong recording
                // drop. Join even when the flush itself reported an error: cleanup completed.
                if let Some(handle) = self.handle.take() {
                    if handle.join().is_err() {
                        return Err(LifecycleError::Disconnected);
                    }
                }
                result.map_err(Into::into)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(LifecycleError::Timeout(timeout))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(LifecycleError::Disconnected)
            }
        }
    }
}

fn receive_lifecycle_result(
    receiver: std::sync::mpsc::Receiver<Result<(), rerun::sink::SinkFlushError>>,
    timeout: Duration,
) -> Result<(), LifecycleError> {
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(Into::into),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(LifecycleError::Timeout(timeout)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(LifecycleError::Disconnected),
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
        if let Some(lifecycle) = self.lifecycle.take() {
            if let Err(error) = lifecycle.shutdown(self.flush_timeout) {
                self.state.disable_on_error(&error);
            }
        }
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

    pub(super) fn is_enabled(&self) -> bool {
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
            tracing::callsite::rebuild_interest_cache();
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
