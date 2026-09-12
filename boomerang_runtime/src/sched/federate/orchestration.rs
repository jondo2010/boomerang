//! Blocking channel adapter for one compiled Federate's coordination state.
//!
//! Per-Enclave ports stamp stored compiled identities onto scheduler messages and inspect only
//! their own event queues. The coordinator serializes those messages through the pure state,
//! executes its semantic actions against a selected backend, and owns no scheduling decisions.

use std::{sync::mpsc, time::Duration as StdDuration};

use tinymap::TinySecondaryMap;

use super::{
    backend::{CoordinationRevision, FederateCoordinationBackend, FederateCoordinationError},
    state::{
        CoordinationAction, CoordinationStateError, FederateCoordinationState, LifecyclePolicy,
        Observation, SchedulerMessage,
    },
};
use crate::{image::EnclaveIndex, AsyncEvent, Tag};

/// Returns whether one asynchronous event introduces executable or terminal scheduler work.
fn revises_candidate(event: &AsyncEvent) -> bool {
    matches!(
        event,
        AsyncEvent::Logical { .. } | AsyncEvent::Physical { .. } | AsyncEvent::Shutdown { .. }
    )
}

/// Terminal coordinator reason consumed by a scheduler wake path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FederateTermination {
    /// Successful Federate termination permits local shutdown lifecycle work.
    Graceful {
        /// Exact logical horizon for shutdown, or the scheduler's next tag for ordinary stop.
        tag: Option<Tag>,
    },
    /// Federate failure requires immediate exit without shutdown lifecycle work.
    Abort,
}

/// Result of blocking while a compiled scheduler has no local candidate.
#[derive(Debug)]
pub(crate) enum FederateIdleWait {
    /// New scheduler work interrupted the idle wait.
    Interrupted(AsyncEvent),
    /// The scheduler may process the shared terminal logical horizon.
    LogicalHorizon(Tag),
    /// Federate coordination terminated without granting scheduler work.
    Stopped,
    /// Federate coordination aborted after a participant or backend failure.
    Aborted,
}

/// Result of requesting permission to process one compiled scheduler tag.
#[derive(Debug)]
pub(crate) enum FederateTagAcquisition {
    /// The requested logical tag may proceed to wall-clock synchronization.
    Granted,
    /// New scheduler work invalidated the candidate before it was granted.
    Interrupted(AsyncEvent),
    /// The scheduler may process the shared terminal logical horizon.
    LogicalHorizon(Tag),
    /// Federate coordination terminated without granting the requested tag.
    Stopped,
    /// Federate coordination aborted after a participant or backend failure.
    Aborted,
}

/// Result of waiting for a Federate horizon that authorizes local control advancement.
#[derive(Debug)]
pub(crate) enum FederateControlAuthorization {
    /// The retained Federate grant covers the requested control tag.
    Authorized,
    /// An asynchronous scheduler event must be handled before retrying authorization.
    Interrupted(AsyncEvent),
    /// Federate coordination ended at the exact shared logical horizon.
    LogicalHorizon(Tag),
    /// Federate stop or failure forbids further logical advancement.
    Stopped,
    /// Federate failure requires immediate scheduler exit.
    Aborted,
}

/// Scheduler-facing coordination port without a backend or transport type.
pub(crate) trait FederateSchedulerCoordination {
    /// Reports work observed before the scheduler can publish its revised candidate.
    fn active(&mut self);

    /// Publishes no local candidate and waits for asynchronous work or terminal coordination.
    fn wait(&mut self) -> Result<FederateIdleWait, FederateCoordinationError>;

    /// Publishes a local candidate and waits for its grant or an asynchronous interruption.
    fn acquire_tag(
        &mut self,
        tag: Tag,
    ) -> Result<FederateTagAcquisition, FederateCoordinationError>;

    /// Waits until the retained Federate grant covers a control-only tag.
    fn authorize_control(
        &mut self,
        tag: Tag,
    ) -> Result<FederateControlAuthorization, FederateCoordinationError>;

    /// Consumes a queued terminal command after the coordinator closes the scheduler event queue.
    fn terminal_after_event_channel_closed(
        &mut self,
    ) -> Result<Option<FederateTermination>, FederateCoordinationError> {
        Ok(None)
    }

    /// Reports completion of one processed logical tag.
    #[allow(dead_code)]
    fn logical_tag_complete(&mut self, tag: Tag) -> Result<(), FederateCoordinationError>;

    /// Reports that the legacy shared logical horizon is being processed.
    fn logical_horizon_reached(&mut self, tag: Tag);

    /// Reports that this scheduler processed a terminal event.
    fn participant_stopped(&mut self);

    /// Reports scheduler failure and requests Federate-wide abortion.
    fn fail(&mut self);
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
        }
    }
}

/// Supervisor-owned handle retained outside worker threads to request Federate-wide abortion.
pub(crate) struct FederateAbortHandle {
    /// Shared coordinator-report sender used for the terminal failure message.
    report_tx: mpsc::Sender<CoordinatorReport>,
}

impl FederateAbortHandle {
    /// Requests idempotent Federate-wide abortion without blocking the supervising thread.
    pub(crate) fn abort(&self) {
        let _ = self
            .report_tx
            .send(CoordinatorReport::Scheduler(SchedulerMessage::Failed {
                enclave: None,
            }));
    }
}

