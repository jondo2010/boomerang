use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rerun::external::re_log_encoding::Decodable as _;
use rerun::{RecordingStream, RecordingStreamBuilder};

#[cfg(feature = "federated")]
use super::entities::{
    bounded_fragment, escape_entity_segment, log_runtime_relation, runtime_display_label,
    runtime_enclave_root,
};
use super::entities::{log_runtime_enclaves, RegistrationIndex};
use super::layer::{AdapterState, RerunLayer, SessionFilter};
use tracing_subscriber::Layer as _;

const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Destination for a Rerun recording.
///
/// Active memory, file, and tee sinks use Rerun 0.36.1's bounded batching pipeline. That does not
/// bound a memory sink, which retains the full recording. File sinks retain an O(chunks) footer
/// manifest while recording. When the pipeline is saturated, logging from [`RerunLayer`] may apply
/// backpressure to the scheduler callback. Boomerang adds no second dynamic-record queue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SinkConfig {
    /// Retain the full recording in memory; active memory use grows with the trace.
    #[default]
    Memory,
    /// Write a recording to a finalized standard RRD file.
    ///
    /// Rerun 0.36.1 retains manifest state for the lifetime of the recording and constructs the
    /// footer using O(chunks) memory during finalization. Call [`RerunSession::finish`] to surface
    /// finalization failures.
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

/// An error encountered while finalizing a Rerun recording session.
#[derive(Debug, thiserror::Error)]
#[error("failed to finalize Rerun recording: {0}")]
pub struct RerunSessionFinishError(#[source] RerunSessionFinishErrorKind);

impl From<LifecycleError> for RerunSessionFinishError {
    fn from(error: LifecycleError) -> Self {
        Self(RerunSessionFinishErrorKind::Lifecycle(error))
    }
}

impl From<RerunSessionFinishErrorKind> for RerunSessionFinishError {
    fn from(error: RerunSessionFinishErrorKind) -> Self {
        Self(error)
    }
}

#[derive(Debug, thiserror::Error)]
enum RerunSessionFinishErrorKind {
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError),
    #[error("failed to verify finalized RRD file {path}: {source}")]
    File {
        path: std::path::PathBuf,
        #[source]
        source: RrdFooterVerificationError,
    },
    #[error("recording was disabled after an observational failure: {0}")]
    PriorObservationalFailure(String),
}

