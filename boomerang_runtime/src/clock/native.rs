//! Host-monotonic pacing for native execution.
use super::{ClockWaitResult, WaitContext};
use crate::{
    observation::SchedulerPhase,
    sched::federate::{
        FederateCoordinationError, FederateSchedulerCoordination, FederateTermination,
    },
    AsyncEvent, Tag,
};
use kanal::ReceiveErrorTimeout;

/// Synchronizes a tag to the system timer with the existing native receive behavior.
pub(super) fn wait_native(
    tag: Tag,
    wait: &mut WaitContext<'_, '_>,
) -> Result<ClockWaitResult, FederateCoordinationError> {
    let target = wait.origin + tag.offset();
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
        use crate::clock::{RuntimeClock, WaitContext};
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

    /// Native handles contain only shared origin ownership, with no hosted registry.
    #[cfg(not(feature = "external-clock"))]
    #[test]
    fn native_clock_handle_contains_only_shared_origin() {
        assert_eq!(
            std::mem::size_of::<crate::clock::RuntimeClock>(),
            std::mem::size_of::<usize>()
        );
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
