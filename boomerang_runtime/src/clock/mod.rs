//! Physical clock selection, identity, checked time, and interruptible pacing.
//!
//! # Clock boundary
//!
//! The scheduler uses one selected-clock handle for time reads and interruptible
//! deadline waits. Native waits use the system timer; manual waits register a
//! deadline against shared externally advanced time. Scheduling, action storage,
//! and input admission remain outside the clock implementation.
//!
//! [`PhysicalClock`] is the target-neutral read contract: a configured
//! [`PhysicalClockDomainId`], a per-run [`ExecutionEpoch`], and an unsigned
//! [`PhysicalInstant`] timestamp. Its definitions in `time.rs` use no hosted runtime
//! facilities. Context reads, delayed physical events, and admission retain this
//! integer representation throughout. Logical-tag conversions live here and return
//! [`PhysicalClockError`] when the result cannot be represented.
//!
//! # Hosted state and lifetime
//!
//! With `external-clock`, a compiled local Federate can select a `ManualClock`
//! through `FederateBindings::with_physical_clock`. All Enclaves share
//! the same clock and random execution epoch. The run claims the clock once,
//! attaches bounded deadline slots, and detaches them during cleanup; the claim
//! remains consumed. Domain mismatch, reuse, terminal failure, or insufficient
//! mailbox wake capacity prevents startup.
//!
//! The manual implementation owns synchronized time, terminal failure, and wake
//! registration. The native implementation owns system-timer waiting and a shared
//! host origin, allocated once per Enclave. Send contexts created during graph
//! construction share that origin; reads return [`PhysicalClockError::NotStarted`]
//! until execution binds it. Bulk live execution binds every participant to one
//! origin before spawning workers; compiled execution does so after construction
//! and connection setup. Individual scheduler startup binds its origin once.
//! Without `external-clock`, there is no manual state, dispatch branch, deadline
//! registry, or optional entropy dependency. Cloning and reading an initialized
//! clock do not allocate.
//!
//! # Scheduler integration
//!
//! Physical actions use checked time and microstep arithmetic for either source.
//! Admission advances raw physical events past completed logical work, and action
//! storage selects distinct microsteps for queued values at the same offset.
//! Overflow propagates without replacing a queued value. Logical actions use the
//! same checked scheduling entry point and cursor selection, with their own time
//! base and full logical-duration range.
//!
//! `fast_forward` bypasses physical pacing, while reaction time reads and physical
//! actions continue to use the selected clock. [`crate::CommonContext::try_get_physical_time`]
//! returns [`PhysicalInstant`] in the selected execution timeline and reports
//! lifecycle, overflow, and terminal errors. Manual values never pass through a
//! host instant. Async action scheduling checks epoch-plus-delay arithmetic before
//! publication; raw events must already use the destination execution's epoch.
//! [`crate::Context::try_get_logical_time`] projects a logical tag into the same
//! coordinates with range checking; logical scheduling itself retains its wider
//! signed duration range.
//! Supervision, transport liveness, and observation timing use host-monotonic
//! time independently of the selected clock.

#[cfg(feature = "external-clock")]
mod manual;
mod native;
mod time;
use crate::{
    sched::federate::{
        FederateCoordinationError, FederateSchedulerCoordination, FederateTermination,
    },
    AsyncEvent, Duration, EnclaveKey, Tag,
};
#[cfg(all(test, feature = "external-clock"))]
pub(crate) use manual::tests::ALLOCATIONS;
#[cfg(feature = "external-clock")]
pub(crate) use manual::ClockRun;
#[cfg(feature = "external-clock")]
pub use manual::ManualClock;
use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};
pub use time::*;