#[derive(Debug, thiserror::Error)]
enum RrdFooterVerificationError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("missing or truncated RRD footer (file has {actual} bytes, requires {required})")]
    Truncated { actual: u64, required: usize },
    #[error("invalid RRD footer: {0}")]
    Invalid(#[source] rerun::external::re_log_encoding::CodecError),
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

#[cfg(test)]
type LifecycleReplyHook = Arc<dyn Fn() + Send + Sync>;
#[cfg(test)]
type LifecycleVerificationHook = Arc<dyn Fn() + Send + Sync>;

#[cfg(test)]
struct LifecycleHooks {
    reply: Option<LifecycleReplyHook>,
    verification: Option<LifecycleVerificationHook>,
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
    #[cfg(test)]
    lifecycle_reply_hook: Option<LifecycleReplyHook>,
    #[cfg(test)]
    lifecycle_verification_hook: Option<LifecycleVerificationHook>,
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
            #[cfg(test)]
            lifecycle_reply_hook: None,
            #[cfg(test)]
            lifecycle_verification_hook: None,
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

    /// Sets the maximum time spent on an explicit flush or final teardown and footer verification.
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

    #[cfg(test)]
    fn lifecycle_reply_hook(mut self, hook: LifecycleReplyHook) -> Self {
        self.lifecycle_reply_hook = Some(hook);
        self
    }

    #[cfg(test)]
    fn lifecycle_verification_hook(mut self, hook: LifecycleVerificationHook) -> Self {
        self.lifecycle_verification_hook = Some(hook);
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
        let file_paths = sink_file_paths(&sink);
        let mut builder =
            RecordingStreamBuilder::new(rerun::ApplicationId::new_or_unknown(self.application_id));
        builder = match self.blueprint {
            BlueprintConfig::Default => builder.with_default_blueprint(default_blueprint()),
            BlueprintConfig::None => builder,
            BlueprintConfig::Custom(blueprint) => builder.with_default_blueprint(*blueprint),
        };
        let (recording, initial_memory) = builder.memory()?;
        recording.set_log_tick_enabled(false);
        recording.set_log_time_enabled(true);
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
            file_paths,
            #[cfg(test)]
            LifecycleHooks {
                reply: self.lifecycle_reply_hook,
                verification: self.lifecycle_verification_hook,
            },
        )
        .map_err(RerunSessionBuildError::LifecycleWorker)?;

        Ok(RerunSession {
            recording: recording.clone_weak(),
            has_memory,
            source_id,
            flush_timeout: self.flush_timeout,
            lifecycle: Some(lifecycle),
            state,
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
                    let mut federation_labels = Vec::new();
                    let mut federation_edges = Vec::new();
                    for (id, federate) in federation.federates() {
                        let path = format!("/federates/{}", escape_entity_segment(id.as_str()));
                        federation_nodes.push(path.clone());
                        federation_labels.push(bounded_fragment(id.as_str(), 24));
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
                            federation_labels.push(runtime_display_label(
                                Some(id.as_str()),
                                &enclave.to_string(),
                                &enclave.to_string(),
                                &enclave.to_string(),
                            ));
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
                            .with_labels(federation_labels),
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
            self.lifecycle
                .as_ref()
                .expect("lifecycle worker exists until drop")
                .flush(timeout)
        });
    }

    /// Finalizes the recording and reports lifecycle, footer, or prior observational failures.
    pub fn finish(mut self) -> Result<(), RerunSessionFinishError> {
        if let Some(lifecycle) = self.lifecycle.take() {
            lifecycle.shutdown(self.flush_timeout)?;
        }
        if let Some(error) = self.state.first_error() {
            return Err(RerunSessionFinishError(
                RerunSessionFinishErrorKind::PriorObservationalFailure(error),
            ));
        }
        Ok(())
    }
}

fn verify_rrd_footer(path: &std::path::Path) -> Result<(), RerunSessionFinishErrorKind> {
    use std::io::{Read as _, Seek as _};

    fn file_error(
        path: &std::path::Path,
        source: impl Into<RrdFooterVerificationError>,
    ) -> RerunSessionFinishErrorKind {
        RerunSessionFinishErrorKind::File {
            path: path.to_owned(),
            source: source.into(),
        }
    }

    let mut file = std::fs::File::open(path).map_err(|error| file_error(path, error))?;
    let file_len = file
        .metadata()
        .map_err(|error| file_error(path, error))?
        .len();
    let footer_size = rerun::external::re_log_encoding::StreamFooter::ENCODED_SIZE_BYTES;
    if file_len < footer_size as u64 {
        return Err(file_error(
            path,
            RrdFooterVerificationError::Truncated {
                actual: file_len,
                required: footer_size,
            },
        ));
    }
    file.seek(std::io::SeekFrom::End(-(footer_size as i64)))
        .map_err(|error| file_error(path, error))?;
    let mut footer = vec![0; footer_size];
    file.read_exact(&mut footer)
        .map_err(|error| file_error(path, error))?;
    rerun::external::re_log_encoding::StreamFooter::from_rrd_bytes(&footer)
        .map_err(|error| file_error(path, RrdFooterVerificationError::Invalid(error)))?;
    Ok(())
}

fn sink_contains_grpc(config: &SinkConfig) -> bool {
    match config {
        SinkConfig::Grpc { .. } => true,
        SinkConfig::Tee(leaves) => leaves.iter().any(sink_contains_grpc),
        SinkConfig::Memory | SinkConfig::File(_) => false,
    }
}

fn sink_file_paths(config: &SinkConfig) -> Vec<std::path::PathBuf> {
    match config {
        SinkConfig::File(path) => vec![path.clone()],
        SinkConfig::Tee(leaves) => leaves.iter().flat_map(sink_file_paths).collect(),
        SinkConfig::Memory | SinkConfig::Grpc { .. } => Vec::new(),
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
                rerun::sink::FileSinkOptions { write_footer: true },
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
        reply: std::sync::mpsc::SyncSender<Result<(), RerunSessionFinishErrorKind>>,
    },
}

struct LifecycleWorker {
    commands: std::sync::mpsc::SyncSender<LifecycleCommand>,
    pending: Arc<AtomicBool>,
    state: SessionState,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct PendingReset<'a> {
    pending: &'a AtomicBool,
    armed: bool,
}

impl<'a> PendingReset<'a> {
    fn new(pending: &'a AtomicBool) -> Self {
        Self {
            pending,
            armed: true,
        }
    }

    fn release(&mut self) {
        if self.armed {
            self.pending.store(false, Ordering::Release);
            self.armed = false;
        }
    }
}

impl Drop for PendingReset<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

impl LifecycleWorker {
    fn spawn(
        recording: RecordingStream,
        memory: Option<rerun::sink::MemorySinkStorage>,
        driver: Arc<dyn FlushDriver>,
        sdk_timeout: Duration,
        state: SessionState,
        file_paths: Vec<std::path::PathBuf>,
        #[cfg(test)] hooks: LifecycleHooks,
    ) -> Result<Self, std::io::Error> {
        let (commands, receiver) = std::sync::mpsc::sync_channel(1);
        let pending = Arc::new(AtomicBool::new(false));
        let worker_pending = pending.clone();
        let handle = std::thread::Builder::new()
            .name("boomerang-rerun-lifecycle".to_owned())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    let mut pending_reset = PendingReset::new(&worker_pending);
                    match command {
                        LifecycleCommand::Flush { reply } => {
                            let result = driver.flush(&recording, sdk_timeout);
                            pending_reset.release();
                            let _ = reply.send(result);
                            #[cfg(test)]
                            if let Some(hook) = hooks.reply.as_ref() {
                                hook();
                            }
                        }
                        LifecycleCommand::Snapshot { reply } => {
                            let result = memory.as_ref().map(|storage| storage.take());
                            pending_reset.release();
                            let _ = reply.send(result);
                            #[cfg(test)]
                            if let Some(hook) = hooks.reply.as_ref() {
                                hook();
                            }
                        }
                        LifecycleCommand::Shutdown { reply } => {
                            let flush = driver.flush(&recording, sdk_timeout);
                            recording.disconnect();
                            drop(memory);
                            let result = driver
                                .teardown(recording, sdk_timeout)
                                .and(flush)
                                .map_err(|error| {
                                    RerunSessionFinishErrorKind::Lifecycle(error.into())
                                })
                                .and_then(|()| {
                                    #[cfg(test)]
                                    if let Some(hook) = hooks.verification.as_ref() {
                                        hook();
                                    }
                                    for path in &file_paths {
                                        verify_rrd_footer(path)?;
                                    }
                                    Ok(())
                                });
                            pending_reset.release();
                            let _ = reply.send(result);
                            #[cfg(test)]
                            if let Some(hook) = hooks.reply.as_ref() {
                                hook();
                            }
                            return;
                        }
                    }
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
        self.begin_pending()?;
        if !self.state.is_enabled() {
            self.pending.store(false, Ordering::Release);
            return Err(LifecycleError::Disabled);
        }
        Ok(())
    }

    fn begin_pending(&self) -> Result<(), LifecycleError> {
        self.pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| LifecycleError::Busy)?;
        Ok(())
    }

    fn submit_admitted(&self, command: LifecycleCommand) -> Result<(), LifecycleError> {
        self.commands.try_send(command).map_err(|error| {
            self.pending.store(false, Ordering::Release);
            match error {
                std::sync::mpsc::TrySendError::Full(_) => LifecycleError::Busy,
                std::sync::mpsc::TrySendError::Disconnected(_) => LifecycleError::Disconnected,
            }
        })
    }

    fn submit(&self, command: LifecycleCommand) -> Result<(), LifecycleError> {
        self.begin_submission()?;
        self.submit_admitted(command)
    }

    fn submit_shutdown(&self, command: LifecycleCommand) -> Result<(), LifecycleError> {
        self.begin_pending()?;
        self.submit_admitted(command)
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

    fn shutdown(mut self, timeout: Duration) -> Result<(), RerunSessionFinishErrorKind> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        self.submit_shutdown(LifecycleCommand::Shutdown { reply })
            .map_err(RerunSessionFinishErrorKind::from)?;
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                // A reply is sent only after final sink teardown, the last strong recording drop,
                // and footer verification. Join even when finalization reported an error: the
                // worker completed its bounded shutdown path.
                if let Some(handle) = self.handle.take() {
                    if handle.join().is_err() {
                        return Err(LifecycleError::Disconnected.into());
                    }
                }
                result
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(LifecycleError::Timeout(timeout).into())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(LifecycleError::Disconnected.into())
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
    let scheduler = StateTimelineView::new("Scheduler phase spans (wall clock)")
        .with_origin("/")
        .with_contents(roots);
    let events = DataframeView::new("Event records")
        .with_origin("/")
        .with_contents(roots);
    let topology = GraphView::new("Ownership and propagation")
        .with_origin("/")
        .with_contents([
            "/enclaves/**",
            "/federates/**",
            "/federation/**",
            "/propagation/**",
        ]);
    let diagnostics = TextLogView::new("Diagnostics")
        .with_origin("/diagnostics")
        .with_contents(["/diagnostics/**"]);
    let measures = TimeSeriesView::new("Logical phases and measures")
        .with_origin("/")
        .with_contents(roots);

    Blueprint::new(
        Grid::new([
            scheduler.into(),
            events.into(),
            topology.into(),
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
    first_error: std::sync::Mutex<Option<String>>,
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
                first_error: std::sync::Mutex::new(None),
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

    fn first_error(&self) -> Option<String> {
        self.inner
            .first_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn try_begin_attempt(&self) -> bool {
        if self.is_enabled() {
            true
        } else {
            self.inner.skipped_count.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn flush_once(&self, flush: impl FnOnce() -> Result<(), LifecycleError>) {
        if self.inner.flushed.swap(true, Ordering::AcqRel) {
            return;
        }

        match flush() {
            Ok(()) | Err(LifecycleError::Disabled) => {}
            Err(LifecycleError::Busy) => self.inner.flushed.store(false, Ordering::Release),
            Err(error) => self.disable_on_error(&error),
        }
    }

    pub(super) fn disable_on_error(&self, error: &dyn std::fmt::Display) {
        let disabled = {
            let mut first_error = self
                .inner
                .first_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.inner.enabled.swap(false, Ordering::AcqRel) {
                *first_error = Some(error.to_string());
                true
            } else {
                false
            }
        };
        if disabled {
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::{Duration, Instant};

    use super::{
        BlueprintConfig, FlushDriver, LifecycleError, RerunSessionBuilder, SessionState, SinkConfig,
    };

    struct PanickingFlush;

    impl FlushDriver for PanickingFlush {
        fn flush(
            &self,
            _recording: &rerun::RecordingStream,
            _timeout: Duration,
        ) -> Result<(), rerun::sink::SinkFlushError> {
            panic!("injected lifecycle panic")
        }
    }

    struct CountingGatedFlush {
        calls: AtomicUsize,
        entered: Barrier,
        release: Barrier,
    }

    impl CountingGatedFlush {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                entered: Barrier::new(2),
                release: Barrier::new(2),
            }
        }
    }

    impl FlushDriver for CountingGatedFlush {
        fn flush(
            &self,
            _recording: &rerun::RecordingStream,
            _timeout: Duration,
        ) -> Result<(), rerun::sink::SinkFlushError> {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                self.entered.wait();
                self.release.wait();
            }
            Ok(())
        }
    }

    struct TeardownCount(Arc<AtomicUsize>);

    impl FlushDriver for TeardownCount {
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
            self.0.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[test]
    fn first_failure_disables_once_and_later_attempts_are_skipped() {
        let state = SessionState::default();
        let shared_state = state.clone();

        assert!(state.try_begin_attempt());
        state.disable_on_error(&"injected recording failure");
        assert!(!state.is_enabled());
        assert_eq!(state.error_count(), 1);
        assert_eq!(state.skipped_count(), 0);
        assert_eq!(
            state.first_error().as_deref(),
            Some("injected recording failure")
        );

        assert!(!shared_state.try_begin_attempt());
        assert!(!shared_state.try_begin_attempt());
        state.disable_on_error(&"later concurrent failure");
        assert_eq!(state.error_count(), 1);
        assert_eq!(state.skipped_count(), 2);
        assert_eq!(
            state.first_error().as_deref(),
            Some("injected recording failure")
        );
    }

    #[test]
    fn flush_once_is_idempotent() {
        let state = SessionState::default();
        let mut flushes = 0;

        state.flush_once(|| {
            flushes += 1;
            Ok(())
        });
        state.flush_once(|| {
            flushes += 1;
            Ok(())
        });

        assert_eq!(flushes, 1);
    }

    #[test]
    fn finish_without_lifecycle_is_ok() {
        let mut session = RerunSessionBuilder::new("already-finalized")
            .blueprint(BlueprintConfig::None)
            .build()
            .unwrap();
        session
            .lifecycle
            .take()
            .unwrap()
            .shutdown(session.flush_timeout)
            .unwrap();

        session.finish().unwrap();
    }

    #[test]
    fn footer_verification_is_bounded_by_shutdown_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let session = RerunSessionBuilder::new("bounded-footer-verification")
            .sink(SinkConfig::File(directory.path().join("bounded.rrd")))
            .blueprint(BlueprintConfig::None)
            .flush_timeout(Duration::from_millis(10))
            .lifecycle_verification_hook(Arc::new({
                let entered = entered.clone();
                let release = release.clone();
                move || {
                    entered.wait();
                    release.wait();
                }
            }))
            .build()
            .unwrap();
        let releasing = std::thread::spawn(move || {
            entered.wait();
            std::thread::sleep(Duration::from_millis(100));
            release.wait();
        });
        let started = Instant::now();

        let error = session.finish().unwrap_err();

        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(error.to_string().contains("exceeded 10ms"));
        releasing.join().unwrap();
    }

    #[test]
    fn worker_panic_releases_admission_and_disables_once_on_disconnect() {
        let session = RerunSessionBuilder::new("panicking-lifecycle-driver")
            .blueprint(BlueprintConfig::None)
            .flush_timeout(Duration::from_secs(1))
            .flush_driver(Arc::new(PanickingFlush))
            .build()
            .unwrap();
        let worker = session.lifecycle.as_ref().unwrap();

        let first = worker.flush(Duration::from_secs(1)).unwrap_err();
        assert!(matches!(first, LifecycleError::Disconnected));
        let next = worker.snapshot(Duration::from_secs(1)).unwrap_err();
        assert!(matches!(next, LifecycleError::Disconnected));

        session.state.disable_on_error(&first);
        session.state.disable_on_error(&next);
        assert!(!session.is_enabled());
        assert_eq!(session.error_count(), 1);
    }

    #[test]
    fn reply_releases_admission_before_immediate_drop_runs_teardown() {
        let teardown_count = Arc::new(AtomicUsize::new(0));
        let first_reply = Arc::new(AtomicBool::new(true));
        let hook_entered = Arc::new(Barrier::new(2));
        let hook_release = Arc::new(Barrier::new(2));
        let session = RerunSessionBuilder::new("reply-before-drop")
            .blueprint(BlueprintConfig::None)
            .flush_timeout(Duration::from_secs(1))
            .flush_driver(Arc::new(TeardownCount(teardown_count.clone())))
            .lifecycle_reply_hook(Arc::new({
                let first_reply = first_reply.clone();
                let hook_entered = hook_entered.clone();
                let hook_release = hook_release.clone();
                move || {
                    if first_reply.swap(false, Ordering::AcqRel) {
                        hook_entered.wait();
                        hook_release.wait();
                    }
                }
            }))
            .build()
            .unwrap();

        assert!(session.take_memory_snapshot_bounded().is_some());
        hook_entered.wait();
        let pending = session.lifecycle.as_ref().unwrap().pending.clone();
        let snapshot_admission_released = !pending.load(Ordering::Acquire);
        if !snapshot_admission_released {
            hook_release.wait();
        }
        assert!(
            snapshot_admission_released,
            "snapshot admission must clear before its reply is observable"
        );

        let (dropped, drop_complete) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            drop(session);
            let _ = dropped.send(());
        });

        let started = Instant::now();
        while !pending.load(Ordering::Acquire) {
            if started.elapsed() >= Duration::from_secs(1) {
                hook_release.wait();
                panic!("shutdown was not admitted while the worker hook was paused");
            }
            std::thread::yield_now();
        }
        hook_release.wait();
        drop_complete.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(teardown_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn busy_flush_can_be_retried_after_lifecycle_operation_completes() {
        let driver = Arc::new(CountingGatedFlush::new());
        let session = Arc::new(
            RerunSessionBuilder::new("retry-busy-flush")
                .blueprint(BlueprintConfig::None)
                .flush_timeout(Duration::from_secs(1))
                .flush_driver(driver.clone())
                .build()
                .unwrap(),
        );
        let in_flight = {
            let session = session.clone();
            std::thread::spawn(move || {
                session
                    .lifecycle
                    .as_ref()
                    .unwrap()
                    .flush(Duration::from_secs(1))
                    .unwrap();
            })
        };
        driver.entered.wait();

        session.flush();
        assert!(session.is_enabled());
        driver.release.wait();
        in_flight.join().unwrap();

        session.flush();
        session.flush();
        assert_eq!(driver.calls.load(Ordering::Acquire), 2);
    }
}
