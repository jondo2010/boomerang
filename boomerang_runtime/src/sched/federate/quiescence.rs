//! Blocking channel adapter for one compiled Federate's coordination state.
//!
//! Participants stamp stored compiled Enclave identities onto scheduler messages and inspect only
//! their own event queues. The coordinator serializes those messages through the pure state,
//! executes its semantic actions against a selected backend, and owns no scheduling decisions.

use std::{collections::BTreeMap, sync::mpsc, time::Duration as StdDuration};

use tinymap::TinySecondaryMap;

use super::{
    backend::{
        CoordinationRevision, FederateCoordinationBackend, FederateCoordinationError,
        LocalFederateCoordinationBackend,
    },
    state::{
        CoordinationAction, CoordinationStateError, FederateCoordinationState, LifecyclePolicy,
        Observation, SchedulerMessage,
    },
};
use crate::{image::EnclaveIndex, AsyncEvent, Tag};

/// Scheduler-facing coordination port without a backend or transport type.
pub(crate) trait FederateSchedulerCoordination {
    /// Reports work observed before the scheduler can publish its revised candidate.
    fn active(&mut self);

    /// Publishes no local candidate and waits for asynchronous work or terminal coordination.
    fn wait(&mut self) -> Result<Option<AsyncEvent>, FederateCoordinationError>;

    /// Publishes a local candidate and waits for its grant or an asynchronous interruption.
    #[allow(dead_code)]
    fn acquire_tag(&mut self, tag: Tag) -> Result<Option<AsyncEvent>, FederateCoordinationError>;

    /// Reports completion of one processed logical tag.
    #[allow(dead_code)]
    fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederateCoordinationError>;

    /// Returns the legacy shared logical horizon, when one ended coordination.
    fn logical_horizon(&self) -> Option<Tag>;

    /// Reports that the legacy shared logical horizon is being processed.
    fn logical_horizon_reached(&mut self, tag: Tag);

    /// Requests planned Federate-wide stop.
    #[allow(dead_code)]
    fn stop(&mut self);

    /// Reports scheduler failure and requests Federate-wide abortion.
    fn fail(&mut self);
}

/// Temporary infallible scheduler seam retained until Task 5 generalizes `SchedulerCore`.
pub(crate) trait QuiescenceControl {
    /// Reports work observed by the legacy compiled scheduler.
    fn active(&mut self);

    /// Waits for asynchronous work or legacy quiescence termination.
    fn wait(&mut self) -> Option<AsyncEvent>;

    /// Returns the legacy shared logical horizon, when one ended coordination.
    fn logical_horizon(&self) -> Option<Tag>;

    /// Reports that the legacy shared logical horizon is being processed.
    fn logical_horizon_reached(&mut self, tag: Tag);
}

/// Participant-to-coordinator messages that retain only typed state inputs and legacy requests.
enum CoordinatorReport {
    /// One scheduler-originated pure-state message.
    Scheduler(
        /// Typed scheduler message stamped by the participant.
        SchedulerMessage,
    ),
    /// One revision-bound fixed-point observation.
    Observation {
        /// Compiled participant supplying the observation.
        enclave: EnclaveIndex,
        /// Candidate revision observed by the participant.
        revision: CoordinationRevision,
        /// Queue or parking observation for that revision.
        observation: Observation,
    },
    /// Planned stop requested by one compiled participant.
    #[allow(dead_code)]
    Stop {
        /// Compiled participant requesting stop.
        enclave: EnclaveIndex,
    },
    /// Legacy shared logical horizon reached by one compiled participant.
    LogicalHorizon {
        /// Compiled participant reporting the horizon.
        enclave: EnclaveIndex,
        /// Shared terminal logical tag.
        tag: Tag,
    },
}

/// Coordinator-to-participant operations translated from pure coordination actions.
#[derive(Clone, Copy)]
enum ParticipantCommand {
    /// Pure action addressed to one or every participant.
    Action(
        /// Grant, fixed-point, resume, or terminal action selected by the pure state.
        CoordinationAction,
    ),
    /// Releases legacy participants at their shared logical horizon.
    LogicalHorizon(
        /// Shared terminal logical tag.
        Tag,
    ),
}

