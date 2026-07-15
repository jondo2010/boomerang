use crate::{
    event::AsyncEvent, CommonContext, Duration, EnclaveKey, RuntimeError, SendContext, Tag,
};

/// Failure while coordinating logical time with a local upstream enclave.
#[derive(Debug, thiserror::Error)]
pub enum LogicalTimeBarrierError {
    #[error("scheduler event channel closed while waiting for upstream enclave {upstream}")]
    EventChannelClosed { upstream: EnclaveKey },
}

/// Result of waiting for permission to process a logical tag.
#[derive(Debug)]
pub enum CoordinationOutcome {
    /// The requested tag may be processed.
    Granted,
    /// An asynchronous event must be handled before requesting the tag again.
    Interrupted(AsyncEvent),
}

/// Protocol-free error returned by an external logical-time coordinator.
#[derive(Debug, thiserror::Error)]
#[error("logical-time coordination failed: {source}")]
pub struct CoordinationError {
    #[source]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl CoordinationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            source: Box::new(CoordinationMessage(message.into())),
        }
    }

    /// Preserve a concrete coordination error as this protocol-free error's source.
    pub fn from_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct CoordinationMessage(String);

impl From<String> for CoordinationError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// Optional protocol-free hook for external logical-time coordination.
///
/// Implementations can block until the requested tag is granted, or return an
/// inbound event that the scheduler should handle before trying to advance.
pub trait LogicalTimeCoordinator: Send {
    /// Acquire permission to advance to `tag`.
    ///
    /// A returned [`CoordinationOutcome`] explicitly distinguishes a grant
    /// from an asynchronous interruption. Coordination failures are terminal.
    fn acquire(
        &mut self,
        tag: Tag,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<CoordinationOutcome, CoordinationError>;

    /// Report that all work for `tag` has completed.
    fn complete(&mut self, tag: Tag) -> Result<(), CoordinationError>;
}

/// Coordinator used when a scheduler has no external logical-time authority.
#[derive(Debug, Default)]
pub struct NoopLogicalTimeCoordinator;

impl LogicalTimeCoordinator for NoopLogicalTimeCoordinator {
    fn acquire(
        &mut self,
        _tag: Tag,
        _event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<CoordinationOutcome, CoordinationError> {
        Ok(CoordinationOutcome::Granted)
    }

    fn complete(&mut self, _tag: Tag) -> Result<(), CoordinationError> {
        Ok(())
    }
}

impl std::fmt::Debug for dyn LogicalTimeCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn LogicalTimeCoordinator")
    }
}

#[derive(Debug)]
pub(super) struct LogicalTimeBarrier {
    /// The last released tag
    pub(super) released_tag: Tag,
    /// The greatest upstream tag for which a provisional release is outstanding.
    ///
    /// An unchanged or weaker acquire request reuses this watermark instead of
    /// sending duplicate coordination traffic. A stronger request advances it,
    /// and an actual release at or beyond it clears it. An intermediate release
    /// advances known progress without retiring the stronger outstanding request.
    /// A failed upstream send does not establish the watermark.
    pub(super) provisional_tag: Tag,
    /// The send context for the upstream enclave
    pub(super) upstream_ctx: SendContext,
    /// Optional delay for the upstream connection
    pub(super) upstream_delay: Option<Duration>,
}

impl LogicalTimeBarrier {
    #[tracing::instrument(skip(self), fields(tag = %tag, released = %self.released_tag))]
    pub(super) fn release_tag(&mut self, tag: Tag) {
        tracing::trace!("Release");

        if tag < self.released_tag {
            tracing::warn!(
                "Cannot release a tag ({tag}) earlier than the last released tag {}",
                self.released_tag
            );
        } else {
            self.released_tag = tag;
        }

        // Only progress sufficient for the outstanding request retires it.
        if self.provisional_tag <= self.released_tag {
            self.provisional_tag = Tag::NEVER;
        }
    }

    pub(super) fn release_tag_provisional(&mut self, tag: Tag) {
        if tag <= self.provisional_tag {
            self.release_tag(tag);
        }
    }

    #[inline]
    /// Try to acquire the given tag without blocking.
    pub(super) fn try_acquire_tag(&mut self, tag: Tag) -> bool {
        tag <= self.released_tag
    }

