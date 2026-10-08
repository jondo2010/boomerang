//! Clock pacing and the scheduler resources needed for interruptible waits.
//!
//! Native pacing is always available and uses static dispatch when hosted external
//! clocks are disabled. External-clock policy lives in `crate::physical_clock`
//! when enabled; both policies return events for the scheduler to handle.

use super::federate::{
    FederateCoordinationError, FederateSchedulerCoordination, FederateTermination,
};
use crate::{observation::SchedulerPhase, AsyncEvent, EnclaveKey, Tag};
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
    pub(crate) fn receive(&mut self) -> Result<WallClockReceive, FederateCoordinationError> {
        if let Some(observation) = self.observation {
            observation.enter(SchedulerPhase::PhysicalWait, Instant::now());
        }
        let event = self.event_rx.recv();
        if let Some(observation) = self.observation {
            observation.enter(SchedulerPhase::Framework, Instant::now());
        }
        match event {
            Ok(event) => Ok(WallClockReceive::Interrupted(event)),
            Err(_) => {
                let terminal = self.coordination.as_deref_mut().map_or(
                    Ok(None),
                    FederateSchedulerCoordination::terminal_after_event_channel_closed,
                )?;
                Ok(WallClockReceive::FederateTerminated(
                    terminal.unwrap_or(FederateTermination::Abort),
                ))
            }
        }
    }
}

/// Zero-sized native clock policy using the system monotonic timer.
#[derive(Debug)]
pub(crate) struct NativeClock;

impl NativeClock {
    /// Synchronizes a logical tag to the host timer, preserving native interruption semantics.
    pub(crate) fn wait(
        &self,
        tag: Tag,
        wait: &mut WaitContext<'_, '_>,
    ) -> Result<WallClockReceive, FederateCoordinationError> {
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
                            observation
                                .enter(SchedulerPhase::PhysicalWait, std::time::Instant::now());
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

        Ok(WallClockReceive::DeadlineReached)
    }
}

/// Raw result from the scheduler event channel before an interruption is handled.
pub(crate) enum WallClockReceive {
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
) -> Result<WallClockReceive, FederateCoordinationError> {
    let advance = target.saturating_duration_since(std::time::Instant::now());
    entered_receive();
    match event_rx.recv_timeout(advance) {
        Ok(event) => Ok(WallClockReceive::Interrupted(event)),
        Err(ReceiveErrorTimeout::Closed) | Err(ReceiveErrorTimeout::SendClosed) => {
            if let Some(termination) = terminal_after_close()? {
                return Ok(WallClockReceive::FederateTerminated(termination));
            }
            if let Some(remaining) = target.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(remaining);
            }
            Ok(WallClockReceive::DeadlineReached)
        }
        Err(ReceiveErrorTimeout::Timeout) => Ok(WallClockReceive::DeadlineReached),
    }
}

#[cfg(test)]
mod wall_clock_tests {
    //! Exact receive-entry coverage for coordinated wall-clock termination.

    use super::{receive_until_wall_clock_deadline, WallClockReceive};
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
        use super::{NativeClock, WaitContext};
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
            NativeClock
                .wait(Tag::new(Duration::ZERO, 0), &mut wait)
                .unwrap(),
            WallClockReceive::DeadlineReached
        ));
        assert!(matches!(
            NativeClock
                .wait(Tag::new(Duration::seconds(60), 0), &mut wait)
                .unwrap(),
            WallClockReceive::Interrupted(AsyncEvent::FederateResume)
        ));
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
                WallClockReceive::FederateTerminated(FederateTermination::Abort)
            ));
            coordinator.join().unwrap().coordination_result.unwrap();
        });
    }
}