/// Maps private state failures into the closed public coordination taxonomy.
impl From<CoordinationStateError> for FederateCoordinationError {
    /// Converts one private construction or transition failure without exposing phase types.
    fn from(error: CoordinationStateError) -> Self {
        match error {
            CoordinationStateError::NoParticipants => Self::NoParticipants,
            CoordinationStateError::DuplicateEnclave { enclave } => {
                Self::DuplicateEnclave { enclave }
            }
            CoordinationStateError::UnknownEnclave { enclave } => Self::UnknownEnclave { enclave },
            CoordinationStateError::InvalidObservationTransition { .. } => {
                Self::InvalidObservationTransition
            }
            CoordinationStateError::ParticipantStoppedBeforeTerminal { enclave } => {
                Self::ParticipantStoppedBeforeTerminal { enclave }
            }
        }
    }
}

/// Supervisor handle for requesting Federate-wide abortion.
pub(crate) struct FederateCoordinationHandle {
    /// Shared coordinator-report sender used for the terminal failure message.
    report_tx: mpsc::Sender<CoordinatorReport>,
}

impl FederateCoordinationHandle {
    /// Requests idempotent Federate-wide abortion without blocking the supervising thread.
    pub(crate) fn abort(&self) {
        let _ = self
            .report_tx
            .send(CoordinatorReport::Scheduler(SchedulerMessage::Failed {
                enclave: None,
            }));
    }
}

/// Blocking coordinator that serializes participant messages through the pure state.
pub(crate) struct FederateCoordinator<B: FederateCoordinationBackend> {
    /// Serialized reports from every compiled participant.
    report_rx: mpsc::Receiver<CoordinatorReport>,
    /// Per-participant command senders keyed by compiled identity.
    commands: TinySecondaryMap<EnclaveIndex, mpsc::Sender<ParticipantCommand>>,
    /// Authoritative candidate, phase, completion, and terminal state.
    state: FederateCoordinationState,
    /// Selected transport-neutral coordination backend.
    backend: B,
    /// Finite publication revision awaiting a backend acquisition.
    pending_acquisition: Option<CoordinationRevision>,
    #[cfg(test)]
    /// One-shot queue-race hook immediately before the final parked recheck.
    commit_window_hook: Option<Box<dyn FnOnce() + Send>>,
}

