use crate::{event::AsyncEvent, CommonContext, Duration, EnclaveKey, SendContext, Tag};

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
    ) -> Option<AsyncEvent> {
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
            return None;
        }

        tracing::trace!(%upstream_tag, "Releasing provisional tag");
        self.provisional_tag = upstream_tag;
        if !self
            .upstream_ctx
            .release_provisional(this_enclave, upstream_tag)
        {
            // The upstream has terminated try to return a queued event here. If the upstream terminated, we probably
            // have an event queued from it. This prevents pre-mature termination of this enclave.
            tracing::warn!("Upstream has terminated");
            return event_rx.try_recv().expect("Upstream terminated");
        }

        // Block until the tag is released
        tracing::trace!("Blocking");
        event_rx.recv().ok()
    }
}
