//! Host requests for scheduler-owned output proofs.
//!
//! The scheduler decides when a tag is complete. A request only asks it to report
//! that proof. Shared lifecycle state carries termination and capacity accounting.
//! It does not retain the scheduler mailbox sender.
use super::{bound::OutputTarget, OutputBound, OutputCompletion, OutputWaitError};
use crate::{clock::ExecutionEpoch, AsyncEvent, Tag};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

/// One scheduler owner's state, shared with its host requests and unwind guards.
///
/// A request can outlive all observers and the scheduler. It therefore holds this
/// state separately from the sender-bearing endpoint. This prevents a cycle from
/// the mailbox queue, through a request, back to the mailbox sender.
#[derive(Debug, Default)]
pub(crate) struct Lifecycle {
    /// The first terminal error. `OnceLock` publishes the value to all readers.
    terminal: OnceLock<OutputWaitError>,
    /// Reserved request slots, including queued requests and unread replies.
    /// The counter publishes no other data, so relaxed ordering is sufficient.
    in_flight: AtomicUsize,
}
impl Lifecycle {
    /// Publishes the exact first error. Later termination cannot replace it.
    ///
    /// This operation is for execution teardown, not normal reaction execution.
    pub(crate) fn close(&self, reason: OutputWaitError) {
        let _ = self.terminal.set(reason);
    }

    /// Reads this owner's first error, or returns `None` before termination.
    pub(crate) fn error(&self) -> Option<OutputWaitError> {
        self.terminal.get().copied()
    }
}

/// A cheap value clone of the host request route.
///
/// The scheduler fixes the epoch before it exports any observer. Clones share
/// lifecycle state, but each clone owns its sender. Never put this value in a
/// queued request: its sender would create a mailbox ownership cycle.
#[derive(Clone, Debug)]
pub(crate) struct OutputEndpoint {
    /// Execution identity copied into each request and response.
    pub(crate) epoch: ExecutionEpoch,
    /// Maximum retained requests across the mailbox, pending list, and replies.
    pub(crate) capacity: usize,
    /// Scheduler mailbox route. Only host handles and scheduler state own it.
    pub(crate) event_tx: crate::Sender<AsyncEvent>,
    /// Shared termination and accounting, independent of this sender's lifetime.
    pub(crate) lifecycle: Arc<Lifecycle>,
}

/// A capability for one reactor's directly declared effects in one execution.
///
/// Clones keep the same reactor, enclave, and immutable execution epoch. The
/// capability covers effects declared directly by that reactor. It does not
/// include descendant reactors or confirm that an external device has settled.
///
/// A tag proof includes that tag and all earlier tags. It does not include later
/// microsteps at the same logical time. [`Self::close_time`] covers all microsteps
/// at a time and installs durable admission cutoffs. Neither proof authorizes
/// stepping over pending timers or delayed work.
///
/// Wait outside reactions, on a separate host thread if needed. A wait does not
/// run the scheduler. Waiting in a reaction can prevent the requested progress.
/// An observer does not keep execution alive. Configure `Config::with_keep_alive`
/// or queue startup requests before execution can quiesce.
/// Cloning an observer does not allocate. Explicit request creation allocates its
/// mailbox value, reply channel, and cancellation token. Checkpoint reporting and
/// ordinary reaction execution do not allocate for these requests.
#[derive(Clone, Debug)]
pub struct OutputObserver {
    /// Value-owned route. Queued requests share only its lifecycle state.
    pub(crate) endpoint: OutputEndpoint,
    /// Exact compiled enclave and reactor owner.
    pub(crate) target: OutputTarget,
}
impl OutputObserver {
    /// Returns the immutable execution identity used by requests and responses.
    pub fn epoch(&self) -> ExecutionEpoch {
        self.endpoint.epoch
    }

