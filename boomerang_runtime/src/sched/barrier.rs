use crate::{
    event::AsyncEvent, progress::ProgressBound, CommonContext, Duration, EnclaveKey, RuntimeError,
    SendContext, Tag,
};

/// Stores execution permission separately from confirmed computational progress.
#[derive(Debug)]
struct UpstreamProgress {
    /// Greatest execution permission, including provisional permission.
    permitted_through: Tag,
    /// Greatest confirmed tag or time boundary. Provisional permission does not advance it.
    completed_through: Option<ProgressBound>,
}

/// Coordinates this scheduler with one upstream Enclave through an existing dependency edge.
#[derive(Debug)]
pub(super) struct LogicalTimeBarrier {
    /// Existing route for upstream coordination messages.
    pub(super) upstream: SendContext,
    /// Logical delay on the upstream connection. `None` preserves the source tag.
    pub(super) delay: Option<Duration>,
    /// Execution permission and confirmed progress for this dependency.
    progress: UpstreamProgress,
    /// Highest outstanding provisional request. Insufficient progress preserves it.
    requested_through: Option<Tag>,
}

impl LogicalTimeBarrier {
    /// Creates a barrier before upstream progress or permission is known.
    pub(super) fn new(upstream: SendContext, delay: Option<Duration>) -> Self {
        Self {
            upstream,
            delay,
            progress: UpstreamProgress {
                permitted_through: Tag::NEVER,
                completed_through: None,
            },
            requested_through: None,
        }
    }

    /// Tests computational evidence without treating provisional permission as completion.
    pub(super) fn completed_through(&self, bound: ProgressBound) -> bool {
        self.progress
            .completed_through
            .is_some_and(|completed| completed >= bound)
    }

    /// Records computational progress without treating a time frontier as an executed tag.
    pub(super) fn observe_completion(&mut self, bound: ProgressBound) {
        self.progress.completed_through = Some(
            self.progress
                .completed_through
                .map_or(bound, |old| old.max(bound)),
        );
        if let ProgressBound::ThroughTag(tag) = bound {
            self.permit_through(tag);
        } else {
            self.retire_satisfied_request();
        }
    }

    /// Advances execution permission without reducing confirmed progress.
    fn permit_through(&mut self, tag: Tag) {
        let permitted = self.progress.permitted_through;
        if tag < permitted {
            tracing::warn!(target: "boomerang::runtime",
                event = "runtime.barrier.release_rejected",
                upstream = self.upstream.enclave_id().as_u32(),
                tag_kind = tag.kind_str(), tag_offset_ns = tag.offset().whole_nanoseconds(), tag_microstep = tag.microstep(),
                released_tag_kind = permitted.kind_str(), released_tag_offset_ns = permitted.offset().whole_nanoseconds(), released_tag_microstep = permitted.microstep(), reason = "tag_regression",
            );
        } else {
            self.progress.permitted_through = tag;
        }
        self.retire_satisfied_request();
    }

    /// Removes the outstanding request only when execution permission covers it.
    fn retire_satisfied_request(&mut self) {
        if self
            .requested_through
            .is_some_and(|tag| self.try_acquire_tag(tag))
        {
            self.requested_through = None;
        }
    }

    /// Accepts provisional permission only within an outstanding request.
    pub(super) fn permit_provisional(&mut self, tag: Tag) {
        if self
            .requested_through
            .is_some_and(|requested| tag <= requested)
        {
            self.permit_through(tag);
        }
    }

    /// Tests ordinary execution permission, including provisional permission.
    #[inline]
    pub(super) fn try_acquire_tag(&self, tag: Tag) -> bool {
        tag <= self.progress.permitted_through
            || self.completed_through(ProgressBound::ThroughTag(tag))
    }

