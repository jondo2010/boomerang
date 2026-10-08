//! Clock pacing and the scheduler resources needed for interruptible waits.
//!
//! A concrete handle selects native or externally advanced time. Both use the same
//! scheduler-facing API; disabling external clocks leaves only a zero-sized native
//! variant. Acquisition/tag compatibility is separate from clock synchronization.

pub(crate) mod mapping;

use super::federate::{
    FederateCoordinationError, FederateSchedulerCoordination, FederateTermination,
};
use crate::{observation::SchedulerPhase, AsyncEvent, EnclaveKey, Tag};
#[cfg(feature = "external-clock")]
use crate::{
    physical_clock::ManualClock,
    physical_time::{PhysicalClockError, PhysicalTimeNanos},
};
use kanal::ReceiveErrorTimeout;
use std::time::Instant;

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

#[cfg(feature = "external-clock")]
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

/// Selected runtime clock; the native-only representation is zero-sized.
/// Manual participants retain only their shared driver, deadline slot, and run origin.
#[derive(Clone, Debug, Default)]
pub(crate) enum RuntimeClock {
    /// Existing host-monotonic timing with no owned clock state.
    #[default]
    Native,
    /// One participant of the Federate's shared externally driven clock.
    #[cfg(feature = "external-clock")]
    Manual {
        /// Shared execution identity, time, and deadline registry.
        clock: ManualClock,
        /// This Enclave's preallocated deadline slot.
        slot: usize,
        /// Shared origin for checked conversion to the legacy Instant representation.
        origin: Instant,
    },
}

impl RuntimeClock {
    /// Binds an attached manual clock without allocating another policy object.
    #[cfg(feature = "external-clock")]
    pub(crate) fn manual(clock: ManualClock, slot: usize, origin: Instant) -> Self {
        Self::Manual {
            clock,
            slot,
            origin,
        }
    }

    /// Reports terminal clock failure, including when fast-forward bypasses waiting.
    pub(crate) fn failed(&self) -> bool {
        match self {
            Self::Native => false,
            #[cfg(feature = "external-clock")]
            Self::Manual { clock, .. } => clock.now().is_err(),
        }
    }

    /// Waits for the selected clock or returns a scheduler interruption or termination.
    pub(crate) fn wait_until(
        &self,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<ClockWaitResult, FederateCoordinationError> {
        match self {
            Self::Native => wait_native(tag, wait),
            #[cfg(feature = "external-clock")]
            Self::Manual { clock, slot, .. } => {
                match PhysicalTimeNanos::from_tag(tag)
                    .and_then(|deadline| clock.register(*slot, deadline))
                {
                    Ok(true) => Ok(ClockWaitResult::DeadlineReached),
                    Err(error) => {
                        clock.latch(error);
                        Ok(ClockWaitResult::FederateTerminated(
                            FederateTermination::Abort,
                        ))
                    }
                    Ok(false) => {
                        let result = wait.receive();
                        clock.cancel(*slot);
                        result
                    }
                }
            }
        }
    }

    /// Reads the selected integer clock, or None for legacy native execution.
    #[cfg(feature = "external-clock")]
    pub(crate) fn physical_time(&self) -> Result<Option<PhysicalTimeNanos>, PhysicalClockError> {
        match self {
            Self::Native => Ok(None),
            Self::Manual { clock, .. } => clock.now().map(Some),
        }
    }

    /// Reads selected time as an Instant and retains manual-clock conversion failure.
    #[cfg(feature = "external-clock")]
    pub(crate) fn instant(&self) -> Result<Instant, PhysicalClockError> {
        let result = self.instant_after(std::time::Duration::ZERO);
        if let (Self::Manual { clock, .. }, Err(error)) = (self, &result) {
            clock.latch(*error);
        }
        result
    }

    /// Adds a nonnegative physical delay with checked conversion.
    #[cfg(feature = "external-clock")]
    pub(crate) fn instant_after(
        &self,
        delay: std::time::Duration,
    ) -> Result<Instant, PhysicalClockError> {
        match self {
            Self::Native => Instant::now()
                .checked_add(delay)
                .ok_or(PhysicalClockError::Overflow),
            Self::Manual { clock, origin, .. } => clock
                .now()?
                .checked_add(PhysicalTimeNanos::from_duration(delay)?)?
                .to_instant(*origin),
        }
    }
}

/// Synchronizes a tag to the system timer with the existing native receive behavior.
fn wait_native(
    tag: Tag,
    wait: &mut WaitContext<'_, '_>,
) -> Result<ClockWaitResult, FederateCoordinationError> {
    let target = tag.to_logical_time(wait.origin);
    let now = std::time::Instant::now();

    match now.cmp(&target) {
        std::cmp::Ordering::Less => {
            let advance = target - now;
            tracing::debug!(target: "boomerang::runtime",
                event = "runtime.scheduler.waiting", enclave = wait.key.as_u32(),
                reason = "wall_clock", duration_ns = advance.as_nanos(),
            );

            let observation = wait.observation;
            let received = receive_until_wall_clock_deadline(
                target,
                wait.event_rx,
                || {
                    if let Some(observation) = observation {
                        observation.enter(SchedulerPhase::PhysicalWait, std::time::Instant::now());
                    }
                },
                || {
                    wait.coordination.as_deref_mut().map_or(
                        Ok(None),
                        FederateSchedulerCoordination::terminal_after_event_channel_closed,
                    )
                },
            );
            if let Some(observation) = observation {
                observation.enter(SchedulerPhase::Framework, std::time::Instant::now());
            }
            return received;
        }

        std::cmp::Ordering::Greater => {
            let delay = now - target;
            tracing::warn!(target: "boomerang::runtime",
                event = "runtime.scheduler.deadline_missed", enclave = wait.key.as_u32(),
                delay_ns = delay.as_nanos(),
            );
        }

        std::cmp::Ordering::Equal => {}
    }

    Ok(ClockWaitResult::DeadlineReached)
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

/// Performs one uninterrupted scheduler-event receive until a physical deadline.
fn receive_until_wall_clock_deadline(
    target: std::time::Instant,
    event_rx: &crate::Receiver<AsyncEvent>,
    entered_receive: impl FnOnce(),
    terminal_after_close: impl FnOnce()
        -> Result<Option<FederateTermination>, FederateCoordinationError>,
) -> Result<ClockWaitResult, FederateCoordinationError> {
    let advance = target.saturating_duration_since(std::time::Instant::now());
    entered_receive();
    match event_rx.recv_timeout(advance) {
        Ok(event) => Ok(ClockWaitResult::Interrupted(event)),
        Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
            if let Some(termination) = terminal_after_close()? {
                return Ok(ClockWaitResult::FederateTerminated(termination));
            }
            if let Some(remaining) = target.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(remaining);
            }
            Ok(ClockWaitResult::DeadlineReached)
        }
        Err(ReceiveErrorTimeout::Timeout) => Ok(ClockWaitResult::DeadlineReached),
    }
}

