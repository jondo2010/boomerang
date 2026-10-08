//! Shared hosted clocks and their domain/epoch read capabilities.
//!
//! A manual clock is claimed once per Federate and shared by its Enclaves. Its
//! single mutex orders deadline registration, advancement, and terminal failure.
//! Scheduler pacing and participant binding live in the private scheduler clock
//! module. This module is gated by `external-clock`; target-neutral contracts live in
//! [`crate::physical_time`].
use crate::{physical_time::*, AsyncEvent, Duration, Tag};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

/// Cloneable driver and observation handle for one execution's physical clock.
#[derive(Clone)]
pub struct ManualClock(Arc<ManualClockInner>);
/// Shared identity and synchronized state for a single manual-clock execution.
struct ManualClockInner {
    domain: PhysicalClockDomainId,
    epoch: ExecutionEpoch,
    state: Mutex<ManualClockState>,
}
/// Manual-clock time, first terminal failure, and deadline slots protected by one mutex.
/// Registration and advancement use this same lock to avoid lost wakes.
#[derive(Default)]
struct ManualClockState {
    now: PhysicalTimeNanos,
    failure: Option<PhysicalClockError>,
    used: bool,
    slots: Vec<DeadlineSlot>,
    abort: Option<crate::sched::federate::FederateAbortHandle>,
}
/// One reusable deadline and scheduler wake sender for an attached Enclave.
struct DeadlineSlot {
    deadline: Option<PhysicalTimeNanos>,
    wake: crate::Sender<AsyncEvent>,
}
impl std::fmt::Debug for ManualClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManualClock")
            .field("domain", &self.domain())
            .field("epoch", &self.epoch())
            .finish_non_exhaustive()
    }
}
impl ManualClock {
    /// Creates a stopped-at-zero clock with a fresh random execution epoch.
    pub fn new(domain: PhysicalClockDomainId) -> Result<Self, PhysicalClockError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| PhysicalClockError::EntropyUnavailable)?;
        let epoch = u128::from_ne_bytes(bytes);
        Ok(Self(Arc::new(ManualClockInner {
            domain,
            epoch: ExecutionEpoch(epoch),
            state: Mutex::new(ManualClockState::default()),
        })))
    }
    /// Configured domain shared by every participant and adapter.
    pub fn domain(&self) -> PhysicalClockDomainId {
        self.0.domain
    }
    /// Fresh identity shared by this run's participants and adapters.
    pub fn epoch(&self) -> ExecutionEpoch {
        self.0.epoch
    }
    /// Validates an adapter or observation identity without changing clock state.
    pub fn validate(
        &self,
        domain: PhysicalClockDomainId,
        epoch: ExecutionEpoch,
    ) -> Result<(), PhysicalClockError> {
        if domain != self.domain() {
            return Err(PhysicalClockError::DomainMismatch);
        }
        if epoch != self.epoch() {
            return Err(PhysicalClockError::EpochMismatch);
        }
        self.now().map(|_| ())
    }
    /// Reads epoch-relative physical time or the first retained terminal failure.
    pub fn now(&self) -> Result<PhysicalTimeNanos, PhysicalClockError> {
        let state = self.0.state.lock().unwrap();
        state.failure.map_or(Ok(state.now), Err)
    }
    /// Advances once, waking each due Enclave at most once. Equal timestamps are no-ops.
    pub fn advance_to(&self, time: PhysicalTimeNanos) -> Result<(), PhysicalClockError> {
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = state.failure {
            return Err(error);
        }
        if time < state.now {
            Self::terminate(&mut state, PhysicalClockError::Regression);
            return Err(PhysicalClockError::Regression);
        }
        if time == state.now {
            return Ok(());
        }
        state.now = time;
        for slot in &mut state.slots {
            if slot.deadline.is_some_and(|deadline| deadline <= time) {
                slot.deadline = None;
                // A full mailbox already retains an interruption; the scheduler rechecks time.
                let _ = slot.wake.try_send(AsyncEvent::FederateResume);
            }
        }
        Ok(())
    }
    /// Closes the clock and releases all participants, including idle coordination waits.
    pub fn close(&self) {
        self.latch(PhysicalClockError::Closed);
    }
    /// Fails the clock and releases all participants. The first failure is retained.
    pub fn fail(&self) {
        self.latch(PhysicalClockError::Failed);
    }
    /// Retains the first terminal error and releases every attached participant.
    pub(crate) fn latch(&self, error: PhysicalClockError) {
        Self::terminate(&mut self.0.state.lock().unwrap(), error);
    }
    /// Records the first failure while holding the state lock and wakes all participants.
    fn terminate(state: &mut ManualClockState, error: PhysicalClockError) {
        if state.failure.is_some() {
            return;
        }
        state.failure = Some(error);
        for slot in &mut state.slots {
            slot.deadline = None;
            let _ = slot.wake.try_send(AsyncEvent::FederateResume);
        }
        if let Some(abort) = &state.abort {
            abort.abort();
        }
    }
    /// Validates the domain and irrevocably reserves this clock for one Federate run.
    pub(crate) fn claim(&self, domain: PhysicalClockDomainId) -> Result<(), PhysicalClockError> {
        self.validate(domain, self.epoch())?;
        let mut state = self.0.state.lock().unwrap();
        if state.used {
            return Err(PhysicalClockError::AlreadyUsed);
        }
        if let Some(error) = state.failure {
            return Err(error);
        }
        state.used = true;
        Ok(())
    }
    /// Allocates one deadline slot per Enclave and installs the run abort handle.
    /// Called after claiming the clock and before starting participant execution.
    pub(crate) fn attach(
        &self,
        senders: &[crate::Sender<AsyncEvent>],
        abort: crate::sched::federate::FederateAbortHandle,
    ) -> Result<(), PhysicalClockError> {
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = state.failure {
            return Err(error);
        }
        state.slots = senders
            .iter()
            .map(|wake| DeadlineSlot {
                deadline: None,
                wake: wake.clone(),
            })
            .collect();
        state.abort = Some(abort);
        Ok(())
    }
    /// Releases run-owned senders and supervision while preserving time and failure.
    pub(crate) fn detach(&self) {
        let mut state = self.0.state.lock().unwrap();
        state.slots.clear();
        state.abort = None;
    }
    /// Replaces an Enclave deadline and atomically checks whether it is already due.
    /// Returns true when no wait is needed; the slot must belong to the attached run.
    pub(crate) fn register(
        &self,
        slot: usize,
        deadline: PhysicalTimeNanos,
    ) -> Result<bool, PhysicalClockError> {
        let mut state = self.0.state.lock().unwrap();
        if let Some(error) = state.failure {
            return Err(error);
        }
        let reached = deadline <= state.now;
        state.slots[slot].deadline = (!reached).then_some(deadline);
        Ok(reached)
    }
    /// Clears an attached Enclave deadline after its interruptible wait returns.
    pub(crate) fn cancel(&self, slot: usize) {
        self.0.state.lock().unwrap().slots[slot].deadline = None;
    }
}