/// Coordinator that exclusively owns the pure state and backend on its dedicated worker thread.
///
/// Construction returns this value to the supervisor, which moves it into exactly one coordinator
/// thread before any scheduler is started.
pub(crate) struct FederateCoordinator<B: FederateCoordinationBackend> {
    /// Serialized reports from every compiled participant.
    report_rx: mpsc::Receiver<CoordinatorReport>,
    /// Per-participant command senders keyed by compiled identity.
    commands: TinySecondaryMap<EnclaveIndex, mpsc::Sender<ParticipantCommand>>,
    /// Per-participant scheduler wake senders closed only after a terminal command is queued.
    events: TinySecondaryMap<EnclaveIndex, crate::Sender<AsyncEvent>>,
    /// Authoritative candidate, phase, completion, and terminal state.
    state: FederateCoordinationState,
    /// Selected transport-neutral coordination backend.
    backend: B,
    /// Records a stop attempt before calling user code, including failures and panics.
    backend_stop_attempted: bool,
    #[cfg(test)]
    /// One-shot queue-race hook immediately before the final parked recheck.
    commit_window_hook: Option<Box<dyn FnOnce() + Send>>,
}

impl<B: FederateCoordinationBackend> FederateCoordinator<B> {
    /// Runs until pure coordination stops or returns its first backend, state, or channel failure.
    pub(crate) fn run(mut self) -> Result<(), FederateCoordinationError> {
        loop {
            let outcome = match self.report_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(report) => self.handle_report(report),
                Err(mpsc::RecvTimeoutError::Timeout) => self.progress_backend(),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(
                    FederateCoordinationError::CoordinatorReportChannelDisconnected {
                        enclave: None,
                    },
                ),
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
                let actions = self.state.handle_scheduler(message)?;
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
            CoordinatorReport::LogicalHorizon { enclave, tag } => {
                self.require_participant(enclave)?;
                let _ = self.state.stop();
                self.stop_backend_once()?;
                self.send_available(ParticipantCommand::LogicalHorizon(tag));
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
                }
                action @ CoordinationAction::AdvanceHorizon { .. } => {
                    self.send_all(ParticipantCommand::Action(action))?;
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
                    self.stop_backend_once()?;
                    self.terminate_available(ParticipantCommand::Action(action));
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Progresses one backend input and applies any acquired grant through the pure state.
    fn progress_backend(&mut self) -> Result<bool, FederateCoordinationError> {
        if let Some(acquisition) = self.backend.progress(StdDuration::from_millis(1))? {
            let actions = self.state.handle_acquisition(acquisition)?;
            if self.execute_actions(actions)? {
                return Ok(true);
            }
        }
        if let Some(revision) = self.state.idle_confirmation_revision() {
            if self.backend.confirm_idle(revision)? {
                let actions = self.state.handle_idle_confirmation(revision);
                return self.execute_actions(actions);
            }
        }
        Ok(false)
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

    /// Queues one terminal command before closing each participant's scheduler wake channel.
    fn terminate_available(&self, command: ParticipantCommand) {
        for (enclave, sender) in self.commands.iter() {
            if sender.send(command).is_ok() {
                let event = self
                    .events
                    .get(enclave)
                    .expect("every participant command sender has a scheduler event sender");
                let _ = event.close();
            }
        }
    }

    /// Wakes participants and stops the backend without replacing the first returned error.
    fn abort_after_error(&mut self) {
        let _ = self
            .state
            .handle_scheduler(SchedulerMessage::Failed { enclave: None });
        self.terminate_available(ParticipantCommand::Action(CoordinationAction::Abort));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.stop_backend_once()));
    }

    /// Releases backend ownership once while preserving an explicit stop failure.
    fn stop_backend_once(&mut self) -> Result<(), FederateCoordinationError> {
        if std::mem::replace(&mut self.backend_stop_attempted, true) {
            return Ok(());
        }
        self.backend.stop()
    }
}

/// Wakes every still-live scheduler if coordinator ownership ends, including during unwinding.
///
/// A terminal command queued by the normal run path remains first in each FIFO; this fallback Abort
/// is observed only when coordinator ownership ends before a graceful command is available.
/// Backend cleanup is attempted once; secondary failures cannot replace an existing unwind.
impl<B: FederateCoordinationBackend> Drop for FederateCoordinator<B> {
    fn drop(&mut self) {
        for (enclave, sender) in self.commands.iter() {
            if sender
                .send(ParticipantCommand::Action(CoordinationAction::Abort))
                .is_ok()
            {
                if let Some(event) = self.events.get(enclave) {
                    let _ = event.close();
                }
            }
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.stop_backend_once()));
    }
}

/// Per-scheduler channel adapter moved into exactly one scheduler thread.
///
/// The stored compiled Enclave identity keeps its channel traffic associated with that scheduler.
pub(crate) struct EnclaveCoordinationPort {
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
    /// Greatest broadcast Federate grant retained for control-only authorization.
    grant_horizon: Option<Tag>,
    /// First report-channel error produced by an infallible compatibility method.
    deferred_error: Option<FederateCoordinationError>,
    /// Whether successful terminal processing awaits coordinator acknowledgement.
    stop_reported: bool,
    /// Terminal coordinator reason observed by this participant, if any.
    termination: Option<FederateTermination>,
}

impl EnclaveCoordinationPort {
    /// Retains one broadcast Federate grant without allowing the horizon to regress.
    fn advance_horizon(&mut self, tag: Tag) {
        self.grant_horizon = Some(self.grant_horizon.map_or(tag, |current| current.max(tag)));
    }