    /// Acquire the given tag, blocking until it is released, or an [`AsyncEvent`] is received.
    ///
    /// If an async event is received, it is returned to the caller. A return value of `None` indicates that the tag has been released.
    #[inline]
    #[tracing::instrument(skip(self, tag, this_enclave, event_rx), fields(tag = %tag))]
    pub(super) fn acquire_tag(
        &mut self,
        tag: Tag,
        this_enclave: EnclaveKey,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<Option<AsyncEvent>, LogicalTimeBarrierError> {
        // Since this is a delayed connection, we can go back in time and need to
        // acquire the latest upstream tag that can create an event at the given
        // tag.
        let upstream_tag = if let Some(delay) = self.upstream_delay {
            tag.pre(delay)
        } else {
            tag
        };

        tracing::trace!(upstream_tag = %upstream_tag, "Try acquire");
        if self.try_acquire_tag(upstream_tag) {
            return Ok(None);
        }

        if upstream_tag > self.provisional_tag {
            tracing::trace!(%upstream_tag, "Releasing provisional tag");
            if !self
                .upstream_ctx
                .release_provisional(this_enclave, upstream_tag)
            {
                // The upstream has terminated try to return a queued event here. If the upstream terminated, we probably
                // have an event queued from it. This prevents pre-mature termination of this enclave.
                tracing::warn!("Upstream has terminated");
                return event_rx.try_recv().map_err(|_| {
                    LogicalTimeBarrierError::EventChannelClosed {
                        upstream: self.upstream_ctx.enclave_id(),
                    }
                });
            }
            self.provisional_tag = upstream_tag;
        }

        // Block until the tag is released
        tracing::trace!("Blocking");
        event_rx
            .recv()
            .map(Some)
            .map_err(|_| LogicalTimeBarrierError::EventChannelClosed {
                upstream: self.upstream_ctx.enclave_id(),
            })
    }
}

/// Result of offering an asynchronous event to scheduler coordination.
pub(super) enum CoordinationEventResult {
    /// The event updated coordination state and needs no further handling.
    Consumed,
    /// The scheduler must continue handling this event normally.
    Event(AsyncEvent),
}

/// Scheduler-owned composition of local barriers and one optional external coordinator.
///
/// Local upstream acquisition always precedes external acquisition. Local downstream release
/// always precedes external completion. The external coordinator remains protocol-free; RTI and
/// transport adapters live outside `boomerang_runtime`.
#[derive(Debug)]
pub(super) struct SchedulerCoordination {
    enclave: EnclaveKey,
    upstream: tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
    downstream: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    external: Box<dyn LogicalTimeCoordinator>,
}

impl SchedulerCoordination {
    pub(super) fn new(
        enclave: EnclaveKey,
        upstream: tinymap::TinySecondaryMap<EnclaveKey, LogicalTimeBarrier>,
        downstream: tinymap::TinySecondaryMap<EnclaveKey, SendContext>,
    ) -> Self {
        Self {
            enclave,
            upstream,
            downstream,
            external: Box::new(NoopLogicalTimeCoordinator),
        }
    }

    pub(super) fn set_external(&mut self, coordinator: impl LogicalTimeCoordinator + 'static) {
        self.external = Box::new(coordinator);
    }

    pub(super) fn observe_event(
        &mut self,
        event: AsyncEvent,
        current_tag: Tag,
    ) -> CoordinationEventResult {
        match event {
            AsyncEvent::TagRelease { enclave, tag } => {
                self.upstream
                    .get_mut(enclave)
                    .expect("Unknown upstream enclave")
                    .release_tag(tag);
                CoordinationEventResult::Consumed
            }
            AsyncEvent::TagReleaseProvisional { enclave, tag } => {
                if tag > current_tag {
                    // In a local cycle, the requesting downstream may also be an upstream.
                    if let Some(barrier) = self.upstream.get_mut(enclave) {
                        barrier.release_tag_provisional(tag);
                    }
                }
                CoordinationEventResult::Event(AsyncEvent::TagReleaseProvisional { enclave, tag })
            }
            event => CoordinationEventResult::Event(event),
        }
    }

