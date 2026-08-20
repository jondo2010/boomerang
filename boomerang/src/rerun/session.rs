use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rerun::{RecordingStream, RecordingStreamBuilder, RecordingStreamResult};

#[cfg(feature = "federated")]
use super::entities::{escape_entity_segment, log_runtime_relation, runtime_enclave_root};
use super::entities::{log_runtime_enclaves, RerunTraceWriter, TraceWriter};
use super::layer::RerunLayer;

const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Destination for a Rerun recording.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SinkConfig {
    /// Retain the recording in memory.
    #[default]
    Memory,
}

/// Configures a [`RerunSession`].
pub struct RerunSessionBuilder {
    application_id: String,
    source_id: Option<String>,
    sink: SinkConfig,
    flush_timeout: Duration,
    trace_writer: Arc<dyn TraceWriter>,
}

impl RerunSessionBuilder {
    /// Creates a builder for the given Rerun application ID.
    pub fn new(application_id: impl Into<String>) -> Self {
        Self {
            application_id: application_id.into(),
            source_id: None,
            sink: SinkConfig::default(),
            flush_timeout: DEFAULT_FLUSH_TIMEOUT,
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

    /// Sets the maximum time spent flushing on an explicit flush or drop.
    pub fn flush_timeout(mut self, flush_timeout: Duration) -> Self {
        self.flush_timeout = flush_timeout;
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
    pub fn build(self) -> RecordingStreamResult<RerunSession> {
        let (recording, memory) = match self.sink {
            SinkConfig::Memory => RecordingStreamBuilder::new(
                rerun::ApplicationId::new_or_unknown(self.application_id),
            )
            .memory()?,
        };
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
            state: SessionState::new(enabled),
            trace_writer: self.trace_writer,
            started: Instant::now(),
            next_trace_id: Arc::new(AtomicU64::new(0)),
        })
    }
}

/// An observational Rerun recording session.
pub struct RerunSession {
    recording: RecordingStream,
    memory: rerun::sink::MemorySinkStorage,
    source_id: String,
    flush_timeout: Duration,
    state: SessionState,
    trace_writer: Arc<dyn TraceWriter>,
    started: Instant,
    next_trace_id: Arc<AtomicU64>,
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
            self.recording.clone(),
            self.state.clone(),
            Arc::from(self.source_id.as_str()),
            self.trace_writer.clone(),
            self.started,
            self.next_trace_id.clone(),
        )
    }

    /// Access the backing memory sink for recording inspection or serialization.
    pub fn memory_sink(&self) -> &rerun::sink::MemorySinkStorage {
        &self.memory
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
                self.observe_registration(|| {
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
                        "/federation/topology/nodes",
                        &rerun::GraphNodes::new(federation_nodes.clone())
                            .with_labels(federation_nodes),
                    )?;
                    self.recording.log_static(
                        "/federation/topology/edges",
                        &rerun::GraphEdges::new(federation_edges)
                            .with_graph_type(rerun::components::GraphType::Directed),
                    )?;
                    Ok(())
                });
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
        self.observe_registration(|| log_runtime_enclaves(&self.recording, federate, enclaves));
    }

    fn observe_registration(
        &self,
        registration: impl FnOnce() -> rerun::RecordingStreamResult<()>,
    ) {
        if !self.state.try_begin_attempt() {
            return;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(registration)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.state.disable_on_error(&error),
            Err(_) => self
                .state
                .disable_on_error(&"runtime registration panicked"),
        }
    }

    /// Flushes pending data once, bounded by the configured timeout.
    pub fn flush(&self) {
        self.state
            .flush_once(|| self.recording.flush_with_timeout(self.flush_timeout));
    }
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