impl PhysicalTimeNanos {
    /// Converts the selected epoch offset to the legacy Instant representation, checking range.
    pub fn to_instant(self, origin: Instant) -> Result<Instant, PhysicalClockError> {
        origin
            .checked_add(self.to_duration())
            .ok_or(PhysicalClockError::Overflow)
    }
    /// Maps an acquisition offset and nonnegative minimum delay to a logical tag.
    pub fn to_tag(self, minimum_delay: Duration) -> Result<Tag, PhysicalClockError> {
        let delay = u64::try_from(minimum_delay.whole_nanoseconds())
            .map_err(|_| PhysicalClockError::Overflow)?;
        let offset = self.checked_add(Self(delay))?;
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

/// Keeps acquisition time separate from the next executable logical tag.
pub(crate) fn after_current_tag(mapped: Tag, current: Tag) -> Result<Tag, PhysicalClockError> {
    if mapped > current {
        Ok(mapped)
    } else {
        Ok(Tag::new(
            current.offset(),
            current
                .microstep()
                .checked_add(1)
                .ok_or(PhysicalClockError::Overflow)?,
        ))
    }
}

#[cfg(test)]
mod tests;

/// Owns a clock claim for one run and detaches its deadline slots on drop.
/// The claim remains consumed after cleanup, so the clock cannot be reused.
pub(crate) struct ClockRun(pub(crate) Option<ManualClock>);
impl ClockRun {
    /// Claims a selected manual clock, or creates an inert native-run guard.
    pub(crate) fn new(
        selection: Option<&(PhysicalClockDomainId, ManualClock)>,
    ) -> Result<Self, PhysicalClockError> {
        if let Some((domain, clock)) = selection {
            clock.claim(*domain)?;
        }
        Ok(Self(selection.map(|(_, clock)| clock.clone())))
    }
}
impl Drop for ClockRun {
    fn drop(&mut self) {
        if let Some(clock) = &self.0 {
            clock.detach();
        }
    }
}

/// Host-monotonic read capability for adapters using the Federate's existing shared origin.
/// It does not change the default scheduler's native host wait path.
#[derive(Clone, Debug)]
pub struct HostClock {
    domain: PhysicalClockDomainId,
    epoch: ExecutionEpoch,
    origin: Instant,
}
impl HostClock {
    /// Creates one identity for a run's host-clock adapters; share clones within that run.
    pub fn new(domain: PhysicalClockDomainId, origin: Instant) -> Result<Self, PhysicalClockError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| PhysicalClockError::EntropyUnavailable)?;
        Ok(Self {
            domain,
            epoch: ExecutionEpoch(u128::from_ne_bytes(bytes)),
            origin,
        })
    }
}
impl PhysicalClock for HostClock {
    fn domain(&self) -> PhysicalClockDomainId {
        self.domain
    }
    fn epoch(&self) -> ExecutionEpoch {
        self.epoch
    }
    fn now(&self) -> Result<PhysicalTimeNanos, PhysicalClockError> {
        let elapsed = Instant::now()
            .checked_duration_since(self.origin)
            .ok_or(PhysicalClockError::Overflow)?;
        PhysicalTimeNanos::from_duration(elapsed)
    }
}
impl PhysicalClock for ManualClock {
    fn domain(&self) -> PhysicalClockDomainId {
        self.domain()
    }
    fn epoch(&self) -> ExecutionEpoch {
        self.epoch()
    }
    fn now(&self) -> Result<PhysicalTimeNanos, PhysicalClockError> {
        self.now()
    }
}