impl<B: FederateCoordinationBackend> FederateCoordinator<B> {
    /// Runs until pure coordination stops or returns its first backend, state, or channel failure.
    pub(crate) fn run(mut self) -> Result<(), FederateCoordinationError> {
        loop {
            let outcome = if self.pending_acquisition.is_some() {
                match self.report_rx.recv_timeout(StdDuration::from_millis(1)) {
                    Ok(report) => self.handle_report(report),
                    Err(mpsc::RecvTimeoutError::Timeout) => self.poll_backend(),
                    Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                        FederateCoordinationError::CoordinatorReportChannelDisconnected {
                            enclave: None,
                        },
                    ),
                }
            } else {
                self.report_rx
                    .recv()
                    .map_err(
                        |_| FederateCoordinationError::CoordinatorReportChannelDisconnected {
                            enclave: None,
                        },
                    )
                    .and_then(|report| self.handle_report(report))
            };

            match outcome {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => {
                    self.abort_after_error();
                    return Err(error);
                }
            }
        }
    }

    /// Applies one channel report and executes the pure actions it produces.
    fn handle_report(
        &mut self,
        report: CoordinatorReport,
    ) -> Result<bool, FederateCoordinationError> {
        match report {
            CoordinatorReport::Scheduler(message) => {
                let prior_revision = self.state.revision();
                let actions = self.state.handle_scheduler(message)?;
                if self.state.revision() != prior_revision {
                    self.pending_acquisition = None;
                }
                self.execute_actions(actions)
            }
            CoordinatorReport::Observation {
                enclave,
                revision,
                observation,
            } => {
                let actions = self
                    .state
                    .handle_observation(enclave, revision, observation)?;
                self.execute_actions(actions)
            }
            CoordinatorReport::Stop { enclave } => {
                self.require_participant(enclave)?;
                self.pending_acquisition = None;
                let actions = self.state.stop();
                self.execute_actions(actions)
            }
            CoordinatorReport::LogicalHorizon { enclave, tag } => {
                self.require_participant(enclave)?;
                self.pending_acquisition = None;
                let _ = self.state.stop();
                let backend_result = self.backend.stop();
                self.send_available(ParticipantCommand::LogicalHorizon(tag));
                backend_result?;
                Ok(true)
            }
        }
    }

    /// Validates a compiled identity before an adapter-only terminal request.
    fn require_participant(&self, enclave: EnclaveIndex) -> Result<(), FederateCoordinationError> {
        self.state
            .candidate(enclave)
            .is_some()
            .then_some(())
            .ok_or(FederateCoordinationError::UnknownEnclave { enclave })
    }

    /// Executes pure actions mechanically against the backend or participant channels.
    fn execute_actions(
        &mut self,
        actions: Vec<CoordinationAction>,
    ) -> Result<bool, FederateCoordinationError> {
        for action in actions {
            match action {
                CoordinationAction::Publish(publication) => {
                    self.backend.publish(publication)?;
                    self.pending_acquisition =
                        publication.next_event().map(|_| publication.revision());
                }
                action @ CoordinationAction::Grant { enclave, .. } => {
                    self.send_one(enclave, ParticipantCommand::Action(action))?;
                }
                CoordinationAction::Complete(completion) => self.backend.complete(completion)?,
                action @ (CoordinationAction::Probe { .. } | CoordinationAction::Park { .. }) => {
                    self.send_all(ParticipantCommand::Action(action))?;
                }
                action @ CoordinationAction::Recheck { .. } => {
                    #[cfg(test)]
                    if let Some(hook) = self.commit_window_hook.take() {
                        hook();
                    }
                    self.send_all(ParticipantCommand::Action(action))?;
                }
                action @ CoordinationAction::Resume { .. } => {
                    self.send_available(ParticipantCommand::Action(action));
                }
                action @ (CoordinationAction::Stop | CoordinationAction::Abort) => {
                    self.pending_acquisition = None;
                    let backend_result = self.backend.stop();
                    self.send_available(ParticipantCommand::Action(action));
                    backend_result?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Polls one outstanding publication and applies an acquired grant through the pure state.
    fn poll_backend(&mut self) -> Result<bool, FederateCoordinationError> {
        let Some(pending) = self.pending_acquisition else {
            return Ok(false);
        };
        let Some(acquisition) = self.backend.poll_acquisition(StdDuration::from_millis(1))? else {
            return Ok(false);
        };
        if acquisition.revision() == pending {
            self.pending_acquisition = None;
        }
        let actions = self.state.handle_acquisition(acquisition)?;
        self.execute_actions(actions)
    }

    /// Sends one translated action to every compiled participant.
    fn send_all(&self, command: ParticipantCommand) -> Result<(), FederateCoordinationError> {
        for (enclave, sender) in self.commands.iter() {
            sender.send(command).map_err(|_| {
                FederateCoordinationError::ParticipantCommandChannelDisconnected { enclave }
            })?;
        }
        Ok(())
    }

    /// Sends one participant-specific translated action.
    fn send_one(
        &self,
        enclave: EnclaveIndex,
        command: ParticipantCommand,
    ) -> Result<(), FederateCoordinationError> {
        self.commands
            .get(enclave)
            .ok_or(FederateCoordinationError::UnknownEnclave { enclave })?
            .send(command)
            .map_err(
                |_| FederateCoordinationError::ParticipantCommandChannelDisconnected { enclave },
            )
    }

    /// Best-effort wakeup for participants that may already have returned from a blocking call.
    fn send_available(&self, command: ParticipantCommand) {
        for sender in self.commands.values() {
            let _ = sender.send(command);
        }
    }

    /// Wakes participants and stops the backend without replacing the first returned error.
    fn abort_after_error(&mut self) {
        self.pending_acquisition = None;
        let _ = self
            .state
            .handle_scheduler(SchedulerMessage::Failed { enclave: None });
        let _ = self.backend.stop();
        self.send_available(ParticipantCommand::Action(CoordinationAction::Abort));
    }
}

/// Per-scheduler channel adapter keyed by its stored compiled Enclave identity.
pub(crate) struct FederateCoordinationParticipant {
    /// Compiled identity stamped onto every state message.
    enclave: EnclaveIndex,
    /// Shared participant-report sender.
    report_tx: mpsc::Sender<CoordinatorReport>,
    /// Dedicated coordinator-command receiver.
    command_rx: mpsc::Receiver<ParticipantCommand>,
    /// Scheduler event receiver inspected only by this participant.
    event_rx: crate::Receiver<AsyncEvent>,
    /// Revision in which this participant entered the parked barrier.
    parked_revision: Option<CoordinationRevision>,
    /// Legacy shared logical horizon, when one ended coordination.
    logical_horizon: Option<Tag>,
    /// First report-channel error produced by an infallible compatibility method.
    deferred_error: Option<FederateCoordinationError>,
    /// Whether this participant observed terminal coordinator release.
    terminal: bool,
}

impl FederateCoordinationParticipant {
    /// Publishes successful terminal idleness and remains available for fixed-point commands.
    pub(crate) fn finish_success(&mut self) -> Result<(), FederateCoordinationError> {
        while !self.terminal {
            let _ = FederateSchedulerCoordination::wait(self)?;
        }
        Ok(())
    }

    /// Sends one participant report or returns a typed channel error.
    fn report(&self, report: CoordinatorReport) -> Result<(), FederateCoordinationError> {
        self.report_tx.send(report).map_err(|_| {
            FederateCoordinationError::CoordinatorReportChannelDisconnected {
                enclave: Some(self.enclave),
            }
        })
    }

    /// Retains a channel error produced by a trait method whose approved signature is infallible.
    fn report_infallible(&mut self, report: CoordinatorReport) {
        if self.deferred_error.is_none() {
            self.deferred_error = self.report(report).err();
        }
    }

    /// Returns any deferred channel error before beginning a fallible operation.
    fn take_deferred_error(&mut self) -> Result<(), FederateCoordinationError> {
        self.deferred_error.take().map_or(Ok(()), Err)
    }

    /// Reports a revision-bound fixed-point observation.
    fn observe(
        &self,
        revision: CoordinationRevision,
        observation: Observation,
    ) -> Result<(), FederateCoordinationError> {
        self.report(CoordinatorReport::Observation {
            enclave: self.enclave,
            revision,
            observation,
        })
    }

    /// Takes queued work and reports that it invalidates an idle or fixed-point candidate.
    fn take_active_event(&self) -> Result<Option<AsyncEvent>, FederateCoordinationError> {
        let event = self.event_rx.try_recv().ok().flatten();
        if event.is_some() {
            self.report(CoordinatorReport::Scheduler(SchedulerMessage::Active {
                enclave: self.enclave,
            }))?;
        }
        Ok(event)
    }
}

impl FederateSchedulerCoordination for FederateCoordinationParticipant {
    /// Reports work without inventing a candidate tag.
    fn active(&mut self) {
        self.report_infallible(CoordinatorReport::Scheduler(SchedulerMessage::Active {
            enclave: self.enclave,
        }));
    }

    /// Publishes idle and blocks while translating fixed-point commands around the event queue.
    fn wait(&mut self) -> Result<Option<AsyncEvent>, FederateCoordinationError> {
        self.take_deferred_error()?;
        self.parked_revision = None;
        self.report(CoordinatorReport::Scheduler(SchedulerMessage::Publish {
            enclave: self.enclave,
            next_event: None,
        }))?;

        loop {
            if let Some(event) = self.take_active_event()? {
                return Ok(Some(event));
            }
            match self.command_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(ParticipantCommand::Action(CoordinationAction::Probe { revision })) => {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(Some(event));
                    }
                    self.observe(revision, Observation::Probed)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Park { revision })) => {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(Some(event));
                    }
                    self.parked_revision = Some(revision);
                    self.observe(revision, Observation::Parked)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Recheck { revision }))
                    if self.parked_revision == Some(revision) =>
                {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(Some(event));
                    }
                    self.observe(revision, Observation::Rechecked)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Resume { .. })) => {
                    self.parked_revision = None;
                }
                Ok(ParticipantCommand::LogicalHorizon(tag)) => {
                    self.logical_horizon = Some(tag);
                    self.terminal = true;
                    return Ok(None);
                }
                Ok(ParticipantCommand::Action(
                    CoordinationAction::Stop | CoordinationAction::Abort,
                )) => {
                    self.terminal = true;
                    return Ok(None);
                }
                Ok(ParticipantCommand::Action(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(
                        FederateCoordinationError::ParticipantCommandChannelDisconnected {
                            enclave: self.enclave,
                        },
                    );
                }
            }
        }
    }

    /// Publishes a finite candidate and blocks until it is granted or asynchronously interrupted.
    fn acquire_tag(&mut self, tag: Tag) -> Result<Option<AsyncEvent>, FederateCoordinationError> {
        self.take_deferred_error()?;
        self.report(CoordinatorReport::Scheduler(SchedulerMessage::Publish {
            enclave: self.enclave,
            next_event: Some(tag),
        }))?;

        loop {
            if let Some(event) = self.take_active_event()? {
                return Ok(Some(event));
            }
            match self.command_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(ParticipantCommand::Action(CoordinationAction::Grant {
                    tag: granted, ..
                })) if granted >= tag => {
                    return Ok(None);
                }
                Ok(ParticipantCommand::LogicalHorizon(tag)) => {
                    self.logical_horizon = Some(tag);
                    self.terminal = true;
                    return Ok(None);
                }
                Ok(ParticipantCommand::Action(
                    CoordinationAction::Stop | CoordinationAction::Abort,
                )) => {
                    self.terminal = true;
                    return Ok(None);
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(
                        FederateCoordinationError::ParticipantCommandChannelDisconnected {
                            enclave: self.enclave,
                        },
                    );
                }
            }
        }
    }

    /// Reports logical completion through the coordinator so backend failures remain supervised.
    fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederateCoordinationError> {
        self.take_deferred_error()?;
        self.report(CoordinatorReport::Scheduler(
            SchedulerMessage::CompleteTag {
                enclave: self.enclave,
                tag,
            },
        ))
    }

    /// Returns the retained legacy shared logical horizon.
    fn logical_horizon(&self) -> Option<Tag> {
        self.logical_horizon
    }

    /// Reports a legacy shared logical horizon without changing key domains.
    fn logical_horizon_reached(&mut self, tag: Tag) {
        self.report_infallible(CoordinatorReport::LogicalHorizon {
            enclave: self.enclave,
            tag,
        });
    }

    /// Requests pure-state stop.
    fn stop(&mut self) {
        self.report_infallible(CoordinatorReport::Stop {
            enclave: self.enclave,
        });
    }

    /// Reports a typed participant-origin failure.
    fn fail(&mut self) {
        self.report_infallible(CoordinatorReport::Scheduler(SchedulerMessage::Failed {
            enclave: Some(self.enclave),
        }));
    }
}

