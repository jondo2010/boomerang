//! Federate-wide quiescence coordination across scheduler-owned Enclaves.

use std::{collections::BTreeMap, sync::mpsc};

use tinymap::TinySecondaryMap;

use crate::{AsyncEvent, EnclaveKey};

/// Minimal quiescence control observed by one scheduler-owned Enclave.
///
/// The scheduler core uses this hook without knowing the Federate-wide channel
/// protocol that coordinates every participating Enclave.
pub(crate) trait QuiescenceControl {
    /// Reports that queued or processed work invalidates an idle candidate.
    fn active(&mut self);
    /// Waits for queued work or a coordinator termination command.
    fn wait(&mut self) -> Option<AsyncEvent>;
    /// Returns the shared logical horizon after it wins Federate-wide termination.
    fn logical_horizon(&self) -> Option<crate::Tag>;
    /// Reports that this scheduler is processing the configured logical horizon.
    fn logical_horizon_reached(&mut self, tag: crate::Tag);
}

/// Scheduler-to-coordinator reports for one owned Federate's quiescence epoch.
enum QuiescenceReport {
    /// One Enclave observed queued or processed work that invalidates the idle candidate.
    Active(
        /// Canonical identity of the active scheduler-owned Enclave.
        EnclaveKey,
    ),
    /// One Enclave observed no queued work for the stated coordinator epoch.
    Idle {
        /// Canonical identity of the idle scheduler-owned Enclave.
        key: EnclaveKey,
        /// Quiescence epoch observed by this Enclave.
        epoch: usize,
    },
    /// One Enclave entered the parked barrier for the stated epoch.
    Parked {
        /// Canonical identity of the parked scheduler-owned Enclave.
        key: EnclaveKey,
        /// Quiescence epoch that this Enclave remains parked within.
        epoch: usize,
    },
    /// One parked Enclave confirmed its event queue remained empty for the stated epoch.
    Rechecked {
        /// Canonical identity of the rechecked scheduler-owned Enclave.
        key: EnclaveKey,
        /// Parked epoch whose final queue check was empty.
        epoch: usize,
    },
    /// One Enclave scheduler exited and no longer participates in quiescence coordination.
    Complete(
        /// Canonical identity of the completed scheduler-owned Enclave.
        EnclaveKey,
    ),
    /// An execution failure requires immediate Federate-wide abortion.
    Abort,
    /// One scheduler reached the shared logical horizon before Federate quiescence.
    LogicalHorizon(
        /// Shared horizon tag being processed by every scheduler-owned Enclave.
        crate::Tag,
    ),
}

/// Coordinator-to-scheduler commands for a parked quiescence barrier or abortion.
#[derive(Clone, Copy)]
enum QuiescenceCommand {
    /// Requests a fresh queue check for the stated quiescence epoch.
    Probe(
        /// Quiescence epoch to inspect.
        usize,
    ),
    /// Requires a scheduler to remain parked after confirming the stated epoch.
    Park(
        /// Quiescence epoch to park within.
        usize,
    ),
    /// Requests a final event-queue check while every scheduler remains parked.
    Recheck(
        /// Parked epoch to recheck.
        usize,
    ),
    /// Cancels the parked candidate and resumes every scheduler in a fresh epoch.
    Resume(
        /// Fresh quiescence epoch entered after resumption.
        usize,
    ),
    /// Commits termination from a unanimously empty parked state.
    Commit(
        /// Parked quiescence epoch being committed.
        usize,
    ),
    /// Stops the scheduler immediately after an execution failure.
    Abort,
    /// Releases parked schedulers to process their already queued shared logical horizon.
    LogicalHorizon(
        /// Shared horizon tag already queued by every scheduler-owned Enclave.
        crate::Tag,
    ),
}

/// Current phase of one owned Federate's channel-coordinated quiescence barrier.
#[derive(Clone, Copy, Eq, PartialEq)]
enum QuiescencePhase {
    /// Collecting ordinary idle observations before requesting an epoch confirmation.
    Observing,
    /// Awaiting fresh idle confirmations after a probe.
    Probing,
    /// Awaiting confirmation that every scheduler has entered the parked state.
    Parking,
    /// Awaiting final empty-queue confirmations while every scheduler remains parked.
    Rechecking,
    /// Awaiting fresh-epoch idle reports after every parked scheduler was resumed.
    Resuming,
    /// Termination was committed from the unanimously empty parked state.
    Committed,
}

