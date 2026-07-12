use crate::{event::AsyncEvent, CommonContext, Duration, EnclaveKey, SendContext, Tag};

/// Failure while coordinating logical time with a local upstream enclave.
#[derive(Debug, thiserror::Error)]
pub enum LogicalTimeBarrierError {
    #[error("scheduler event channel closed while waiting for upstream enclave {upstream}")]
    EventChannelClosed { upstream: EnclaveKey },
}

/// Result of waiting for federated permission to process a logical tag.
#[cfg(feature = "federated")]
#[derive(Debug)]
pub enum FederatedBarrierOutcome {
    /// The requested tag may be processed.
    Granted,
    /// An asynchronous event must be handled before requesting the tag again.
    Interrupted(AsyncEvent),
}

/// Protocol-free error returned by a federated scheduler barrier.
#[cfg(feature = "federated")]
#[derive(Debug, thiserror::Error)]
#[error("federated coordination failed: {source}")]
pub struct FederatedBarrierError {
    #[source]
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

#[cfg(feature = "federated")]
impl FederatedBarrierError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            source: Box::new(FederatedBarrierMessage(message.into())),
        }
    }

    /// Preserve a concrete coordination error as this protocol-free error's source.
    pub fn from_error(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }
}

#[cfg(feature = "federated")]
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct FederatedBarrierMessage(String);

#[cfg(feature = "federated")]
impl From<String> for FederatedBarrierError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// Optional scheduler hook for federated logical-time coordination.
///
/// Implementations can block until the requested tag is granted, or return an
/// inbound event that the scheduler should handle before trying to advance.
#[cfg(feature = "federated")]
pub trait FederatedTimeBarrier: Send {
    /// Acquire permission to advance to `tag`.
    ///
    /// A returned [`FederatedBarrierOutcome`] explicitly distinguishes a grant
    /// from an asynchronous interruption. Coordination failures are terminal.
    fn acquire_tag(
        &mut self,
        tag: Tag,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<FederatedBarrierOutcome, FederatedBarrierError>;

    /// Report that all work for `tag` has completed.
    fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederatedBarrierError>;
}

#[cfg(feature = "federated")]
pub(super) struct NoFederatedTimeBarrier;

#[cfg(feature = "federated")]
impl FederatedTimeBarrier for NoFederatedTimeBarrier {
    fn acquire_tag(
        &mut self,
        _tag: Tag,
        _event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<FederatedBarrierOutcome, FederatedBarrierError> {
        Ok(FederatedBarrierOutcome::Granted)
    }

    fn logical_tag_complete(&mut self, _tag: Tag) -> Result<(), FederatedBarrierError> {
        Ok(())
    }
}

#[cfg(feature = "federated")]
impl std::fmt::Debug for dyn FederatedTimeBarrier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn FederatedTimeBarrier")
    }
}

#[derive(Debug)]
pub(super) struct LogicalTimeBarrier {
    /// The last released tag
    pub(super) released_tag: Tag,
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
        }
        self.released_tag = tag;
        // Reset the provisional tag
        self.provisional_tag = Tag::NEVER;
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

    #[cfg(feature = "federated")]
    #[test]
    fn federated_barrier_error_preserves_concrete_source() {
        #[derive(Debug, thiserror::Error)]
        #[error("concrete coordination failure")]
        struct ConcreteError;

        let error = FederatedBarrierError::from_error(ConcreteError);
        let source = std::error::Error::source(&error).expect("source should be preserved");

        assert_eq!(source.to_string(), "concrete coordination failure");
    }
}