impl QuiescenceControl for FederateCoordinationParticipant {
    /// Forwards legacy activity to the generalized scheduler port.
    fn active(&mut self) {
        FederateSchedulerCoordination::active(self);
    }

    /// Forwards legacy waiting and exposes impossible channel loss as a supervisor-visible panic.
    fn wait(&mut self) -> Option<AsyncEvent> {
        FederateSchedulerCoordination::wait(self)
            .unwrap_or_else(|error| panic!("legacy Federate coordination failed: {error}"))
    }

    /// Returns the horizon retained by the generalized scheduler port.
    fn logical_horizon(&self) -> Option<Tag> {
        FederateSchedulerCoordination::logical_horizon(self)
    }

    /// Forwards the legacy logical-horizon report to the generalized scheduler port.
    fn logical_horizon_reached(&mut self, tag: Tag) {
        FederateSchedulerCoordination::logical_horizon_reached(self, tag);
    }
}

/// Coordinator, supervisor handle, and scheduler participants for one compiled Federate.
pub(crate) struct FederateCoordination<B: FederateCoordinationBackend> {
    /// Supervisor handle used to request Federate-wide abortion.
    pub(crate) abort_handle: FederateCoordinationHandle,
    /// Blocking coordinator that owns the pure state and selected backend.
    pub(crate) coordinator: FederateCoordinator<B>,
    /// Participants keyed directly by their compiled Enclave identities.
    pub(crate) participants: BTreeMap<EnclaveIndex, FederateCoordinationParticipant>,
}

