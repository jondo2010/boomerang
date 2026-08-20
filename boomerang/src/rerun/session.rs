use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rerun::{RecordingStream, RecordingStreamBuilder, RecordingStreamResult};

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
#[derive(Debug)]
pub struct RerunSessionBuilder {
    application_id: String,
    source_id: Option<String>,
    sink: SinkConfig,
    flush_timeout: Duration,
}

impl RerunSessionBuilder {
    /// Creates a builder for the given Rerun application ID.
    pub fn new(application_id: impl Into<String>) -> Self {
        Self {
            application_id: application_id.into(),
            source_id: None,
            sink: SinkConfig::default(),
            flush_timeout: DEFAULT_FLUSH_TIMEOUT,
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
            _memory: memory,
            source_id,
            flush_timeout: self.flush_timeout,
            state: SessionState::new(enabled),
        })
    }
}

/// An observational Rerun recording session.
pub struct RerunSession {
    recording: RecordingStream,
    _memory: rerun::sink::MemorySinkStorage,
    source_id: String,
    flush_timeout: Duration,
    state: SessionState,
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

    /// Flushes pending data once, bounded by the configured timeout.
    pub fn flush(&self) {
        self.state
            .flush_once(|| self.recording.flush_with_timeout(self.flush_timeout));
    }

    #[expect(dead_code, reason = "used by the sibling tracing layer")]
    pub(super) fn recording_parts(&self) -> (&RecordingStream, SessionState) {
        (&self.recording, self.state.clone())
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

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the sibling tracing layer")
    )]
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