/// Borrowed event and coordination resources for one clock synchronization.
/// Contains no deadline registry or owned clock state.
pub(crate) struct WaitContext<'a, 'coordination> {
    /// Enclave identity for wait diagnostics.
    pub(crate) key: EnclaveKey,
    /// Shared run origin used to map logical tags to host deadlines.
    pub(crate) origin: Instant,
    /// Scheduler mailbox that interrupts either form of physical wait.
    pub(crate) event_rx: &'a crate::Receiver<AsyncEvent>,
    /// Optional scheduler phase observer, always timed with the host clock.
    pub(crate) observation: Option<&'a crate::ObservationHandle>,
    /// Existing coordinator used to distinguish terminal mailbox closure.
    pub(crate) coordination:
        &'a mut Option<&'coordination mut (dyn FederateSchedulerCoordination + 'coordination)>,
}

/// Selected physical clock shared by an Enclave and its pre-start send contexts.
#[derive(Clone, Debug)]
pub(crate) enum RuntimeClock {
    /// Host origin bound once at execution startup; clones share the same origin.
    Native(Arc<OnceLock<Instant>>),
    /// One participant in a shared manual-clock execution.
    #[cfg(feature = "external-clock")]
    Manual {
        /// Shared time, execution identity and deadline registry.
        clock: ManualClock,
        /// Preallocated Enclave deadline slot.
        slot: usize,
    },
}

impl Default for RuntimeClock {
    fn default() -> Self {
        Self::Native(Arc::new(OnceLock::new()))
    }
}

impl RuntimeClock {
    /// Binds native time once and returns the origin actually used for host pacing.
    /// Manual time is already bound to its execution epoch and ignores this host origin.
    pub(crate) fn initialize(&self, origin: Instant) -> Instant {
        match self {
            Self::Native(shared) => *shared.get_or_init(|| origin),
            #[cfg(feature = "external-clock")]
            Self::Manual { .. } => origin,
        }
    }

    /// Creates an initialized native clock for isolated context tests.
    #[cfg(test)]
    pub(crate) fn native(origin: Instant) -> Self {
        let clock = Self::default();
        clock.initialize(origin);
        clock
    }

    /// Binds one participant without another allocation.
    #[cfg(feature = "external-clock")]
    pub(crate) fn manual(clock: ManualClock, slot: usize) -> Self {
        Self::Manual { clock, slot }
    }
    /// Reports retained clock failure, including during logical fast-forward.
    pub(crate) fn failed(&self) -> bool {
        match self {
            Self::Native(_) => false,
            #[cfg(feature = "external-clock")]
            Self::Manual { clock, .. } => clock.now().is_err(),
        }
    }
    /// Synchronizes a logical deadline or returns a scheduler interruption.
    pub(crate) fn wait_until(
        &self,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<ClockWaitResult, FederateCoordinationError> {
        match self {
            Self::Native(_) => native::wait_native(tag, wait),
            #[cfg(feature = "external-clock")]
            Self::Manual { clock, slot, .. } => clock.wait_until(*slot, tag, wait),
        }
    }
    /// Reads the selected execution timeline without converting manual time to host time.
    pub(crate) fn now(&self) -> Result<PhysicalInstant, PhysicalClockError> {
        match self {
            Self::Native(origin) => {
                let origin = *origin.get().ok_or(PhysicalClockError::NotStarted)?;
                PhysicalInstant::from_duration(
                    Instant::now()
                        .checked_duration_since(origin)
                        .ok_or(PhysicalClockError::Overflow)?,
                )
            }
            #[cfg(feature = "external-clock")]
            Self::Manual { clock, .. } => clock.now(),
        }
    }

    /// Applies a nonnegative delay with checked epoch arithmetic before publication.
    pub(crate) fn now_after(
        &self,
        delay: core::time::Duration,
    ) -> Result<PhysicalInstant, PhysicalClockError> {
        self.now()?.checked_add(delay)
    }
}
/// Result of physical synchronization, including scheduler interruption or termination.
pub(crate) enum ClockWaitResult {
    /// The requested physical deadline elapsed normally.
    DeadlineReached,
    /// A scheduler event interrupted the deadline and remains to be handled.
    Interrupted(AsyncEvent),
    /// Federate coordination terminated the scheduler through its existing event channel.
    FederateTerminated(FederateTermination),
}

impl PhysicalInstant {
    /// Maps an acquisition offset and nonnegative minimum delay to a logical tag.
    pub fn to_tag(self, minimum_delay: Duration) -> Result<Tag, PhysicalClockError> {
        let delay = u64::try_from(minimum_delay.whole_nanoseconds())
            .map_err(|_| PhysicalClockError::Overflow)?;
        let offset = self.checked_add(core::time::Duration::from_nanos(delay))?;
        let duration =
            Duration::try_from(offset.to_duration()).map_err(|_| PhysicalClockError::Overflow)?;
        Ok(Tag::new(duration, 0))
    }
    /// Converts a logical offset to unsigned nanoseconds, ignoring its microstep.
    pub(crate) fn from_tag(tag: Tag) -> Result<Self, PhysicalClockError> {
        u64::try_from(tag.offset().whole_nanoseconds())
            .map(Self)
            .map_err(|_| PhysicalClockError::Overflow)
    }
}
