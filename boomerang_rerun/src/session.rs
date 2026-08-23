use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rerun::external::re_log_encoding::Decodable as _;
use rerun::{RecordingStream, RecordingStreamBuilder};

#[cfg(feature = "federated")]
use super::entities::{
    bounded_fragment, escape_entity_segment, log_runtime_relation, runtime_display_label,
    runtime_enclave_root,
};
use super::entities::{log_runtime_enclaves, RegistrationSnapshot};
use super::layer::{AdapterState, RerunLayer, SessionFilter};
use tracing_subscriber::Layer as _;

const DEFAULT_FINISH_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors returned while creating a file-backed Rerun session.
#[derive(Debug, thiserror::Error)]
pub enum RerunSessionBuildError {
    /// Rerun could not construct the recording stream or file sink.
    #[error(transparent)]
    Recording(#[from] rerun::RecordingStreamError),
}

/// An error encountered while finalizing an offline Rerun recording.
#[derive(Debug, thiserror::Error)]
#[error("failed to finalize Rerun recording: {0}")]
pub struct RerunSessionFinishError(#[source] RerunSessionFinishErrorKind);

#[derive(Debug, thiserror::Error)]
enum RerunSessionFinishErrorKind {
    #[error("failed to spawn the Rerun finalizer: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("Rerun finalization exceeded {0:?}")]
    Timeout(Duration),
    #[error("Rerun finalizer disconnected")]
    Disconnected,
    #[error(transparent)]
    Flush(#[from] rerun::sink::SinkFlushError),
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

/// A file-backed Boomerang trace recording.
///
/// Construct sessions with [`Self::save`], register the already-lowered runtime before execution,
/// install [`Self::layer`] in the active subscriber, and call [`Self::finish`] to obtain a checked,
/// footer-bearing RRD file.
pub struct RerunSession {
    recording: Option<RecordingStream>,
    path: std::path::PathBuf,
    finish_timeout: Duration,
    state: SessionState,
    adapter: AdapterState,
}

impl RerunSession {
    /// Creates a file-backed session with Boomerang's default timeline-first blueprint.
    pub fn save(
        application_id: impl Into<String>,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, RerunSessionBuildError> {
        let path = path.into();
        let builder = RecordingStreamBuilder::new(rerun::ApplicationId::new_or_unknown(
            application_id.into(),
        ))
        .with_default_blueprint(default_blueprint());
        let recording = builder.save(path.clone())?;
        recording.set_log_tick_enabled(false);
        recording.set_log_time_enabled(true);
        let state = SessionState::new(recording.is_enabled());
        Ok(Self {
            recording: Some(recording),
            path,
            finish_timeout: DEFAULT_FINISH_TIMEOUT,
            state,
            adapter: AdapterState::default(),
        })
    }

    /// Returns a subscriber layer that writes Boomerang trace annotations into this session.
    pub fn layer<S>(&self) -> impl tracing_subscriber::Layer<S>
    where
        S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    {
        let state = self.state.clone();
        RerunLayer::new(
            self.recording().clone_weak(),
            state.clone(),
            self.adapter.clone(),
        )
        .with_filter(SessionFilter::new(state))
    }

    /// Logs the immutable hierarchy already produced by builder lowering.
    ///
    /// Registration is synchronous and retains no runtime graph. Call this before execution
    /// consumes the [`boomerang_builder::RuntimeAssembly`].
    pub fn register_runtime(&self, runtime: &boomerang_builder::RuntimeAssembly) {
        let registration = match &runtime.execution {
            boomerang_builder::RuntimeExecution::Local(enclaves) => {
                self.observe_registration(|| {
                    log_runtime_enclaves(self.recording(), None, enclaves)
                        .map(RegistrationSnapshot::local)
                })
            }
            #[cfg(feature = "federated")]
            boomerang_builder::RuntimeExecution::Federated(federation) => self
                .observe_registration(|| {
                    let mut registrations = std::collections::BTreeMap::new();
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
                        self.recording().log_static(path.as_str(), &entity)?;
                        for enclave in federate.enclaves().keys() {
                            let enclave_path = runtime_enclave_root(Some(id.as_str()), enclave);
                            let relation_index = federation_edges.len();
                            log_runtime_relation(
                                self.recording(),
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
                        let enclaves = log_runtime_enclaves(
                            self.recording(),
                            Some(id.as_str()),
                            federate.enclaves(),
                        )?;
                        registrations.insert(id.as_str().to_owned(), enclaves);
                    }

                    for (endpoint, source, target, delay) in federation.graph().endpoint_routes() {
                        let source =
                            format!("/federates/{}", escape_entity_segment(source.as_str()));
                        let target =
                            format!("/federates/{}", escape_entity_segment(target.as_str()));
                        log_runtime_relation(
                            self.recording(),
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
                    self.recording().log_static(
                        "/federation/topology",
                        &rerun::GraphNodes::new(federation_nodes).with_labels(federation_labels),
                    )?;
                    self.recording().log_static(
                        "/federation/topology",
                        &rerun::GraphEdges::new(federation_edges)
                            .with_graph_type(rerun::components::GraphType::Directed),
                    )?;
                    Ok(RegistrationSnapshot::federated(registrations))
                }),
        };
        if let Some(registration) = registration {
            *self
                .adapter
                .registration
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = registration;
        }
    }

    /// Finalizes the file and verifies that its RRD footer can be decoded.
    pub fn finish(mut self) -> Result<(), RerunSessionFinishError> {
        let recording = self.recording.take().expect("session recording is present");
        finalize_bounded(recording, self.path.clone(), self.finish_timeout)
            .map_err(RerunSessionFinishError)?;
        if let Some(error) = self.state.first_error() {
            return Err(RerunSessionFinishError(
                RerunSessionFinishErrorKind::PriorObservationalFailure(error),
            ));
        }
        Ok(())
    }

    fn recording(&self) -> &RecordingStream {
        self.recording
            .as_ref()
            .expect("session recording is present")
    }

    fn observe_registration<T>(
        &self,
        registration: impl FnOnce() -> rerun::RecordingStreamResult<T>,
    ) -> Option<T> {
        if !self.state.is_enabled() {
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
}

impl Drop for RerunSession {
    fn drop(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let timeout = self.finish_timeout;
        let _ = std::thread::Builder::new()
            .name("boomerang-rerun-drop".to_owned())
            .spawn(move || {
                let _ = recording.flush_with_timeout(timeout);
                recording.disconnect();
            });
    }
}

fn finalize_bounded(
    recording: RecordingStream,
    path: std::path::PathBuf,
    timeout: Duration,
) -> Result<(), RerunSessionFinishErrorKind> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let sdk_timeout = timeout / 2;
    std::thread::Builder::new()
        .name("boomerang-rerun-finalize".to_owned())
        .spawn(move || {
            let result = (|| {
                recording.flush_with_timeout(sdk_timeout)?;
                recording.disconnect();
                drop(recording);
                verify_rrd_footer(&path)
            })();
            let _ = sender.send(result);
        })
        .map_err(RerunSessionFinishErrorKind::Spawn)?;
    receiver
        .recv_timeout(timeout)
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => {
                RerunSessionFinishErrorKind::Timeout(timeout)
            }
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                RerunSessionFinishErrorKind::Disconnected
            }
        })?
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
        .with_contents(["/enclaves/**", "/federates/**", "/federation/**"]);
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

#[derive(Clone)]
pub(super) struct SessionState {
    inner: Arc<SessionStateInner>,
}

struct SessionStateInner {
    enabled: AtomicBool,
    first_error: std::sync::Mutex<Option<String>>,
}

impl SessionState {
    fn new(enabled: bool) -> Self {
        Self {
            inner: Arc::new(SessionStateInner {
                enabled: AtomicBool::new(enabled),
                first_error: std::sync::Mutex::new(None),
            }),
        }
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Acquire)
    }

    fn first_error(&self) -> Option<String> {
        self.inner
            .first_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn disable_on_error(&self, error: &dyn std::fmt::Display) {
        if self.inner.enabled.swap(false, Ordering::AcqRel) {
            *self
                .inner
                .first_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.to_string());
            tracing::callsite::rebuild_interest_cache();
            tracing::warn!(
                target: "boomerang::rerun_internal",
                %error,
                "disabling Rerun recording after an observational failure"
            );
        }
    }
}