    /// Applies one command while control-only work waits for authorization.
    fn apply_control_command(&mut self, command: ParticipantCommand) -> bool {
        match command {
            ParticipantCommand::Action(CoordinationAction::AdvanceHorizon { tag }) => {
                self.advance_horizon(tag);
                false
            }
            ParticipantCommand::LogicalHorizon(tag) => {
                self.logical_horizon = Some(tag);
                self.termination = Some(FederateTermination::Graceful { tag: Some(tag) });
                true
            }
            ParticipantCommand::Action(CoordinationAction::Stop) => {
                self.termination = Some(FederateTermination::Graceful { tag: None });
                true
            }
            ParticipantCommand::Action(CoordinationAction::Abort) => {
                self.termination = Some(FederateTermination::Abort);
                true
            }
            ParticipantCommand::Action(_) => false,
        }
    }

    /// Maps the retained terminal reason into the control-authorization outcome.
    fn terminal_authorization(&self) -> FederateControlAuthorization {
        match self.termination {
            Some(FederateTermination::Graceful { tag: Some(tag) }) => {
                FederateControlAuthorization::LogicalHorizon(tag)
            }
            Some(FederateTermination::Graceful { tag: None }) => {
                FederateControlAuthorization::Stopped
            }
            Some(FederateTermination::Abort) => FederateControlAuthorization::Aborted,
            None => unreachable!("terminal authorization requires a terminal reason"),
        }
    }

    /// Publishes successful terminal idleness and remains available for fixed-point commands.
    pub(crate) fn finish_success(&mut self) -> Result<(), FederateCoordinationError> {
        self.take_deferred_error()?;
        while self.termination.is_none() {
            match FederateSchedulerCoordination::wait(self)? {
                FederateIdleWait::Interrupted(_) => {}
                FederateIdleWait::LogicalHorizon(_) => {}
                FederateIdleWait::Stopped | FederateIdleWait::Aborted => break,
            }
        }
        Ok(())
    }

    /// Sends one report, accepting disconnect only after the terminal wake channel closes.
    fn report(&self, report: CoordinatorReport) -> Result<(), FederateCoordinationError> {
        if self.report_tx.send(report).is_ok() || self.event_rx.is_closed() {
            return Ok(());
        }
        Err(
            FederateCoordinationError::CoordinatorReportChannelDisconnected {
                enclave: Some(self.enclave),
            },
        )
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
        if event.as_ref().is_some_and(revises_candidate) {
            self.report(CoordinatorReport::Scheduler(SchedulerMessage::Active {
                enclave: self.enclave,
            }))?;
        }
        Ok(event)
    }
}

impl FederateSchedulerCoordination for EnclaveCoordinationPort {
    /// Reports work without inventing a candidate tag.
    fn active(&mut self) {
        self.report_infallible(CoordinatorReport::Scheduler(SchedulerMessage::Active {
            enclave: self.enclave,
        }));
    }