#[cfg(test)]
mod wall_clock_tests {
    //! Exact receive-entry coverage for coordinated wall-clock termination.

    use super::{receive_until_wall_clock_deadline, ClockWaitResult};
    use crate::{
        image::EnclaveIndex,
        sched::federate::{
            FederateCoordinationParts, FederateSchedulerCoordination, FederateTermination,
            LifecyclePolicy, LocalFederateCoordinationBackend,
        },
        AsyncEvent,
    };
    use std::{sync::mpsc, time::Duration as StdDuration};

    /// Checks that native pacing leaves due tags alone and returns queued interruptions.
    #[test]
    fn native_clock_paces_tags_and_preserves_interruptions() {
        use super::{RuntimeClock, WaitContext};
        use crate::{Duration, EnclaveKey, Tag};
        let (tx, rx) = kanal::unbounded();
        let mut coordination = None;
        let mut wait = WaitContext {
            key: EnclaveKey::new(0),
            origin: std::time::Instant::now(),
            event_rx: &rx,
            observation: None,
            coordination: &mut coordination,
        };
        tx.send(AsyncEvent::FederateResume).unwrap();
        assert!(matches!(
            RuntimeClock::default()
                .wait_until(Tag::new(Duration::ZERO, 0), &mut wait)
                .unwrap(),
            ClockWaitResult::DeadlineReached
        ));
        assert!(matches!(
            RuntimeClock::default()
                .wait_until(Tag::new(Duration::seconds(60), 0), &mut wait)
                .unwrap(),
            ClockWaitResult::Interrupted(AsyncEvent::FederateResume)
        ));
    }

    /// Proves disabling hosted clocks leaves no clock state in the scheduler layout.
    #[cfg(not(feature = "external-clock"))]
    #[test]
    fn native_clock_handle_is_zero_sized() {
        assert_eq!(std::mem::size_of::<super::RuntimeClock>(), 0);
    }

    /// Verifies coordinator abort closes an entered timed receive only after queuing termination.
    #[test]
    fn coordinated_abort_interrupts_entered_wall_clock_receive() {
        let enclave = EnclaveIndex::new(0);
        let (event_tx, event_rx) = kanal::unbounded::<AsyncEvent>();
        let scheduler_event_rx = event_rx.clone();
        let FederateCoordinationParts {
            abort_handle,
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::KeepAlive,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let target = std::time::Instant::now() + StdDuration::from_secs(1);

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            let waiter = scope.spawn(move || {
                receive_until_wall_clock_deadline(
                    target,
                    &scheduler_event_rx,
                    || entered_tx.send(()).unwrap(),
                    || participant.terminal_after_event_channel_closed(),
                )
            });
            entered_rx
                .recv_timeout(StdDuration::from_millis(100))
                .expect("scheduler must enter the production timed-receive boundary");
            abort_handle.abort();

            assert!(matches!(
                waiter.join().unwrap().unwrap(),
                ClockWaitResult::FederateTerminated(FederateTermination::Abort)
            ));
            coordinator.join().unwrap().coordination_result.unwrap();
        });
    }
}
