//! Hosted physical clocks and the execution policies used by reactions and schedulers.
//!
//! A manual clock is claimed once per Federate and shared by its Enclaves. Its
//! single mutex orders deadline registration, advancement, and terminal failure.
//! Native execution keeps the existing host-monotonic pacing behavior. The entire
//! module is gated by `external-clock`; target-neutral contracts live in
//! [`crate::physical_time`].
pub(crate) use crate::sched::clock::NativeClock;
use crate::sched::{
    clock::{WaitContext, WallClockReceive},
    federate::{FederateCoordinationError, FederateTermination},
};
use crate::{physical_time::*, AsyncEvent, Duration, Tag};
use std::{
    sync::{Arc, Mutex},
    time::{Duration as StdDuration, Instant},
};

/// Cloneable driver and observation handle for one execution's physical clock.
#[derive(Clone)]
pub struct ManualClock(Arc<Clock>);
/// Shared identity and synchronized state for a single manual-clock execution.
struct Clock {
    domain: PhysicalClockDomainId,
    epoch: ExecutionEpoch,
    state: Mutex<State>,
}
/// Clock time, first terminal failure, and deadline slots protected by one mutex.
/// Registration and advancement use this same lock to avoid lost wakes.
#[derive(Default)]
struct State {
    now: PhysicalTimeNanos,
    failure: Option<PhysicalClockError>,
    used: bool,
    slots: Vec<Slot>,
    abort: Option<crate::sched::federate::FederateAbortHandle>,
}
/// One reusable deadline and scheduler wake sender for an attached Enclave.
struct Slot {
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
        Ok(Self(Arc::new(Clock {
            domain,
            epoch: ExecutionEpoch(epoch),
            state: Mutex::new(State::default()),
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
    fn terminate(state: &mut State, error: PhysicalClockError) {
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
            .map(|wake| Slot {
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

/// Hosted execution policy. The native implementation preserves legacy host semantics.
pub(crate) trait ExecutionClock: std::fmt::Debug + Send + Sync {
    /// Reads the selected integer clock, or returns None for legacy host execution.
    fn physical_time(&self) -> Result<Option<PhysicalTimeNanos>, PhysicalClockError> {
        Ok(None)
    }
    /// Reads physical time in the scheduler Instant representation.
    fn instant(&self) -> Result<Instant, PhysicalClockError> {
        Ok(Instant::now())
    }
    /// Adds a nonnegative delay to the selected clock with checked conversion.
    fn instant_after(&self, delay: StdDuration) -> Result<Instant, PhysicalClockError> {
        Instant::now()
            .checked_add(delay)
            .ok_or(PhysicalClockError::Overflow)
    }
    /// Computes route arrival time while preserving each policy's error taxonomy.
    /// Native execution delegates to the existing host calculation.
    fn route_time(
        &self,
        _delay: StdDuration,
        host: &dyn Fn() -> Result<Instant, crate::OwnedStorageError>,
    ) -> Result<Instant, crate::OwnedStorageError> {
        host()
    }
    /// Maps a physical action delay to a tag under this clock's ordering policy.
    fn action_tag(
        &self,
        origin: Instant,
        _current: Tag,
        minimum: Duration,
        delay: Duration,
    ) -> Result<Tag, PhysicalClockError> {
        Ok(Tag::from_physical_time(origin, Instant::now()).delay(minimum + delay))
    }
    /// Validates a reserved action cursor; native execution retains legacy saturation.
    fn check_action_tag(&self, tag: Tag) -> Result<Tag, PhysicalClockError> {
        Ok(tag)
    }
    /// Applies incoming-event ordering and reserves a cursor when the policy requires it.
    /// Manual execution latches mapping or reservation failure before discarding an event.
    fn event_tag(
        &self,
        tag: Tag,
        _current: Tag,
        _reserve: &dyn Fn(Tag) -> Result<Tag, PhysicalClockError>,
    ) -> Result<Tag, PhysicalClockError> {
        Ok(tag)
    }
    /// Reports whether execution must stop because the selected clock has failed.
    fn failed(&self) -> bool {
        false
    }
    /// Paces a logical tag through the scheduler's interruptible waiting capability.
    fn wait(
        &self,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<WallClockReceive, FederateCoordinationError> {
        NativeClock.wait(tag, wait)
    }
}
impl ExecutionClock for NativeClock {}

/// Cloneable participant policy; the manual adapter is allocated once per Enclave.
#[derive(Clone, Debug)]
pub(crate) struct ClockContext(Arc<dyn ExecutionClock>);
impl Default for ClockContext {
    fn default() -> Self {
        static HOST: std::sync::LazyLock<Arc<dyn ExecutionClock>> =
            std::sync::LazyLock::new(|| Arc::new(NativeClock));
        Self(Arc::clone(&HOST))
    }
}
impl std::ops::Deref for ClockContext {
    type Target = dyn ExecutionClock;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}
impl ClockContext {
    /// Binds a shared manual clock to one attached Enclave slot and the run origin.
    pub(crate) fn manual(clock: ManualClock, slot: usize, origin: Instant) -> Self {
        Self(Arc::new(ManualExecutionClock {
            clock,
            slot,
            origin,
        }))
    }
}
/// Per-Enclave execution policy sharing one Federate clock and common origin.
#[derive(Debug)]
struct ManualExecutionClock {
    clock: ManualClock,
    slot: usize,
    origin: Instant,
}
impl ExecutionClock for ManualExecutionClock {
    fn physical_time(&self) -> Result<Option<PhysicalTimeNanos>, PhysicalClockError> {
        self.clock.now().map(Some)
    }
    fn instant(&self) -> Result<Instant, PhysicalClockError> {
        let result = self.clock.now().and_then(|now| now.to_instant(self.origin));
        if let Err(error) = result {
            self.clock.latch(error);
        }
        result
    }
    fn instant_after(&self, delay: StdDuration) -> Result<Instant, PhysicalClockError> {
        self.clock
            .now()?
            .checked_add(PhysicalTimeNanos::from_duration(delay)?)?
            .to_instant(self.origin)
    }
    fn route_time(
        &self,
        delay: StdDuration,
        _host: &dyn Fn() -> Result<Instant, crate::OwnedStorageError>,
    ) -> Result<Instant, crate::OwnedStorageError> {
        self.instant_after(delay).map_err(Into::into)
    }
    fn action_tag(
        &self,
        _origin: Instant,
        current: Tag,
        minimum: Duration,
        delay: Duration,
    ) -> Result<Tag, PhysicalClockError> {
        let delay = minimum
            .checked_add(delay)
            .ok_or(PhysicalClockError::Overflow)?;
        let tag = self.clock.now()?.to_tag(delay)?;
        after_current_tag(
            if delay.is_zero() {
                Tag::new(tag.offset(), 1)
            } else {
                tag
            },
            current,
        )
    }
    fn check_action_tag(&self, tag: Tag) -> Result<Tag, PhysicalClockError> {
        if tag.microstep() == usize::MAX {
            Err(PhysicalClockError::Overflow)
        } else {
            Ok(tag)
        }
    }
    fn event_tag(
        &self,
        tag: Tag,
        current: Tag,
        reserve: &dyn Fn(Tag) -> Result<Tag, PhysicalClockError>,
    ) -> Result<Tag, PhysicalClockError> {
        let result = after_current_tag(tag, current).and_then(reserve);
        if let Err(error) = result {
            self.clock.latch(error);
        }
        result
    }
    fn failed(&self) -> bool {
        self.clock.now().is_err()
    }
    fn wait(
        &self,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<WallClockReceive, FederateCoordinationError> {
        match PhysicalTimeNanos::from_tag(tag)
            .and_then(|deadline| self.clock.register(self.slot, deadline))
        {
            Ok(true) => Ok(WallClockReceive::DeadlineReached),
            Err(error) => {
                self.clock.latch(error);
                Ok(WallClockReceive::FederateTerminated(
                    FederateTermination::Abort,
                ))
            }
            Ok(false) => {
                let result = wait.receive();
                self.clock.cancel(self.slot);
                result
            }
        }
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