    /// Requests an inclusive tag proof without waiting for scheduler progress.
    ///
    /// The scheduler must still establish its existing completion proof. Creating
    /// this request does not execute a reaction or prove that an effect occurred.
    /// The request can remain queued until the host next runs the scheduler.
    ///
    /// The configured mailbox capacity also bounds all retained output requests.
    /// A slot stays reserved through mailbox and pending-list retention, and until
    /// the caller consumes or drops an unread reply. Dropping the returned handle
    /// cancels the request. It does not undo effects that have already occurred.
    ///
    /// # Errors
    ///
    /// Returns [`OutputWaitError::InvalidBound`] for an invalid tag and
    /// [`OutputWaitError::Capacity`] if the request limit or mailbox is full.
    /// Returns the owner's first terminal error if it is already known, or
    /// [`OutputWaitError::Disconnected`] if the mailbox route has closed.
    /// Request-specific validation errors can also arrive through [`OutputWait`].
    /// Backends without final logical-ingress exclusion return
    /// [`OutputWaitError::UnsupportedBackend`] for tag and time proofs.
    pub fn request_tag(&self, tag: Tag) -> Result<OutputWait, OutputWaitError> {
        self.request(OutputBound::ThroughTag(tag))
    }

    /// Requests closure of all microsteps at this physical time and earlier.
    ///
    /// Closure installs a durable admission cutoff. Dropping or timing out the
    /// returned wait does not restore admission below an installed cutoff.
    /// Existing execution permissions, source progress, and physical pacing apply.
    /// Overlapping proofs or exhausted bounded discovery can return `Capacity`.
    /// Cutoffs apply at Enclave boundaries and applicable upstream cuts. Input
    /// admission beyond this reactor can change. Pre-cutoff batches and authorized
    /// internal traffic must drain; fresh strict input follows its late policy.
    /// Unrestricted external physical input is retagged above the cutoff.
    /// Required sources must publish truthful exclusive frontiers beyond each cut.
    /// A failed discovery can leave earlier local or upstream cutoffs installed.
    /// Wait outside reactions and continue driving the scheduler.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use boomerang_runtime::{clock::PhysicalInstant, OutputObserver, OutputWaitError};
    /// fn close(observer: &OutputObserver) -> Result<(), OutputWaitError> {
    ///     let proof = observer.close_time(PhysicalInstant(10))?
    ///         .wait_timeout(std::time::Duration::from_secs(1))?;
    ///     assert_eq!(proof.epoch(), observer.epoch());
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `InvalidBound` when no later physical tag is representable.
    /// Other request and execution errors are the same as for `request_tag`.
    pub fn close_time(
        &self,
        time: crate::clock::PhysicalInstant,
    ) -> Result<OutputWait, OutputWaitError> {
        let bound = OutputBound::ThroughTime(time);
        bound.first_tag_after()?;
        self.request(bound)
    }

    /// Reserves one bounded request slot and transfers the request to its owner.
    fn request(&self, bound: OutputBound) -> Result<OutputWait, OutputWaitError> {
        bound.validate()?;
        let lifecycle = &self.endpoint.lifecycle;
        if let Some(error) = lifecycle.error() {
            return Err(error);
        }
        lifecycle
            .in_flight
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < self.endpoint.capacity).then_some(count + 1)
            })
            .map_err(|_| OutputWaitError::Capacity)?;
        let token = Arc::new(RequestToken {
            canceled: AtomicBool::new(false),
            lifecycle: lifecycle.clone(),
        });
        let (reply, receiver) = kanal::bounded(1);
        let request = OutputRequest {
            epoch: self.epoch(),
            target: self.target,
            bound,
            token: token.clone(),
            reply,
        };
        let wait = OutputWait {
            token,
            receiver,
            wake: self.endpoint.event_tx.clone(),
        };
        // On failure, try_send drops the request and this scope drops the wait.
        // The final token drop releases the reservation in either order.
        match self
            .endpoint
            .event_tx
            .try_send(AsyncEvent::OutputRequest(Box::new(request)))
        {
            Ok(true) => Ok(wait),
            Ok(false) => Err(OutputWaitError::Capacity),
            Err(_) => Err(lifecycle.error().unwrap_or(OutputWaitError::Disconnected)),
        }
    }

    /// Requests an inclusive tag proof, then waits for that request to resolve.
    ///
    /// Run the scheduler independently. Do not call this method in a reaction.
    ///
    /// # Errors
    ///
    /// Returns the errors from [`Self::request_tag`] or [`OutputWait::wait`].
    pub fn wait_through_tag(&self, tag: Tag) -> Result<OutputCompletion, OutputWaitError> {
        self.request_tag(tag)?.wait()
    }
}