/// Coordinator-owned phase flags for one dense Enclave identity.
#[derive(Clone, Copy, Default)]
struct CoordinatorEnclaveState {
    /// Whether the Enclave has not completed or aborted execution.
    active: bool,
    /// Whether the Enclave has reported work since its last idle report.
    busy: bool,
    /// Whether the Enclave has reported idle in the observing phase.
    seen_idle: bool,
    /// Whether the Enclave confirmed the coordinator's current command.
    confirmed: bool,
}

impl CoordinatorEnclaveState {
    /// Creates the initial state for one live scheduler participant.
    const fn active() -> Self {
        Self {
            active: true,
            busy: false,
            seen_idle: false,
            confirmed: false,
        }
    }
}

/// Abort handle for one owned Federate's shared quiescence coordinator.
pub(crate) struct FederateQuiescenceHandle {
    /// Shared participant-report sender used to request immediate abortion.
    report_tx: mpsc::Sender<QuiescenceReport>,
}

impl FederateQuiescenceHandle {
    /// Interrupts parked Enclave participants; event-channel closure remains abort-only.
    pub(crate) fn abort(&self) {
        let _ = self.report_tx.send(QuiescenceReport::Abort);
    }
}

/// Blocking Federate-wide coordinator that confirms an uninterrupted all-idle epoch.
pub(crate) struct FederateQuiescenceCoordinator {
    /// Serialized quiescence reports from every scheduler-owned Enclave.
    report_rx: mpsc::Receiver<QuiescenceReport>,
    /// Per-Enclave command senders keyed by canonical scheduler identity.
    commands: TinySecondaryMap<EnclaveKey, mpsc::Sender<QuiescenceCommand>>,
    /// Per-Enclave active membership and phase flags.
    active: TinySecondaryMap<EnclaveKey, CoordinatorEnclaveState>,
    #[cfg(test)]
    /// One-shot test hook that admits work immediately before the final parked queue recheck.
    commit_window_hook: Option<Box<dyn FnOnce() + Send>>,
}