impl<B: FederateCoordinationBackend> FederateCoordination<B> {
    /// Creates one participant per unique compiled identity and one shared coordinator.
    pub(crate) fn new(
        participants: impl IntoIterator<Item = (EnclaveIndex, crate::Receiver<AsyncEvent>)>,
        lifecycle: LifecyclePolicy,
        backend: B,
    ) -> Result<Self, FederateCoordinationError> {
        let participants = participants.into_iter().collect::<Vec<_>>();
        let state = FederateCoordinationState::new(
            participants.iter().map(|(enclave, _)| *enclave),
            lifecycle,
        )?;
        let (report_tx, report_rx) = mpsc::channel();
        let mut commands = TinySecondaryMap::new();
        let mut ports = BTreeMap::new();
        for (enclave, event_rx) in participants {
            let (command_tx, command_rx) = mpsc::channel();
            commands.insert(enclave, command_tx);
            ports.insert(
                enclave,
                FederateCoordinationParticipant {
                    enclave,
                    report_tx: report_tx.clone(),
                    command_rx,
                    event_rx,
                    parked_revision: None,
                    logical_horizon: None,
                    deferred_error: None,
                    terminal: false,
                },
            );
        }
        Ok(Self {
            abort_handle: FederateCoordinationHandle {
                report_tx: report_tx.clone(),
            },
            coordinator: FederateCoordinator {
                report_rx,
                commands,
                state,
                backend,
                pending_acquisition: None,
                #[cfg(test)]
                commit_window_hook: None,
            },
            participants: ports,
        })
    }
}