    /// Waits for upstream permission or a scheduler event.
    ///
    /// Returns `None` when upstream permission covers the required tag.
    /// Returns the event when another scheduler event interrupts the wait.
    /// Returns an error when the scheduler event channel is closed.
    #[inline]
    pub(super) fn acquire_tag(
        &mut self,
        tag: Tag,
        this_enclave: EnclaveKey,
        event_rx: &crate::Receiver<AsyncEvent>,
    ) -> Result<Option<AsyncEvent>, RuntimeError> {
        // Require upstream permission through the last tag that can affect this tag.
        let upstream_tag = if let Some(delay) = self.delay {
            tag.pre(delay)
        } else {
            tag
        };

        if self.try_acquire_tag(upstream_tag) {
            return Ok(None);
        }

        if self
            .requested_through
            .is_none_or(|requested| upstream_tag > requested)
        {
            if self
                .upstream
                .release_provisional(this_enclave, upstream_tag)
                .is_err()
            {
                // Drain any queued event after the upstream route terminates.
                // This preserves work that arrived before termination.
                tracing::warn!(target: "boomerang::runtime",
                    event = "runtime.barrier.wait_interrupted", enclave = this_enclave.as_u32(),
                    upstream = self.upstream.enclave_id().as_u32(),
                    tag_kind = upstream_tag.kind_str(), tag_offset_ns = upstream_tag.offset().whole_nanoseconds(), tag_microstep = upstream_tag.microstep(),
                    reason = "upstream_closed",
                );
                return event_rx
                    .try_recv()
                    .map_err(|_| RuntimeError::UpstreamEventChannelClosed {
                        upstream: self.upstream.enclave_id(),
                    });
            }
            self.requested_through = Some(upstream_tag);
        }

        tracing::debug!(target: "boomerang::runtime",
            event = "runtime.scheduler.waiting", enclave = this_enclave.as_u32(),
            reason = "upstream_release", upstream = self.upstream.enclave_id().as_u32(),
            tag_kind = upstream_tag.kind_str(), tag_offset_ns = upstream_tag.offset().whole_nanoseconds(), tag_microstep = upstream_tag.microstep(),
        );
        event_rx
            .recv()
            .map(Some)
            .map_err(|_| RuntimeError::UpstreamEventChannelClosed {
                upstream: self.upstream.enclave_id(),
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
        let mut barrier = LogicalTimeBarrier::new(
            SendContext {
                clock: Default::default(),
                enclave_key: upstream,
                async_tx: upstream_tx,
                shutdown_rx,
            },
            None,
        );
        let (event_tx, event_rx) = kanal::unbounded();
        drop(event_tx);

        assert!(matches!(
            barrier.acquire_tag(Tag::ZERO, EnclaveKey::from(0), &event_rx),
            Err(RuntimeError::UpstreamEventChannelClosed { upstream: observed })
                if observed == upstream
        ));
    }

    #[test]
    fn local_barrier_suppresses_repeated_provisional_requests_until_release() {
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let mut barrier = LogicalTimeBarrier::new(
            SendContext {
                clock: Default::default(),
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            None,
        );
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

        barrier.observe_completion(ProgressBound::ThroughTag(later));
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
        let mut barrier = LogicalTimeBarrier::new(
            SendContext {
                clock: Default::default(),
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            None,
        );
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

        barrier.observe_completion(ProgressBound::ThroughTag(intermediate));
        assert!(barrier.try_acquire_tag(intermediate));
        assert_eq!(barrier.requested_through, Some(requested));

        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(requested, this_enclave, &event_rx)
            .unwrap()
            .is_some());
        assert!(upstream_rx.try_recv().unwrap().is_none());

        barrier.observe_completion(ProgressBound::ThroughTag(requested));
        assert_eq!(barrier.requested_through, None);
        assert!(barrier
            .acquire_tag(requested, this_enclave, &event_rx)
            .unwrap()
            .is_none());
        assert!(upstream_rx.try_recv().unwrap().is_none());
    }

    #[test]
    fn local_barrier_distinguishes_permission_tag_completion_and_time_closure() {
        use crate::clock::PhysicalInstant;
        let (upstream_tx, upstream_rx) = kanal::unbounded();
        let (_shutdown_tx, shutdown_rx) = keepalive::channel();
        let mut barrier = LogicalTimeBarrier::new(
            SendContext {
                clock: Default::default(),
                enclave_key: EnclaveKey::from(1),
                async_tx: upstream_tx,
                shutdown_rx,
            },
            None,
        );
        let (event_tx, event_rx) = kanal::unbounded();
        let tag = Tag::new(Duration::nanoseconds(2), 0);
        barrier.permit_provisional(tag);
        assert!(
            !barrier.try_acquire_tag(tag),
            "unsolicited provisional release granted permission"
        );
        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(tag, EnclaveKey::from(0), &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, tag);
        barrier.permit_provisional(tag);
        assert!(barrier.try_acquire_tag(tag));
        assert!(
            !barrier.completed_through(ProgressBound::ThroughTime(PhysicalInstant(1))),
            "provisional permission became completion evidence"
        );

        barrier.observe_completion(ProgressBound::ThroughTag(tag));
        assert!(barrier.completed_through(ProgressBound::ThroughTag(tag)));
        assert!(barrier.completed_through(ProgressBound::ThroughTime(PhysicalInstant(1))));
        assert!(!barrier.completed_through(ProgressBound::ThroughTime(PhysicalInstant(2))));

        let last_microstep = Tag::new(tag.offset(), usize::MAX);
        queue_interruption(&event_tx);
        assert!(barrier
            .acquire_tag(last_microstep, EnclaveKey::from(0), &event_rx)
            .unwrap()
            .is_some());
        assert_provisional_request(&upstream_rx, last_microstep);
        barrier.observe_completion(ProgressBound::ThroughTime(PhysicalInstant(1)));
        assert_eq!(barrier.requested_through, Some(last_microstep));
        barrier.observe_completion(ProgressBound::ThroughTime(PhysicalInstant(2)));
        assert_eq!(barrier.requested_through, None);
        assert!(barrier.try_acquire_tag(last_microstep));

        barrier.observe_completion(ProgressBound::ThroughTag(Tag::ZERO));
        assert!(
            barrier.completed_through(ProgressBound::ThroughTime(PhysicalInstant(2))),
            "older tag completion replaced the stronger time frontier"
        );
    }
}