impl FederateQuiescenceCoordinator {
    /// Runs until all scheduler-owned Enclaves complete or abort is requested.
    pub(crate) fn run(self) {
        let Self {
            report_rx,
            commands,
            mut active,
            #[cfg(test)]
            mut commit_window_hook,
        } = self;
        let send = |command, active: &TinySecondaryMap<EnclaveKey, CoordinatorEnclaveState>| {
            for (key, tx) in commands.iter() {
                if active.get(key).is_some_and(|state| state.active) {
                    let _ = tx.send(command);
                }
            }
        };
        let any_active = |active: &TinySecondaryMap<EnclaveKey, CoordinatorEnclaveState>| {
            active.values().any(|state| state.active)
        };
        let all_confirmed = |active: &TinySecondaryMap<EnclaveKey, CoordinatorEnclaveState>| {
            active.values().all(|state| state.confirmed == state.active)
        };
        let clear_confirmed =
            |active: &mut TinySecondaryMap<EnclaveKey, CoordinatorEnclaveState>| {
                active
                    .iter_mut()
                    .for_each(|(_, state)| state.confirmed = false);
            };
        let mut epoch = 0usize;
        let mut phase = QuiescencePhase::Observing;
        while any_active(&active) {
            let report = match report_rx.recv() {
                Ok(report) => report,
                Err(_) => {
                    send(QuiescenceCommand::Abort, &active);
                    break;
                }
            };
            match report {
                QuiescenceReport::Abort => {
                    send(QuiescenceCommand::Abort, &active);
                    break;
                }
                QuiescenceReport::LogicalHorizon(tag) => {
                    send(QuiescenceCommand::LogicalHorizon(tag), &active);
                    break;
                }
                QuiescenceReport::Complete(key) => {
                    if let Some(state) = active.get_mut(key) {
                        *state = CoordinatorEnclaveState::default();
                    }
                }
                QuiescenceReport::Active(key) => {
                    if let Some(state) = active.get_mut(key) {
                        state.busy = true;
                        state.seen_idle = false;
                    }
                    clear_confirmed(&mut active);
                    if matches!(
                        phase,
                        QuiescencePhase::Parking | QuiescencePhase::Rechecking
                    ) {
                        epoch = epoch.wrapping_add(1);
                        active
                            .iter_mut()
                            .for_each(|(_, state)| state.seen_idle = false);
                        send(QuiescenceCommand::Resume(epoch), &active);
                        phase = QuiescencePhase::Resuming;
                    } else if phase != QuiescencePhase::Committed {
                        phase = QuiescencePhase::Observing;
                    }
                }
                QuiescenceReport::Idle {
                    key,
                    epoch: observed,
                } => {
                    if let Some(state) = active.get_mut(key) {
                        state.busy = false;
                        if phase == QuiescencePhase::Observing {
                            state.seen_idle = true;
                        } else if matches!(
                            phase,
                            QuiescencePhase::Probing | QuiescencePhase::Resuming
                        ) && observed == epoch
                            && state.active
                        {
                            state.confirmed = true;
                        }
                    }
                }
                QuiescenceReport::Parked {
                    key,
                    epoch: observed,
                } if phase == QuiescencePhase::Parking
                    && observed == epoch
                    && active.get(key).is_some_and(|state| state.active) =>
                {
                    active[key].confirmed = true;
                }
                QuiescenceReport::Rechecked {
                    key,
                    epoch: observed,
                } if phase == QuiescencePhase::Rechecking
                    && observed == epoch
                    && active.get(key).is_some_and(|state| state.active) =>
                {
                    active[key].confirmed = true;
                }
                QuiescenceReport::Parked { .. } | QuiescenceReport::Rechecked { .. } => {}
            }
            if !any_active(&active) {
                continue;
            }
            match phase {
                QuiescencePhase::Observing
                    if active.values().all(|state| !state.busy)
                        && active.values().all(|state| state.seen_idle == state.active) =>
                {
                    epoch = epoch.wrapping_add(1);
                    clear_confirmed(&mut active);
                    send(QuiescenceCommand::Probe(epoch), &active);
                    phase = QuiescencePhase::Probing;
                }
                QuiescencePhase::Probing if all_confirmed(&active) => {
                    clear_confirmed(&mut active);
                    send(QuiescenceCommand::Park(epoch), &active);
                    phase = QuiescencePhase::Parking;
                }
                QuiescencePhase::Parking if all_confirmed(&active) => {
                    #[cfg(test)]
                    if let Some(hook) = commit_window_hook.take() {
                        hook();
                    }
                    clear_confirmed(&mut active);
                    send(QuiescenceCommand::Recheck(epoch), &active);
                    phase = QuiescencePhase::Rechecking;
                }
                QuiescencePhase::Rechecking if all_confirmed(&active) => {
                    send(QuiescenceCommand::Commit(epoch), &active);
                    clear_confirmed(&mut active);
                    phase = QuiescencePhase::Committed;
                }
                QuiescencePhase::Resuming if all_confirmed(&active) => {
                    clear_confirmed(&mut active);
                    send(QuiescenceCommand::Park(epoch), &active);
                    phase = QuiescencePhase::Parking;
                }
                _ => {}
            }
        }
    }
}

/// Per-Enclave participant in one owned Federate's shared quiescence protocol.
pub(crate) struct QuiescenceParticipant {
    /// Canonical identity of this scheduler-owned Enclave.
    key: EnclaveKey,
    /// Most recent quiescence-probe epoch received from the coordinator.
    epoch: usize,
    /// Whether work has been reported since this scheduler last became idle.
    active: bool,
    /// Quiescence epoch while this scheduler is held inside the parked barrier.
    parked_epoch: Option<usize>,
    /// Shared channel used to report scheduler activity and completion.
    report_tx: mpsc::Sender<QuiescenceReport>,
    /// Dedicated coordinator-command receiver for this scheduler-owned Enclave.
    command_rx: mpsc::Receiver<QuiescenceCommand>,
    /// Scheduler event queue inspected before confirming idleness or termination.
    event_rx: crate::Receiver<AsyncEvent>,
    /// Shared logical horizon committed by the coordinator, if it won termination.
    logical_horizon: Option<crate::Tag>,
}