/// One capacity reservation shared by the request and its host wait handle.
///
/// The final owner releases the slot. A canceled request still in the mailbox
/// must keep its slot until the scheduler removes it. A delivered reply must
/// keep its slot until the caller consumes or drops its wait handle.
#[derive(Debug)]
struct RequestToken {
    /// Cancellation has no associated payload, so relaxed ordering is sufficient.
    canceled: AtomicBool,
    /// Keeps accounting alive without retaining a mailbox sender.
    lifecycle: Arc<Lifecycle>,
}
impl Drop for RequestToken {
    fn drop(&mut self) {
        self.lifecycle.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// An opaque one-shot request carried by the scheduler mailbox.
///
/// Only an observer can create this value. Only scheduler-owned state can supply
/// its reply. Its token shares accounting and cancellation with the host handle.
/// The value owns no scheduler mailbox sender, so queued requests cannot retain
/// their own mailbox.
#[derive(Debug)]
pub struct OutputRequest {
    /// Execution to which this request belongs.
    pub(crate) epoch: ExecutionEpoch,
    /// Exact enclave and reactor that the scheduler must validate.
    pub(crate) target: OutputTarget,
    /// Requested inclusive tag or durable all-microstep time closure.
    pub(crate) bound: OutputBound,
    /// Per-request cancellation and capacity reservation.
    token: Arc<RequestToken>,
    /// Unique response route. Dropping it also marks the request as no longer held.
    reply: crate::Sender<Result<OutputCompletion, OutputWaitError>>,
}
impl OutputRequest {
    /// Tests cancellation before retaining or considering this request.
    pub(crate) fn canceled(&self) -> bool {
        self.token.canceled.load(Ordering::Relaxed)
    }

    /// Consumes this request and attempts its single reply without waiting.
    /// A dropped receiver means the caller no longer needs the result.
    pub(crate) fn reply(self, value: Result<OutputCompletion, OutputWaitError>) {
        let _ = self.reply.try_send(value);
    }
}

/// One cancellable output request, separate from the execution it observes.
///
/// Consuming this handle waits for its exact reply. Dropping it, including after
/// a timeout, cancels the request and asks the scheduler to reconsider its next
/// checkpoint. Cancellation does not stop execution or undo an external effect.
/// A canceled request cannot supply a reply to a later request.
///
/// Wait outside reactions. The scheduler must run independently. A proof already
/// delivered to this handle stays readable after later execution termination.
#[derive(Debug)]
pub struct OutputWait {
    /// Nonblocking cancellation wake route. This handle is never mailbox-owned.
    wake: crate::Sender<AsyncEvent>,
    /// Reservation held until both this handle and the scheduler request are gone.
    token: Arc<RequestToken>,
    /// Dedicated one-shot response. It cannot be reused by another request.
    receiver: crate::Receiver<Result<OutputCompletion, OutputWaitError>>,
}
impl OutputWait {
    /// Waits until this request resolves or its execution terminates.
    ///
    /// This method has no host-time deadline. A live scheduler that is never driven
    /// again can leave it waiting indefinitely. A delivered proof takes priority
    /// over a later error.
    ///
    /// # Errors
    ///
    /// Returns a request validation error or the owner's first terminal error.
    /// Returns [`OutputWaitError::Disconnected`] if the request or mailbox route
    /// disappears without a known terminal error.
    pub fn wait(self) -> Result<OutputCompletion, OutputWaitError> {
        self.receive(
            None,
            #[cfg(test)]
            || {},
        )
    }

    /// Waits for this request with a host-monotonic timeout.
    ///
    /// The timeout is independent of the execution's physical clock. It limits
    /// this wait only. Expiry consumes and cancels the request. A reply already
    /// available is returned even for a zero timeout.
    ///
    /// # Errors
    ///
    /// Returns [`OutputWaitError::Timeout`] when the host deadline expires.
    /// Otherwise returns the errors from [`Self::wait`].
    pub fn wait_timeout(self, timeout: Duration) -> Result<OutputCompletion, OutputWaitError> {
        self.receive(
            Some(timeout),
            #[cfg(test)]
            || {},
        )
    }

    /// Reads the one-shot result before checking termination or the host deadline.
    /// Short receive intervals also detect termination for mailbox-queued requests,
    /// which do not yet have a scheduler-owned reply operation to wake the channel.
    fn receive(
        &self,
        timeout: Option<Duration>,
        // Tests can publish a reply between the empty poll and the terminal checks.
        // This ordering hook and its calls do not exist in production builds.
        #[cfg(test)] mut after_empty_poll: impl FnMut(),
    ) -> Result<OutputCompletion, OutputWaitError> {
        let start = Instant::now();
        loop {
            // A completed one-shot proof remains valid if the scheduler later terminates.
            match self.receiver.try_recv() {
                Ok(Some(reply)) => return reply,
                Err(_) => {
                    return self.terminal_result(OutputWaitError::Disconnected);
                }
                Ok(None) => {}
            }
            #[cfg(test)]
            after_empty_poll();
            if let Some(error) = self.token.lifecycle.error() {
                return self.terminal_result(error);
            }
            if self.wake.is_closed() {
                return self.terminal_result(OutputWaitError::Disconnected);
            }
            let remaining = match timeout {
                Some(timeout) => {
                    let Some(remaining) = timeout
                        .checked_sub(start.elapsed())
                        .filter(|left| !left.is_zero())
                    else {
                        return self.terminal_result(OutputWaitError::Timeout);
                    };
                    remaining
                }
                None => Duration::from_millis(1),
            };
            match self
                .receiver
                .recv_timeout(remaining.min(Duration::from_millis(1)))
            {
                Ok(reply) => return reply,
                Err(kanal::ReceiveErrorTimeout::Timeout) => {}
                Err(_) => {
                    return self.terminal_result(OutputWaitError::Disconnected);
                }
            }
        }
    }

    /// Reads a delivered reply once more after observing termination or a deadline.
    /// Publication can race the earlier empty poll. Observe the first known error
    /// before this final read so a prior reply keeps priority over later termination.
    /// This check does not wait, allocate, or extend the host deadline.
    fn terminal_result(
        &self,
        fallback: OutputWaitError,
    ) -> Result<OutputCompletion, OutputWaitError> {
        let error = self.token.lifecycle.error().unwrap_or(fallback);
        match self.receiver.try_recv() {
            Ok(Some(reply)) => reply,
            _ => Err(error),
        }
    }
}
impl Drop for OutputWait {
    fn drop(&mut self) {
        self.token.canceled.store(true, Ordering::Relaxed);
        // Only an outstanding request owns the reply sender. Its disappearance
        // means no scheduler work needs cancellation. This tests request lifetime,
        // not an incidental Arc count, and avoids filling a one-slot mailbox after
        // a completed wait. A race can cause an extra wake, which is harmless.
        if !self.receiver.is_disconnected() {
            // Full means the scheduler already has an interruption to consume.
            let _ = self.wake.try_send(AsyncEvent::OutputWake);
        }
    }
}

#[cfg(test)]
mod tests;
