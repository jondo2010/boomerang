//! Optional hosted manual clock, bounded deadline registry, and run ownership.
//!
//! A [`ManualClock`] shares domain, epoch, time, failure, and deadline state through
//! one reference-counted allocation. The run guard claims it once and detaches
//! deadline slots on drop without making the clock reusable.
//!
//! Registration and advancement hold the same mutex, so advancing between a time
//! check and registration cannot lose a wake. One preallocated slot per Enclave
//! holds its current deadline and mailbox sender. Repeated timestamps do nothing;
//! a forward jump clears each due slot and sends at most one wake per Enclave.
//! A full mailbox already retains an interruption, after which the scheduler
//! rechecks time. The existing event queue preserves every eligible timer tag.
//! Deadline registration and advancement require no steady-state allocation.
//!
//! Regression, explicit close, and failure retain the first terminal error and
//! release physical and coordination waits through the existing Federate abort
//! path. Clock advancement provides time and wakeups; it does not authorize
//! execution beyond unresolved input progress.

use super::*;
use crate::{observation::SchedulerPhase, AsyncEvent};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(test)]
mod tests;

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
    now: PhysicalInstant,
    failure: Option<PhysicalClockError>,
    used: bool,
    slots: Vec<DeadlineSlot>,
    abort: Option<crate::sched::federate::FederateAbortHandle>,
}

/// One reusable deadline and scheduler wake sender for an attached Enclave.
struct DeadlineSlot {
    deadline: Option<PhysicalInstant>,
    wake: crate::Sender<AsyncEvent>,
}

/// Cloneable driver and observation handle for one execution's physical clock.
#[derive(Clone)]
pub struct ManualClock(Arc<ManualClockInner>);

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
    pub fn now(&self) -> Result<PhysicalInstant, PhysicalClockError> {
        let state = self.0.state.lock().unwrap();
        state.failure.map_or(Ok(state.now), Err)
    }
    /// Advances once, waking each due Enclave at most once. Equal timestamps are no-ops.
    pub fn advance_to(&self, time: PhysicalInstant) -> Result<(), PhysicalClockError> {
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
        deadline: PhysicalInstant,
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

    /// Registers a participant deadline before blocking on scheduler interruption.
    pub(super) fn wait_until(
        &self,
        slot: usize,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<ClockWaitResult, FederateCoordinationError> {
        match PhysicalInstant::from_tag(tag).and_then(|deadline| self.register(slot, deadline)) {
            Ok(true) => Ok(ClockWaitResult::DeadlineReached),
            Err(error) => {
                self.latch(error);
                Ok(ClockWaitResult::FederateTerminated(
                    FederateTermination::Abort,
                ))
            }
            Ok(false) => {
                let result = wait.receive();
                self.cancel(slot);
                result
            }
        }
    }
}

/// Owns a clock claim for one run and detaches its deadline slots on drop.
/// The claim remains consumed after cleanup, so the clock cannot be reused.
pub struct ClockRun(pub(crate) Option<ManualClock>);

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

impl PhysicalClock for ManualClock {
    fn domain(&self) -> PhysicalClockDomainId {
        self.domain()
    }
    fn epoch(&self) -> ExecutionEpoch {
        self.epoch()
    }
    fn now(&self) -> Result<PhysicalInstant, PhysicalClockError> {
        self.now()
    }
}

impl WaitContext<'_, '_> {
    /// Receives an interruption without imposing a host-time deadline.
    /// The selected manual clock has already registered its own deadline.
    pub(crate) fn receive(&mut self) -> Result<ClockWaitResult, FederateCoordinationError> {
        if let Some(observation) = self.observation {
            observation.enter(SchedulerPhase::PhysicalWait, Instant::now());
        }
        let event = self.event_rx.recv();
        if let Some(observation) = self.observation {
            observation.enter(SchedulerPhase::Framework, Instant::now());
        }
        match event {
            Ok(event) => Ok(ClockWaitResult::Interrupted(event)),
            Err(_) => {
                let terminal = self.coordination.as_deref_mut().map_or(
                    Ok(None),
                    FederateSchedulerCoordination::terminal_after_event_channel_closed,
                )?;
                Ok(ClockWaitResult::FederateTerminated(
                    terminal.unwrap_or(FederateTermination::Abort),
                ))
            }
        }
    }
}