impl QuiescenceParticipant {
    /// Reports this scheduler-owned Enclave's idle observation for its current epoch.
    fn report_idle(&self) {
        let _ = self.report_tx.send(QuiescenceReport::Idle {
            key: self.key,
            epoch: self.epoch,
        });
    }

    /// Reports activity once until this scheduler-owned Enclave next begins an idle wait.
    fn report_active(&mut self) {
        if !self.active {
            let _ = self.report_tx.send(QuiescenceReport::Active(self.key));
            self.active = true;
        }
    }

    /// Removes one queued scheduler event without leaving a parked barrier independently.
    fn take_queued_event(&self) -> Option<AsyncEvent> {
        self.event_rx.try_recv().ok().flatten()
    }
}

impl QuiescenceControl for QuiescenceParticipant {
    fn active(&mut self) {
        self.parked_epoch = None;
        self.report_active();
    }

    fn wait(&mut self) -> Option<AsyncEvent> {
        self.active = false;
        self.parked_epoch = None;
        self.report_idle();
        let mut parked_event = None;
        loop {
            if self.parked_epoch.is_some() {
                if parked_event.is_none() {
                    if let Some(event) = self.take_queued_event() {
                        self.report_active();
                        parked_event = Some(event);
                    }
                }
            } else if let Some(event) = self.take_queued_event() {
                self.active();
                return Some(event);
            }
            match self
                .command_rx
                .recv_timeout(std::time::Duration::from_millis(1))
            {
                Ok(QuiescenceCommand::Probe(epoch)) => {
                    self.parked_epoch = None;
                    self.epoch = epoch;
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.report_idle();
                }
                Ok(QuiescenceCommand::Park(epoch)) => {
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.parked_epoch = Some(epoch);
                    let _ = self.report_tx.send(QuiescenceReport::Parked {
                        key: self.key,
                        epoch,
                    });
                }
                Ok(QuiescenceCommand::Recheck(epoch)) if self.parked_epoch == Some(epoch) => {
                    if parked_event.is_none() {
                        if let Some(event) = self.take_queued_event() {
                            self.report_active();
                            parked_event = Some(event);
                        }
                    }
                    if parked_event.is_none() {
                        let _ = self.report_tx.send(QuiescenceReport::Rechecked {
                            key: self.key,
                            epoch,
                        });
                    }
                }
                Ok(QuiescenceCommand::Resume(epoch)) => {
                    self.parked_epoch = None;
                    self.epoch = epoch;
                    if let Some(event) = parked_event.take() {
                        return Some(event);
                    }
                    if let Some(event) = self.take_queued_event() {
                        self.active();
                        return Some(event);
                    }
                    self.active = false;
                    self.report_idle();
                }
                Ok(QuiescenceCommand::Commit(epoch)) if self.parked_epoch == Some(epoch) => {
                    return None;
                }
                Ok(QuiescenceCommand::LogicalHorizon(tag)) => {
                    self.logical_horizon = Some(tag);
                    return None;
                }
                Ok(QuiescenceCommand::Recheck(_)) | Ok(QuiescenceCommand::Commit(_)) => {}
                Ok(QuiescenceCommand::Abort) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn logical_horizon(&self) -> Option<crate::Tag> {
        self.logical_horizon
    }

    fn logical_horizon_reached(&mut self, tag: crate::Tag) {
        let _ = self.report_tx.send(QuiescenceReport::LogicalHorizon(tag));
    }
}

impl Drop for QuiescenceParticipant {
    fn drop(&mut self) {
        let _ = self.report_tx.send(QuiescenceReport::Complete(self.key));
    }
}

/// Federate-wide quiescence resources shared by all of its scheduler-owned Enclaves.
pub(crate) struct FederateQuiescence {
    /// Handle used to abort every participating Enclave scheduler after execution failure.
    pub(crate) abort_handle: FederateQuiescenceHandle,
    /// Coordinator that commits termination only after every Enclave remains parked and empty.
    pub(crate) coordinator: FederateQuiescenceCoordinator,
    /// Per-Enclave participants keyed by canonical scheduler identity.
    pub(crate) participants: BTreeMap<EnclaveKey, QuiescenceParticipant>,
}

impl FederateQuiescence {
    /// Creates one Federate-wide coordinator and one participant per scheduler-owned Enclave.
    pub(crate) fn new(
        keys: impl IntoIterator<Item = (EnclaveKey, crate::Receiver<AsyncEvent>)>,
    ) -> Self {
        let (report_tx, report_rx) = mpsc::channel();
        let mut commands = TinySecondaryMap::new();
        let mut active = TinySecondaryMap::new();
        let mut participants = BTreeMap::new();
        for (key, event_rx) in keys {
            let (command_tx, command_rx) = mpsc::channel();
            commands.insert(key, command_tx);
            active.insert(key, CoordinatorEnclaveState::active());
            participants.insert(
                key,
                QuiescenceParticipant {
                    key,
                    epoch: 0,
                    active: false,
                    parked_epoch: None,
                    report_tx: report_tx.clone(),
                    command_rx,
                    event_rx,
                    logical_horizon: None,
                },
            );
        }
        Self {
            abort_handle: FederateQuiescenceHandle {
                report_tx: report_tx.clone(),
            },
            coordinator: FederateQuiescenceCoordinator {
                report_rx,
                commands,
                active,
                #[cfg(test)]
                commit_window_hook: None,
            },
            participants,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FederateQuiescence, QuiescenceControl};
    use crate::{AsyncEvent, Duration, EnclaveKey};
    use std::{sync::mpsc, time::Duration as StdDuration};

    #[test]
    fn parked_barrier_resumes_all_schedulers_when_work_arrives_before_commit() {
        let eventful = EnclaveKey::from(3);
        let peer = EnclaveKey::from(7);
        let (eventful_tx, eventful_rx) = kanal::unbounded();
        let (_peer_tx, peer_rx) = kanal::unbounded();
        let FederateQuiescence {
            abort_handle,
            mut coordinator,
            mut participants,
        } = FederateQuiescence::new([(eventful, eventful_rx), (peer, peer_rx)]);
        coordinator.commit_window_hook = Some(Box::new(move || {
            eventful_tx
                .send(AsyncEvent::Shutdown {
                    delay: Duration::ZERO,
                })
                .unwrap();
        }));
        let mut eventful_participant = participants.remove(&eventful).unwrap();
        let mut peer_participant = participants.remove(&peer).unwrap();
        let (result_tx, result_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let coordinator_handle = scope.spawn(move || coordinator.run());
            let eventful_result_tx = result_tx.clone();
            let eventful_handle = scope.spawn(move || {
                eventful_result_tx
                    .send((eventful, eventful_participant.wait()))
                    .unwrap();
                eventful_result_tx
                    .send((eventful, eventful_participant.wait()))
                    .unwrap();
            });
            let peer_result_tx = result_tx.clone();
            let peer_handle = scope.spawn(move || {
                peer_result_tx
                    .send((peer, peer_participant.wait()))
                    .unwrap();
            });
            drop(result_tx);

            let mut observations = Vec::new();
            for _ in 0..3 {
                match result_rx.recv_timeout(StdDuration::from_secs(1)) {
                    Ok(observation) => observations.push(observation),
                    Err(_) => {
                        abort_handle.abort();
                        break;
                    }
                }
            }
            eventful_handle.join().unwrap();
            peer_handle.join().unwrap();
            coordinator_handle.join().unwrap();

            assert_eq!(
                observations.len(),
                3,
                "both parked schedulers must reach a later common commit"
            );
            assert!(observations.iter().any(|(key, event)| {
                *key == eventful
                    && matches!(
                        event,
                        Some(AsyncEvent::Shutdown { delay }) if *delay == Duration::ZERO
                    )
            }));
            assert!(observations
                .iter()
                .any(|(key, event)| *key == eventful && event.is_none()));
            assert!(observations
                .iter()
                .any(|(key, event)| *key == peer && event.is_none()));
        });
    }
}