/// Temporary local-backend name retained for reference construction until Task 6.
pub(crate) type FederateQuiescence = FederateCoordination<LocalFederateCoordinationBackend>;

/// Temporary supervisor-handle name retained for reference supervision until Task 6.
pub(crate) type FederateQuiescenceHandle = FederateCoordinationHandle;

/// Temporary participant name retained by the compiled scheduler seam until Task 5.
pub(crate) type QuiescenceParticipant = FederateCoordinationParticipant;

#[cfg(test)]
mod tests {
    //! Channel-level tests for wakeable idle execution and the parked queue race.

    use super::super::state::CoordinationPhase;
    use super::{
        CoordinationStateError, CoordinatorReport, FederateCoordination,
        FederateCoordinationParticipant, FederateSchedulerCoordination, Observation,
        ParticipantCommand, SchedulerMessage,
    };
    use crate::{
        image::EnclaveIndex,
        sched::federate::{
            backend::{
                FederateAcquisition, FederateCompletion, FederateCoordinationBackend,
                FederateCoordinationError, FederatePublication, LocalFederateCoordinationBackend,
            },
            LifecyclePolicy,
        },
        AsyncEvent, Duration, Tag,
    };
    use std::{sync::mpsc, time::Duration as StdDuration};

    /// Local backend wrapper that signals an observed no-future publication.
    struct IdleSignalingBackend {
        /// Real in-process backend used for publication and acquisition behavior.
        local: LocalFederateCoordinationBackend,
        /// Test-harness signal emitted only after the coordinator publishes no future event.
        idle_tx: mpsc::Sender<()>,
    }

    impl FederateCoordinationBackend for IdleSignalingBackend {
        /// Signals no-future publication before forwarding it to the real local backend.
        fn publish(
            &mut self,
            publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            if publication.next_event().is_none() {
                self.idle_tx.send(()).unwrap();
            }
            self.local.publish(publication)
        }

        /// Polls the real local backend for the publication's matching acquisition.
        fn poll_acquisition(
            &mut self,
            timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            self.local.poll_acquisition(timeout)
        }

        /// Forwards aggregate logical completion to the real local backend.
        fn complete(
            &mut self,
            completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            self.local.complete(completion)
        }