    /// Publishes idle and blocks while translating fixed-point commands around the event queue.
    fn wait(&mut self) -> Result<FederateIdleWait, FederateCoordinationError> {
        self.take_deferred_error()?;
        self.parked_revision = None;
        if !self.stop_reported {
            self.report(CoordinatorReport::Scheduler(SchedulerMessage::Publish {
                enclave: self.enclave,
                next_event: None,
            }))?;
        }

        loop {
            if let Some(event) = self.take_active_event()? {
                return Ok(FederateIdleWait::Interrupted(event));
            }
            match self.command_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(ParticipantCommand::Action(CoordinationAction::Probe { revision })) => {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(FederateIdleWait::Interrupted(event));
                    }
                    self.observe(revision, Observation::Probed)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Park { revision })) => {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(FederateIdleWait::Interrupted(event));
                    }
                    self.parked_revision = Some(revision);
                    self.observe(revision, Observation::Parked)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Recheck { revision }))
                    if self.parked_revision == Some(revision) =>
                {
                    if let Some(event) = self.take_active_event()? {
                        return Ok(FederateIdleWait::Interrupted(event));
                    }
                    self.observe(revision, Observation::Rechecked)?;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Resume { .. })) => {
                    self.parked_revision = None;
                }
                Ok(ParticipantCommand::Action(CoordinationAction::AdvanceHorizon { tag })) => {
                    self.advance_horizon(tag);
                }
                Ok(ParticipantCommand::LogicalHorizon(tag)) => {
                    self.logical_horizon = Some(tag);
                    self.termination = Some(FederateTermination::Graceful { tag: Some(tag) });
                    return Ok(FederateIdleWait::LogicalHorizon(tag));
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Stop)) => {
                    self.termination = Some(FederateTermination::Graceful { tag: None });
                    return Ok(FederateIdleWait::Stopped);
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Abort)) => {
                    self.termination = Some(FederateTermination::Abort);
                    return Ok(FederateIdleWait::Aborted);
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
    fn acquire_tag(
        &mut self,
        tag: Tag,
    ) -> Result<FederateTagAcquisition, FederateCoordinationError> {
        self.take_deferred_error()?;
        self.report(CoordinatorReport::Scheduler(SchedulerMessage::Publish {
            enclave: self.enclave,
            next_event: Some(tag),
        }))?;

        loop {
            if let Some(event) = self.take_active_event()? {
                return Ok(FederateTagAcquisition::Interrupted(event));
            }
            match self.command_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(ParticipantCommand::Action(CoordinationAction::Grant {
                    tag: granted, ..
                })) if granted >= tag => {
                    return Ok(FederateTagAcquisition::Granted);
                }
                Ok(ParticipantCommand::Action(CoordinationAction::AdvanceHorizon { tag })) => {
                    self.advance_horizon(tag);
                }
                Ok(ParticipantCommand::LogicalHorizon(tag)) => {
                    self.logical_horizon = Some(tag);
                    self.termination = Some(FederateTermination::Graceful { tag: Some(tag) });
                    return Ok(FederateTagAcquisition::LogicalHorizon(tag));
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Stop)) => {
                    self.termination = Some(FederateTermination::Graceful { tag: None });
                    return Ok(FederateTagAcquisition::Stopped);
                }
                Ok(ParticipantCommand::Action(CoordinationAction::Abort)) => {
                    self.termination = Some(FederateTermination::Abort);
                    return Ok(FederateTagAcquisition::Aborted);
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

    /// Waits for retained-horizon authorization without publishing executable work.
    fn authorize_control(
        &mut self,
        tag: Tag,
    ) -> Result<FederateControlAuthorization, FederateCoordinationError> {
        self.take_deferred_error()?;
        if self.termination.is_some() {
            return Ok(self.terminal_authorization());
        }

        loop {
            loop {
                match self.command_rx.try_recv() {
                    Ok(command) if self.apply_control_command(command) => {
                        return Ok(self.terminal_authorization());
                    }
                    Ok(_) => {}
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.termination = Some(FederateTermination::Graceful { tag: None });
                        return Ok(FederateControlAuthorization::Stopped);
                    }
                }
            }
            if let Some(event) = self.take_active_event()? {
                return Ok(FederateControlAuthorization::Interrupted(event));
            }
            if self.grant_horizon.is_some_and(|horizon| horizon >= tag) {
                return Ok(FederateControlAuthorization::Authorized);
            }
            match self.command_rx.recv_timeout(StdDuration::from_millis(1)) {
                Ok(command) if self.apply_control_command(command) => {
                    return Ok(self.terminal_authorization());
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.termination = Some(FederateTermination::Graceful { tag: None });
                    return Ok(FederateControlAuthorization::Stopped);
                }
            }
        }
    }

    /// Drains already-queued commands until the terminal command that precedes event closure.
    fn terminal_after_event_channel_closed(
        &mut self,
    ) -> Result<Option<FederateTermination>, FederateCoordinationError> {
        self.take_deferred_error()?;
        loop {
            match self.command_rx.try_recv() {
                Ok(command) if self.apply_control_command(command) => return Ok(self.termination),
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return Ok(None),
                Err(mpsc::TryRecvError::Disconnected) => {
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

    /// Reports a legacy shared logical horizon without changing key domains.
    fn logical_horizon_reached(&mut self, tag: Tag) {
        if self.logical_horizon == Some(tag) {
            return;
        }
        self.logical_horizon = Some(tag);
        self.termination = Some(FederateTermination::Graceful { tag: Some(tag) });
        self.report_infallible(CoordinatorReport::LogicalHorizon {
            enclave: self.enclave,
            tag,
        });
    }

    /// Reports successful terminal processing through the pure coordination state.
    fn participant_stopped(&mut self) {
        if self.termination.is_some() || self.stop_reported {
            return;
        }
        self.stop_reported = true;
        self.report_infallible(CoordinatorReport::Scheduler(
            SchedulerMessage::ParticipantStopped {
                enclave: self.enclave,
            },
        ));
    }

    /// Reports a typed participant-origin failure.
    fn fail(&mut self) {
        self.report_infallible(CoordinatorReport::Scheduler(SchedulerMessage::Failed {
            enclave: Some(self.enclave),
        }));
    }
}

/// Transient constructor result immediately destructured before worker threads are spawned.
///
/// Its coordinator moves to the dedicated coordinator thread, each participant moves to exactly one
/// scheduler thread, and the abort handle remains owned by the supervising thread.
pub(crate) struct FederateCoordinationParts<B: FederateCoordinationBackend> {
    /// Supervisor handle used to request Federate-wide abortion.
    pub(crate) abort_handle: FederateAbortHandle,
    /// Blocking coordinator that owns the pure state and selected backend.
    pub(crate) coordinator: FederateCoordinator<B>,
    /// Participants keyed directly by their compiled Enclave identities.
    pub(crate) participants: TinySecondaryMap<EnclaveIndex, EnclaveCoordinationPort>,
    #[cfg(test)]
    /// Test-visible lifecycle policy supplied to the authoritative pure state.
    lifecycle_policy: LifecyclePolicy,
}

impl<B: FederateCoordinationBackend> FederateCoordinationParts<B> {
    /// Iterates participant identities without rebasing deployment-global Enclave keys.
    #[cfg(test)]
    pub(crate) fn participant_indices(&self) -> impl Iterator<Item = EnclaveIndex> + '_ {
        self.participants.keys()
    }

    /// Returns the lifecycle policy supplied to the authoritative pure state.
    #[cfg(test)]
    pub(crate) const fn lifecycle_policy(&self) -> LifecyclePolicy {
        self.lifecycle_policy
    }

    /// Creates one participant per unique compiled identity and one shared coordinator.
    pub(crate) fn new(
        participants: impl IntoIterator<
            Item = (
                EnclaveIndex,
                crate::Sender<AsyncEvent>,
                crate::Receiver<AsyncEvent>,
            ),
        >,
        lifecycle: LifecyclePolicy,
        backend: B,
    ) -> Result<Self, FederateCoordinationError> {
        let participants = participants.into_iter().collect::<Vec<_>>();
        let state = FederateCoordinationState::new(
            participants.iter().map(|(enclave, _, _)| *enclave),
            lifecycle,
        )?;
        let (report_tx, report_rx) = mpsc::channel();
        let mut commands = TinySecondaryMap::new();
        let mut events = TinySecondaryMap::new();
        let mut ports = TinySecondaryMap::new();
        for (enclave, event_tx, event_rx) in participants {
            let (command_tx, command_rx) = mpsc::channel();
            commands.insert(enclave, command_tx);
            events.insert(enclave, event_tx);
            ports.insert(
                enclave,
                EnclaveCoordinationPort {
                    enclave,
                    report_tx: report_tx.clone(),
                    command_rx,
                    event_rx,
                    parked_revision: None,
                    logical_horizon: None,
                    grant_horizon: None,
                    deferred_error: None,
                    stop_reported: false,
                    termination: None,
                },
            );
        }
        Ok(Self {
            abort_handle: FederateAbortHandle {
                report_tx: report_tx.clone(),
            },
            coordinator: FederateCoordinator {
                report_rx,
                commands,
                events,
                state,
                backend,
                backend_stop_attempted: false,
                #[cfg(test)]
                commit_window_hook: None,
            },
            participants: ports,
            #[cfg(test)]
            lifecycle_policy: lifecycle,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Channel-level tests for wakeable idle execution and the parked queue race.

    use super::super::state::CoordinationPhase;
    use super::{
        CoordinationAction, CoordinationStateError, CoordinatorReport, EnclaveCoordinationPort,
        FederateControlAuthorization, FederateCoordinationParts, FederateIdleWait,
        FederateSchedulerCoordination, FederateTagAcquisition, FederateTermination, Observation,
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

    /// Local backend wrapper that signals a coordinator polling cycle.
    struct IdlePollingBackend {
        /// Real in-process backend used for publication and acquisition behavior.
        local: LocalFederateCoordinationBackend,
        /// Whether the coordinator has published no future event for every idle participant.
        idle_published: bool,
        /// Test-harness signal emitted by progress after the no-future publication.
        progress_tx: mpsc::Sender<()>,
    }

    impl FederateCoordinationBackend for IdlePollingBackend {
        /// Arms the test after no-future publication before forwarding to the local backend.
        fn publish(
            &mut self,
            publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            self.idle_published |= publication.next_event().is_none();
            self.local.publish(publication)
        }

        /// Signals only progress that follows the no-future publication.
        fn progress(
            &mut self,
            timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            if self.idle_published {
                self.progress_tx.send(()).unwrap();
            }
            self.local.progress(timeout)
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

    /// Controllable authority at the transport boundary; queue handling stays real.
    struct IdleAuthorityBackend {
        local: LocalFederateCoordinationBackend,
        accepted: bool,
        fail: bool,
        input: Option<crate::Sender<AsyncEvent>>,
        confirmations: usize,
        progresses: usize,
    }

    impl FederateCoordinationBackend for IdleAuthorityBackend {
        fn publish(
            &mut self,
            publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            self.local.publish(publication)
        }
        fn progress(
            &mut self,
            timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            self.progresses += 1;
            self.local.progress(timeout)
        }
        fn complete(
            &mut self,
            completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            self.local.complete(completion)
        }
        fn confirm_idle(
            &mut self,
            _revision: super::super::backend::CoordinationRevision,
        ) -> Result<bool, FederateCoordinationError> {
            self.confirmations += 1;
            if self.fail {
                return Err(FederateCoordinationError::BackendStop {
                    message: "idle rejected".into(),
                });
            }
            if self.accepted {
                if let Some(input) = self.input.take() {
                    input
                        .send(AsyncEvent::Shutdown {
                            delay: Duration::ZERO,
                        })
                        .unwrap();
                }
            }
            Ok(self.accepted)
        }
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            if self.fail {
                return Err(FederateCoordinationError::BackendStop {
                    message: "cleanup rejected".into(),
                });
            }
            self.local.stop()
        }
    }

    /// Catches premature local stop, missing backend polling, or skipping the queue recheck.
    #[test]
    fn coordinated_idle_waits_and_admits_input_during_confirmation() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let FederateCoordinationParts {
            mut coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx.clone(), event_rx)],
            LifecyclePolicy::CoordinatedTermination,
            IdleAuthorityBackend {
                local: LocalFederateCoordinationBackend::default(),
                accepted: false,
                fail: false,
                input: Some(event_tx),
                confirmations: 0,
                progresses: 0,
            },
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();
        coordinator
            .handle_report(CoordinatorReport::Scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: None,
            }))
            .unwrap();
        let revision = coordinator.state.revision();
        for observation in [
            Observation::Probed,
            Observation::Parked,
            Observation::Rechecked,
        ] {
            assert!(!coordinator
                .handle_report(CoordinatorReport::Observation {
                    enclave,
                    revision,
                    observation
                })
                .unwrap());
        }
        assert!(!coordinator.progress_backend().unwrap());
        assert_eq!(coordinator.backend.confirmations, 1);
        assert_eq!(coordinator.backend.progresses, 1);
        assert!(!coordinator.state.is_stopped());
        coordinator.backend.accepted = true;
        assert!(!coordinator.progress_backend().unwrap());
        assert_eq!(coordinator.backend.confirmations, 2);
        assert_eq!(coordinator.state.phase(), CoordinationPhase::Probing);
        assert!(matches!(
            participant.wait().unwrap(),
            FederateIdleWait::Interrupted(AsyncEvent::Shutdown { .. })
        ));
        while let Ok(report) = coordinator.report_rx.try_recv() {
            assert!(!coordinator.handle_report(report).unwrap());
        }
        assert_eq!(coordinator.state.phase(), CoordinationPhase::Active);
        assert!(!coordinator.state.is_stopped());
    }

    /// Catches discarded authority errors and cleanup overwriting the first failure.
    #[test]
    fn coordinated_idle_confirmation_failure_aborts_and_retains_first_error() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let FederateCoordinationParts {
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::CoordinatedTermination,
            IdleAuthorityBackend {
                local: LocalFederateCoordinationBackend::default(),
                accepted: false,
                fail: true,
                input: None,
                confirmations: 0,
                progresses: 0,
            },
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();
        std::thread::scope(|scope| {
            let handle = scope.spawn(move || coordinator.run());
            assert!(matches!(
                participant.wait().unwrap(),
                FederateIdleWait::Aborted
            ));
            assert_eq!(
                handle.join().unwrap().unwrap_err(),
                FederateCoordinationError::BackendStop {
                    message: "idle rejected".into()
                }
            );
        });
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
        fn progress(
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
    struct PublishAndStopFailingBackend {
        /// Selects a panic instead of a returned cleanup error.
        panic_stop: bool,
        /// Counts stop attempts across normal error handling and destruction.
        stops: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

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
        fn progress(
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
            self.stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(!self.panic_stop, "injected cleanup panic");
            Err(FederateCoordinationError::BackendStop {
                message: "cleanup stop rejected".to_owned(),
            })
        }
    }

    /// Backend that accepts coordination until terminal cleanup fails.
    struct StopFailingBackend;

    impl FederateCoordinationBackend for StopFailingBackend {
        /// Accepts the aggregate publication that drives terminal coordination.
        fn publish(
            &mut self,
            _publication: FederatePublication,
        ) -> Result<(), FederateCoordinationError> {
            Ok(())
        }

        /// Supplies no acquisition because terminal tests publish no finite candidate.
        fn progress(
            &mut self,
            _timeout: StdDuration,
        ) -> Result<Option<FederateAcquisition>, FederateCoordinationError> {
            Ok(None)
        }

        /// Accepts completion because terminal tests do not process logical work.
        fn complete(
            &mut self,
            _completion: FederateCompletion,
        ) -> Result<(), FederateCoordinationError> {
            Ok(())
        }

        /// Rejects terminal cleanup so participants must observe abortion, not graceful stop.
        fn stop(&mut self) -> Result<(), FederateCoordinationError> {
            Err(FederateCoordinationError::BackendStop {
                message: "terminal cleanup rejected".to_owned(),
            })
        }
    }

    /// Builds one participant directly for deterministic channel-disconnection tests.
    fn participant(
        enclave: EnclaveIndex,
        report_tx: mpsc::Sender<CoordinatorReport>,
        command_rx: mpsc::Receiver<ParticipantCommand>,
    ) -> EnclaveCoordinationPort {
        participant_with_events(enclave, report_tx, command_rx).0
    }

    /// Builds one participant and retains its scheduler-event sender for priority tests.
    fn participant_with_events(
        enclave: EnclaveIndex,
        report_tx: mpsc::Sender<CoordinatorReport>,
        command_rx: mpsc::Receiver<ParticipantCommand>,
    ) -> (EnclaveCoordinationPort, crate::Sender<AsyncEvent>) {
        let (event_tx, event_rx) = kanal::unbounded();
        let participant = EnclaveCoordinationPort {
            enclave,
            report_tx,
            command_rx,
            event_rx,
            parked_revision: None,
            logical_horizon: None,
            grant_horizon: None,
            deferred_error: None,
            stop_reported: false,
            termination: None,
        };
        (participant, event_tx)
    }

    /// Verifies terminal coordination cannot be mistaken for an acquired logical tag.
    #[test]
    fn terminal_acquisition_is_explicit() {
        let enclave = EnclaveIndex::new(3);
        let (report_tx, _report_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let mut participant = participant(enclave, report_tx, command_rx);
        command_tx
            .send(ParticipantCommand::Action(CoordinationAction::Stop))
            .unwrap();

        assert!(matches!(
            participant.acquire_tag(Tag::ZERO).unwrap(),
            FederateTagAcquisition::Stopped
        ));
    }

    /// Verifies control work waits for a covering broadcast horizon without publishing a candidate.
    #[test]
    fn control_authorization_waits_for_broadcast_horizon() {
        let enclave = EnclaveIndex::new(3);
        let (report_tx, report_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let mut participant = participant(enclave, report_tx, command_rx);
        let requested = Tag::new(Duration::seconds(2), 0);
        let weaker = Tag::new(Duration::seconds(1), 0);

        std::thread::scope(|scope| {
            let (result_tx, result_rx) = mpsc::channel();
            let authorization = scope.spawn(move || {
                result_tx
                    .send(participant.authorize_control(requested))
                    .unwrap();
            });
            command_tx
                .send(ParticipantCommand::Action(
                    CoordinationAction::AdvanceHorizon { tag: weaker },
                ))
                .unwrap();
            assert!(result_rx
                .recv_timeout(StdDuration::from_millis(10))
                .is_err());
            command_tx
                .send(ParticipantCommand::Action(
                    CoordinationAction::AdvanceHorizon { tag: requested },
                ))
                .unwrap();
            assert!(matches!(
                result_rx.recv_timeout(StdDuration::from_secs(1)).unwrap(),
                Ok(FederateControlAuthorization::Authorized)
            ));
            authorization.join().unwrap();
        });
        assert!(report_rx.try_recv().is_err());
    }

    /// Verifies queued terminal coordination wins over a retained covering horizon.
    #[test]
    fn cached_control_authorization_observes_queued_terminal_command() {
        let enclave = EnclaveIndex::new(3);
        let requested = Tag::new(Duration::seconds(2), 0);
        for terminal in [
            ParticipantCommand::Action(CoordinationAction::Stop),
            ParticipantCommand::Action(CoordinationAction::Abort),
            ParticipantCommand::LogicalHorizon(requested),
        ] {
            let expects_abort = matches!(
                terminal,
                ParticipantCommand::Action(CoordinationAction::Abort)
            );
            let expects_horizon =
                matches!(terminal, ParticipantCommand::LogicalHorizon(tag) if tag == requested);
            let (report_tx, _report_rx) = mpsc::channel();
            let (command_tx, command_rx) = mpsc::channel();
            let mut participant = participant(enclave, report_tx, command_rx);
            command_tx
                .send(ParticipantCommand::Action(
                    CoordinationAction::AdvanceHorizon { tag: requested },
                ))
                .unwrap();
            assert!(matches!(
                participant.authorize_control(requested).unwrap(),
                FederateControlAuthorization::Authorized
            ));

            command_tx
                .send(ParticipantCommand::Action(
                    CoordinationAction::AdvanceHorizon { tag: requested },
                ))
                .unwrap();
            command_tx.send(terminal).unwrap();
            let authorization = participant.authorize_control(requested).unwrap();
            let matches_terminal = match authorization {
                FederateControlAuthorization::Aborted => expects_abort,
                FederateControlAuthorization::LogicalHorizon(tag) => {
                    expects_horizon && tag == requested
                }
                FederateControlAuthorization::Stopped => !expects_abort && !expects_horizon,
                FederateControlAuthorization::Authorized
                | FederateControlAuthorization::Interrupted(_) => false,
            };
            assert!(matches_terminal);
        }
    }

    /// Verifies queued scheduler work also wins over a retained covering horizon.
    #[test]
    fn cached_control_authorization_observes_queued_event() {
        let enclave = EnclaveIndex::new(3);
        let (report_tx, _report_rx) = mpsc::channel();
        let (_command_tx, command_rx) = mpsc::channel();
        let (mut participant, event_tx) = participant_with_events(enclave, report_tx, command_rx);
        let requested = Tag::new(Duration::seconds(2), 0);
        participant.advance_horizon(requested);
        event_tx.send(AsyncEvent::shutdown(Duration::ZERO)).unwrap();

        assert!(matches!(
            participant.authorize_control(requested).unwrap(),
            FederateControlAuthorization::Interrupted(AsyncEvent::Shutdown { delay })
                if delay == Duration::ZERO
        ));
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
        let (participant, _event_tx) = participant_with_events(enclave, report_tx, command_rx);

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
        for panic_stop in [false, true] {
            let stops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let enclave = EnclaveIndex::new(3);
            let (event_tx, event_rx) = kanal::unbounded();
            let FederateCoordinationParts {
                coordinator,
                participants,
                ..
            } = FederateCoordinationParts::new(
                [(enclave, event_tx, event_rx)],
                LifecyclePolicy::KeepAlive,
                PublishAndStopFailingBackend {
                    panic_stop,
                    stops: stops.clone(),
                },
            )
            .unwrap();
            let (participant_enclave, mut participant) = participants.into_iter().next().unwrap();
            assert_eq!(participant_enclave, enclave);

            std::thread::scope(|scope| {
                let coordinator = scope.spawn(move || coordinator.run());
                assert!(matches!(
                    participant.wait().unwrap(),
                    FederateIdleWait::Aborted
                ));
                assert_eq!(
                    coordinator.join().unwrap().unwrap_err(),
                    FederateCoordinationError::BackendPublish {
                        message: "publication rejected".to_owned(),
                    }
                );
            });
            assert_eq!(stops.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }

    /// Verifies failed backend cleanup aborts an otherwise graceful idle stop.
    #[test]
    fn backend_stop_failure_precedes_graceful_idle_stop() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let FederateCoordinationParts {
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            StopFailingBackend,
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();

        std::thread::scope(|scope| {
            let coordinator = scope.spawn(move || coordinator.run());
            assert!(matches!(
                participant.wait().unwrap(),
                FederateIdleWait::Aborted
            ));
            assert_eq!(
                coordinator.join().unwrap().unwrap_err(),
                FederateCoordinationError::BackendStop {
                    message: "terminal cleanup rejected".to_owned(),
                }
            );
        });
    }

    /// Verifies failed backend cleanup aborts before a logical-horizon command is exposed.
    #[test]
    fn backend_stop_failure_precedes_graceful_logical_horizon() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let FederateCoordinationParts {
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::KeepAlive,
            StopFailingBackend,
        )
        .unwrap();
        let (_, mut participant) = participants.into_iter().next().unwrap();
        let horizon = Tag::new(Duration::seconds(1), 0);

        participant.logical_horizon_reached(horizon);
        let coordinator_error = coordinator.run().unwrap_err();
        assert_eq!(
            coordinator_error,
            FederateCoordinationError::BackendStop {
                message: "terminal cleanup rejected".to_owned(),
            }
        );
        assert_eq!(
            participant.terminal_after_event_channel_closed().unwrap(),
            Some(FederateTermination::Abort)
        );
    }

    /// Dropping an unstarted or unwinding coordinator releases its connected backend once.
    #[test]
    fn coordinator_drop_stops_backend_before_start_and_during_unwind() {
        for unwind in [false, true] {
            let (event_tx, event_rx) = kanal::unbounded();
            let (call_tx, call_rx) = mpsc::channel();
            let parts = FederateCoordinationParts::new(
                [(EnclaveIndex::new(3), event_tx, event_rx)],
                LifecyclePolicy::CoordinatedTermination,
                CallRecordingBackend { call_tx },
            )
            .unwrap();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _coordinator = parts.coordinator;
                if unwind {
                    panic!("injected coordinator unwind");
                }
            }));
            assert_eq!(outcome.is_err(), unwind);
            assert_eq!(call_rx.into_iter().collect::<Vec<_>>(), [BackendCall::Stop]);
        }
    }

    /// Verifies successful scheduler finalization remains present for fixed-point commands.
    #[test]
    fn successful_participant_waits_for_fixed_point_before_closing() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let (call_tx, call_rx) = mpsc::channel();
        let FederateCoordinationParts {
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            CallRecordingBackend { call_tx },
        )
        .unwrap();
        let (participant_enclave, mut participant) = participants.into_iter().next().unwrap();
        assert_eq!(participant_enclave, enclave);
        participant.participant_stopped();
        assert!(participant.termination.is_none());

        coordinator.run().unwrap();
        participant.finish_success().unwrap();

        assert_eq!(call_rx.into_iter().collect::<Vec<_>>(), [BackendCall::Stop]);
    }

    /// Verifies a queued terminal acknowledgement wins a late report disconnection.
    #[test]
    fn queued_stop_acknowledgement_wins_late_report_disconnection() {
        let enclave = EnclaveIndex::new(3);
        let (report_tx, report_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (mut participant, event_tx) = participant_with_events(enclave, report_tx, command_rx);
        command_tx
            .send(ParticipantCommand::Action(CoordinationAction::Stop))
            .unwrap();
        event_tx.close().unwrap();
        drop(report_rx);

        participant.participant_stopped();

        participant.finish_success().unwrap();
    }

    /// Verifies failed participant cleanup never publishes successful terminal idleness.
    #[test]
    fn failed_participant_drop_does_not_publish_idle() {
        let enclave = EnclaveIndex::new(3);
        let (event_tx, event_rx) = kanal::unbounded();
        let (call_tx, call_rx) = mpsc::channel();
        let FederateCoordinationParts {
            abort_handle,
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [(enclave, event_tx, event_rx)],
            LifecyclePolicy::TerminateWhenIdle,
            CallRecordingBackend { call_tx },
        )
        .unwrap();
        let (participant_enclave, participant) = participants.into_iter().next().unwrap();
        assert_eq!(participant_enclave, enclave);

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
        let (peer_tx, peer_rx) = kanal::unbounded();
        let FederateCoordinationParts {
            abort_handle,
            mut coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [
                (eventful, eventful_tx.clone(), eventful_rx),
                (peer, peer_tx, peer_rx),
            ],
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
        let mut participant_ports = participants.into_iter();
        let (eventful_enclave, mut eventful_participant) = participant_ports.next().unwrap();
        let (peer_enclave, mut peer_participant) = participant_ports.next().unwrap();
        assert_eq!((eventful_enclave, peer_enclave), (eventful, peer));
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
                        Ok(FederateIdleWait::Interrupted(AsyncEvent::Shutdown { delay }))
                            if *delay == Duration::ZERO
                    )
            }));
            assert!(observations
                .iter()
                .any(|(enclave, event)| *enclave == eventful
                    && matches!(event, Ok(FederateIdleWait::Stopped))));
            assert!(observations.iter().any(|(enclave, event)| *enclave == peer
                && matches!(event, Ok(FederateIdleWait::Stopped))));
        });
    }

    /// Verifies a unanimously idle kept-alive Federate progresses its backend and admits later work.
    ///
    /// Mutation caught: polling only while a finite acquisition is pending leaves the post-idle
    /// progress signal unsent and prevents later work from being admitted.
    #[test]
    fn kept_alive_all_idle_participants_wake_for_later_input() {
        let eventful = EnclaveIndex::new(3);
        let peer = EnclaveIndex::new(7);
        let (eventful_tx, eventful_rx) = kanal::unbounded();
        let (peer_tx, peer_rx) = kanal::unbounded();
        let (progress_tx, progress_rx) = mpsc::channel();
        let FederateCoordinationParts {
            abort_handle,
            coordinator,
            participants,
            ..
        } = FederateCoordinationParts::new(
            [
                (eventful, eventful_tx.clone(), eventful_rx),
                (peer, peer_tx, peer_rx),
            ],
            LifecyclePolicy::KeepAlive,
            IdlePollingBackend {
                local: LocalFederateCoordinationBackend::default(),
                idle_published: false,
                progress_tx,
            },
        )
        .unwrap();
        let mut participant_ports = participants.into_iter();
        let (eventful_enclave, mut eventful_participant) = participant_ports.next().unwrap();
        let (peer_enclave, mut peer_participant) = participant_ports.next().unwrap();
        assert_eq!((eventful_enclave, peer_enclave), (eventful, peer));
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

            let progressed = progress_rx.recv_timeout(StdDuration::from_millis(100));
            if progressed.is_ok() {
                eventful_tx
                    .send(AsyncEvent::Shutdown {
                        delay: Duration::ZERO,
                    })
                    .unwrap();
                assert!(matches!(
                    result_rx.recv_timeout(StdDuration::from_secs(1)).unwrap(),
                    (
                        enclave,
                        Ok(FederateIdleWait::Interrupted(AsyncEvent::Shutdown { delay }))
                    )
                        if enclave == eventful && delay == Duration::ZERO
                ));
            }
            abort_handle.abort();
            eventful_thread.join().unwrap();
            assert!(matches!(
                peer_thread.join().unwrap().unwrap(),
                FederateIdleWait::Aborted
            ));
            coordinator.join().unwrap().unwrap();
            assert!(
                progressed.is_ok(),
                "kept-alive idle coordination must continue backend progress"
            );
        });
    }
}
