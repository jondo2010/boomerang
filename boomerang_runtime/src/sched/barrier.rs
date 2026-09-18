use crate::{event::AsyncEvent, CommonContext, Duration, EnclaveKey, SendContext, Tag};

/// Failure while coordinating logical time with a local upstream enclave.
#[derive(Debug, thiserror::Error)]
pub enum LogicalTimeBarrierError {
    #[error("scheduler event channel closed while waiting for upstream enclave {upstream}")]
    EventChannelClosed { upstream: EnclaveKey },
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
    pub(super) fn release_tag(&mut self, tag: Tag) {
        if tag < self.released_tag {
            tracing::warn!(target: "boomerang::runtime",
                event = "runtime.barrier.release_rejected",
                upstream = %self.upstream_ctx.enclave_id(), tag = %tag,
                released_tag = %self.released_tag, reason = "tag_regression",
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

        if self.try_acquire_tag(upstream_tag) {
            return Ok(None);
        }

        if upstream_tag > self.provisional_tag {
            if !self
                .upstream_ctx
                .release_provisional(this_enclave, upstream_tag)
            {
                // The upstream has terminated try to return a queued event here. If the upstream terminated, we probably
                // have an event queued from it. This prevents pre-mature termination of this enclave.
                tracing::warn!(target: "boomerang::runtime",
                    event = "runtime.barrier.wait_interrupted", enclave = %this_enclave,
                    upstream = %self.upstream_ctx.enclave_id(), tag = %upstream_tag,
                    reason = "upstream_closed",
                );
                return event_rx.try_recv().map_err(|_| {
                    LogicalTimeBarrierError::EventChannelClosed {
                        upstream: self.upstream_ctx.enclave_id(),
                    }
                });
            }
            self.provisional_tag = upstream_tag;
        }

        tracing::debug!(target: "boomerang::runtime",
            event = "runtime.scheduler.waiting", enclave = %this_enclave,
            reason = "upstream_release", upstream = %self.upstream_ctx.enclave_id(),
            tag = %upstream_tag,
        );
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
}