        /// Stops the real local backend and releases any pending publication.
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            self.local.stop()
        }
    }

    /// Backend operation observed by terminal-path integration tests.
    #[derive(Debug, Eq, PartialEq)]
    enum BackendCall {
        /// Aggregate publication sent before terminal coordination.
        Publish {
            /// Published logical candidate, or no future event.
            next_event: Option<Tag>,
        },
        /// Terminal backend stop.
        Stop,
    }

    /// Backend that records publication and stop ordering through a channel.
    struct CallRecordingBackend {
        /// Receiver-owned operation stream sender.
        call_tx: mpsc::Sender<BackendCall>,
    }

    impl FederateCoordinationBackend for CallRecordingBackend {
        /// Records an aggregate publication.
        fn publish(
            &mut self,
            publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            self.call_tx
                .send(BackendCall::Publish {
                    next_event: publication.next_event(),
                })
                .unwrap();
            Ok(())
        }

        /// Supplies no acquisition because terminal tests publish no finite candidate.
        fn poll_acquisition(
            &mut self,
            _timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            Ok(None)
        }

        /// Accepts completion because terminal tests do not inspect logical frontiers.
        fn complete(
            &mut self,
            _completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            Ok(())
        }

        /// Records terminal backend stop.
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            self.call_tx.send(BackendCall::Stop).unwrap();
            Ok(())
        }
    }

    /// Backend whose abort path proves the original operation failure remains authoritative.
    struct PublishAndStopFailingBackend;

    impl FederateCoordinationBackend for PublishAndStopFailingBackend {
        /// Rejects the publication with the first operation-specific failure.
        fn publish(
            &mut self,
            _publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            Err(FederateCoordinationError::BackendPublish {
                message: "publication rejected".to_owned(),
            })
        }

        /// Returns no acquisition because publication always fails first.
        fn poll_acquisition(
            &mut self,
            _timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            Ok(None)
        }

        /// Accepts completion because this test reaches only publication.
        fn complete(
            &mut self,
            _completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            Ok(())
        }

        /// Fails during cleanup so the test can prove it does not replace the first failure.
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            Err(FederateCoordinationError::BackendStop {
                message: "cleanup stop rejected".to_owned(),
            })
        }
    }

    /// Builds one participant directly for deterministic channel-disconnection tests.
    fn participant(
        enclave: EnclaveIndex,
        report_tx: mpsc::Sender<CoordinatorReport>,
        command_rx: mpsc::Receiver<ParticipantCommand>,
    ) -> FederateCoordinationParticipant {
        let (_event_tx, event_rx) = kanal::unbounded();
        FederateCoordinationParticipant {
            enclave,
            report_tx,
            command_rx,
            event_rx,
            parked_revision: None,
            logical_horizon: None,
            deferred_error: None,
            terminal: false,
        }
    }

    /// Verifies every private pure-state failure maps to a closed public category.
    #[test]
    fn pure_state_failures_map_to_closed_public_categories() {
        let enclave = EnclaveIndex::new(7);
        for (private, public) in [
            (
                CoordinationStateError::NoParticipants,
                FederateCoordinationError::NoParticipants,
            ),
            (
                CoordinationStateError::DuplicateEnclave { enclave },
                FederateCoordinationError::DuplicateEnclave { enclave },
            ),
            (
                CoordinationStateError::UnknownEnclave { enclave },
                FederateCoordinationError::UnknownEnclave { enclave },
            ),
            (
                CoordinationStateError::InvalidObservationTransition {
                    phase: CoordinationPhase::Probing,
                    observation: Observation::Parked,
                },
                FederateCoordinationError::InvalidObservationTransition,
            ),
            (
                CoordinationStateError::ParticipantStoppedBeforeTerminal { enclave },
                FederateCoordinationError::ParticipantStoppedBeforeTerminal { enclave },
            ),
        ] {
            assert_eq!(FederateCoordinationError::from(private), public);
        }
    }

    /// Verifies participant-to-coordinator channel loss retains the reporting identity.
    #[test]
    fn coordinator_report_disconnection_retains_participant() {
        let enclave = EnclaveIndex::new(3);
        let (report_tx, report_rx) = mpsc::channel();
        let (_command_tx, command_rx) = mpsc::channel();
        drop(report_rx);
        let participant = participant(enclave, report_tx, command_rx);

        assert_eq!(
            participant
                .report(CoordinatorReport::Scheduler(SchedulerMessage::Active {
                    enclave,
                }))
                .unwrap_err(),
            FederateCoordinationError::CoordinatorReportChannelDisconnected {
                enclave: Some(enclave),
            }
        );
    }

    /// Verifies coordinator-to-participant channel loss retains the addressed identity.
    #[test]
    fn participant_command_disconnection_retains_participant() {
        let enclave = EnclaveIndex::new(7);
        let (report_tx, _report_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        drop(command_tx);
        let mut participant = participant(enclave, report_tx, command_rx);

        assert_eq!(
            participant.wait().unwrap_err(),
            FederateCoordinationError::ParticipantCommandChannelDisconnected { enclave }
        );
    }

    /// Verifies abort cleanup cannot replace the first backend operation failure.
    #[test]
    fn first_backend_failure_survives_abort_cleanup() {
        let enclave = EnclaveIndex::new(3);
        let (_event_tx, event_rx) = kanal::unbounded();
        let FederateCoordination {
            coordinator,
            mut participants,
            ..
        } = FederateCoordination::new(
            [(enclave, event_rx)],
            LifecyclePolicy::KeepAlive,
            PublishAndStopFailingBackend,
        )
        .unwrap();
        let mut participant = participants.remove(&enclave).unwrap();

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            assert!(participant.wait().unwrap().is_none());
            assert_eq!(
                coordinator.join().unwrap().unwrap_err(),
                FederateCoordinationError::BackendPublish {
                    message: "publication rejected".to_owned(),
                }
            );
        });
    }

    /// Verifies successful scheduler finalization remains present for fixed-point commands.
    #[test]
    fn successful_participant_waits_for_fixed_point_before_closing() {
        let enclave = EnclaveIndex::new(3);
        let (_event_tx, event_rx) = kanal::unbounded();
        let (call_tx, call_rx) = mpsc::channel();
        let FederateCoordination {
            coordinator,
            mut participants,
            ..
        } = FederateCoordination::new(
            [(enclave, event_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            CallRecordingBackend { call_tx },
        )
        .unwrap();
        let mut participant = participants.remove(&enclave).unwrap();

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            let participant = scope.spawn(move || participant.finish_success());
            participant.join().unwrap().unwrap();
            coordinator.join().unwrap().unwrap();
        });

        assert_eq!(
            call_rx.into_iter().collect::<Vec<_>>(),
            [BackendCall::Publish { next_event: None }, BackendCall::Stop,]
        );
    }

    /// Verifies failed participant cleanup never publishes successful terminal idleness.
    #[test]
    fn failed_participant_drop_does_not_publish_idle() {
        let enclave = EnclaveIndex::new(3);
        let (_event_tx, event_rx) = kanal::unbounded();
        let (call_tx, call_rx) = mpsc::channel();
        let FederateCoordination {
            abort_handle,
            coordinator,
            mut participants,
        } = FederateCoordination::new(
            [(enclave, event_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            CallRecordingBackend { call_tx },
        )
        .unwrap();
        let participant = participants.remove(&enclave).unwrap();

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            drop(participant);
            abort_handle.abort();
            coordinator.join().unwrap().unwrap();
        });

        assert_eq!(call_rx.into_iter().collect::<Vec<_>>(), [BackendCall::Stop]);
    }

    /// Verifies queued work in the commit window resumes every parked participant.
    #[test]
    fn parked_barrier_resumes_all_schedulers_when_work_arrives_before_commit() {
        let eventful = EnclaveIndex::new(3);
        let peer = EnclaveIndex::new(7);
        let (eventful_tx, eventful_rx) = kanal::unbounded();
        let (_peer_tx, peer_rx) = kanal::unbounded();
        let FederateCoordination {
            abort_handle,
            mut coordinator,
            mut participants,
        } = FederateCoordination::new(
            [(eventful, eventful_rx), (peer, peer_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            LocalFederateCoordinationBackend::default(),
        )
        .unwrap();
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
            coordinator_handle.join().unwrap().unwrap();

            assert_eq!(
                observations.len(),
                3,
                "both parked schedulers must reach a later common stop"
            );
            assert!(observations.iter().any(|(enclave, event)| {
                *enclave == eventful
                    && matches!(
                        event,
                        Ok(Some(AsyncEvent::Shutdown { delay })) if *delay == Duration::ZERO
                    )
            }));
            assert!(observations
                .iter()
                .any(|(enclave, event)| *enclave == eventful && matches!(event, Ok(None))));
            assert!(observations
                .iter()
                .any(|(enclave, event)| *enclave == peer && matches!(event, Ok(None))));
        });
    }

    /// Verifies a unanimously idle kept-alive Federate remains wakeable and does not stop.
    #[test]
    fn kept_alive_all_idle_participants_wake_for_later_input() {
        let eventful = EnclaveIndex::new(3);
        let peer = EnclaveIndex::new(7);
        let (eventful_tx, eventful_rx) = kanal::unbounded();
        let (_peer_tx, peer_rx) = kanal::unbounded();
        let (idle_tx, idle_rx) = mpsc::channel();
        let FederateCoordination {
            abort_handle,
            coordinator,
            mut participants,
        } = FederateCoordination::new(
            [(eventful, eventful_rx), (peer, peer_rx)],
            LifecyclePolicy::KeepAlive,
            IdleSignalingBackend {
                local: LocalFederateCoordinationBackend::default(),
                idle_tx,
            },
        )
        .unwrap();
        let mut eventful_participant = participants.remove(&eventful).unwrap();
        let mut peer_participant = participants.remove(&peer).unwrap();
        let (result_tx, result_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            let eventful_result = result_tx.clone();
            let eventful_thread = scope.spawn(move || {
                eventful_result
                    .send((eventful, eventful_participant.wait()))
                    .unwrap();
            });
            let peer_thread = scope.spawn(move || peer_participant.wait());

            idle_rx.recv_timeout(StdDuration::from_secs(1)).unwrap();
            eventful_tx
                .send(AsyncEvent::Shutdown {
                    delay: Duration::ZERO,
                })
                .unwrap();
            assert!(matches!(
                result_rx.recv_timeout(StdDuration::from_secs(1)).unwrap(),
                (enclave, Ok(Some(AsyncEvent::Shutdown { delay })))
                    if enclave == eventful && delay == Duration::ZERO
            ));
            abort_handle.abort();
            eventful_thread.join().unwrap();
            assert!(peer_thread.join().unwrap().unwrap().is_none());
            coordinator.join().unwrap().unwrap();
        });
    }
}