    pub(super) fn acquire(
        &mut self,
        tag: Tag,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<CoordinationOutcome, RuntimeError> {
        for (_upstream, barrier) in self.upstream.iter_mut() {
            if let Some(event) = barrier.acquire_tag(tag, self.enclave, event_rx)? {
                return Ok(CoordinationOutcome::Interrupted(event));
            }
        }
        self.external.acquire(tag, event_rx).map_err(Into::into)
    }

    pub(super) fn release_downstream(&self, tag: Tag, shutting_down: bool) {
        for (downstream, context) in self.downstream.iter() {
            let event = AsyncEvent::release(self.enclave, tag);
            tracing::trace!(%downstream, event = %event, "Releasing downstream");
            if !context.schedule_external(event) && !shutting_down {
                tracing::warn!(
                    "Failed to send tag downstream, downstream has unexpectedly terminated."
                );
            }
        }
    }

    pub(super) fn complete(
        &mut self,
        tag: Tag,
        shutting_down: bool,
    ) -> Result<(), CoordinationError> {
        self.release_downstream(tag, shutting_down);
        self.external.complete(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keepalive;

    fn queue_interruption(event_tx: &crate::Sender<AsyncEvent>) {
        event_tx.send(AsyncEvent::shutdown(Duration::ZERO)).unwrap();
    }

    fn assert_provisional_request(upstream_rx: &crate::Receiver<AsyncEvent>, expected_tag: Tag) {
        assert!(matches!(
            upstream_rx.try_recv().unwrap(),
            Some(AsyncEvent::TagReleaseProvisional { enclave, tag })
                if enclave == EnclaveKey::from(0) && tag == expected_tag
        ));
    }

    #[test]
    fn local_barrier_reports_closed_scheduler_event_channel_without_panicking() {
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        drop(upstream_rx);
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let upstream = EnclaveKey::from(1);
        let mut barrier = LogicalTimeBarrier {
            released_tag: Tag::NEVER,
            provisional_tag: Tag::NEVER,
            upstream_ctx: SendContext {
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx,
            },
            upstream_delay: None,
        };
        let (event_tx, event_rx) = kanal::unbounded();
        drop(event_tx);

        assert!(matches!(
            barrier.acquire_tag(Tag::ZERO, EnclaveKey::from(0), &event_rx),
            Err(LogicalTimeBarrierError::EventChannelClosed { upstream: observed })
                if observed == upstream
        ));
    }

    #[test]
    fn local_barrier_suppresses_repeated_provisional_requests_until_release() {
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let mut barrier = LogicalTimeBarrier {
            released_tag: Tag::NEVER,
            provisional_tag: Tag::NEVER,
            upstream_ctx: SendContext {
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            upstream_delay: None,
        };
        let (event_tx, event_rx) = kanal::unbounded();
        let this_enclave = EnclaveKey::from(0);
        let first = Tag::new(Duration::seconds(1), 0);
        let later = Tag::new(Duration::seconds(2), 0);
        let after_release = Tag::new(Duration::seconds(3), 0);

        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(first, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, first);

        for repeated_or_weaker in [first, Tag::ZERO] {
            queue_interruption(&event_tx);
            assert!(barrier
                .acquire_tag(repeated_or_weaker, this_enclave, &event_rx)
                .unwrap()
                .is_some());
            assert!(upstream_rx.try_recv().unwrap().is_none());
        }

        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(later, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, later);

        barrier.release_tag(later);
        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(after_release, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, after_release);
    }

    #[test]
    fn local_barrier_preserves_pending_request_across_insufficient_release() {
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let mut barrier = LogicalTimeBarrier {
            released_tag: Tag::NEVER,
            provisional_tag: Tag::NEVER,
            upstream_ctx: SendContext {
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            upstream_delay: None,
        };
        let (event_tx, event_rx) = kanal::unbounded();
        let this_enclave = EnclaveKey::from(0);
        let intermediate = Tag::new(Duration::seconds(1), 0);
        let requested = Tag::new(Duration::seconds(2), 0);

        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(requested, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, requested);

        barrier.release_tag(intermediate);
        assert_eq!(barrier.released_tag, intermediate);
        assert_eq!(barrier.provisional_tag, requested);

        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(requested, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert!(upstream_rx.try_recv().unwrap().is_none());

        barrier.release_tag(requested);
        assert_eq!(barrier.provisional_tag, Tag::NEVER);
        assert!(barrier
            .acquire_tag(requested, this_enclave, &event_rx)
            .unwrap()
            .is_none());
        assert!(upstream_rx.try_recv().unwrap().is_none());
    }

    #[test]
    fn local_barrier_release_progress_is_monotonic() {
        let (upstream_tx, _upstream_rx) = kanal::unbounded();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let released = Tag::new(Duration::seconds(2), 0);
        let stale = Tag::new(Duration::seconds(1), 0);
        let mut barrier = LogicalTimeBarrier {
            released_tag: Tag::NEVER,
            provisional_tag: Tag::NEVER,
            upstream_ctx: SendContext {
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            upstream_delay: None,
        };

        barrier.release_tag(released);
        barrier.release_tag(stale);

        assert_eq!(barrier.released_tag, released);
    }

    #[test]
    fn coordination_error_preserves_concrete_source() {
        #[derive(Debug, thiserror::Error)]
        #[error("concrete coordination failure")]
        struct ConcreteError;

        let error = CoordinationError::from_error(ConcreteError);
        let source = std::error::Error::source(&error).expect("source should be preserved");

        assert_eq!(source.to_string(), "concrete coordination failure");
    }
}
