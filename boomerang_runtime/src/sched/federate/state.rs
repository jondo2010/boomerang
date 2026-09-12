//! Pure lifecycle and logical-time coordination for one compiled Federate.
//!
//! The state machine aggregates scheduler candidates and completions by compiled [`EnclaveIndex`],
//! advances revision-bound fixed-point phases, and latches terminal stop or failure without owning
//! channels, clocks, scheduler storage, or a concrete coordination backend. A later adapter maps
//! these semantic [`CoordinationAction`] values onto runtime operations.

use tinymap::TinySecondaryMap;

use super::backend::{
    CoordinationRevision, FederateAcquisition, FederateCompletion, FederatePublication,
};
use crate::{image::EnclaveIndex, Tag};

/// Policy applied when every participant has published no next logical event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecyclePolicy {
    /// Remain parked until a changed candidate resumes the Federate.
    #[allow(dead_code)]
    KeepAlive,
    /// Confirm a stable fixed point and stop the Federate.
    TerminateWhenIdle,
    /// Require backend idle authority before the final local fixed point.
    CoordinatedTermination,
}

/// Current phase of the pure Federate coordination state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationPhase {
    /// At least one participant may still have logical work.
    Active,
    /// Participants are confirming the candidate revision before parking.
    Probing,
    /// Participants are entering their parked state.
    Parking,
    /// Parked participants are checking the candidate revision again.
    Rechecking,
    /// KeepAlive participants are unanimously idle and wakeable.
    Parked,
    /// Locally idle participants await backend authority while remaining wakeable.
    AwaitingIdleConfirmation,
    /// The Federate has committed stop or failure and accepts no more work.
    Stopped,
}

/// One participant's acknowledgement of a fixed-point phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Observation {
    /// The participant confirmed the probed revision.
    Probed,
    /// The participant entered its parked state.
    Parked,
    /// The participant rechecked the unchanged revision while parked.
    Rechecked,
}

/// Scheduler-originated input to the pure coordination state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchedulerMessage {
    /// Report work observed before the participant can publish its revised candidate.
    Active {
        /// Compiled participant whose work invalidates an idle or fixed-point candidate.
        enclave: EnclaveIndex,
    },
    /// Publish one participant's optional next logical event.
    Publish {
        /// Compiled participant reporting the candidate.
        enclave: EnclaveIndex,
        /// Earliest pending event, or `None` when the participant is idle.
        next_event: Option<Tag>,
    },
    /// Report the greatest logical tag completed by one participant.
    CompleteTag {
        /// Compiled participant reporting completion.
        enclave: EnclaveIndex,
        /// Monotonic participant completion tag.
        tag: Tag,
    },
    /// Report that a participant has observed terminal stop.
    ParticipantStopped {
        /// Compiled participant reporting stop.
        enclave: EnclaveIndex,
    },
    /// Report a supervision failure with an optional typed participant origin.
    Failed {
        /// Compiled origin when the failure belongs to a participant.
        enclave: Option<EnclaveIndex>,
    },
}

/// Semantic operation emitted by a pure coordination transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationAction {
    /// Publish the Federate-wide optional next logical event.
    Publish(
        /// Aggregate publication for the current revision.
        FederatePublication,
    ),
    /// Broadcast a monotonically acquired Federate grant horizon to every participant.
    AdvanceHorizon {
        /// Greatest logical tag authorized for control-only local advancement.
        tag: Tag,
    },
    /// Grant logical progress to a covered participant candidate.
    Grant {
        /// Compiled participant receiving the grant.
        enclave: EnclaveIndex,
        /// Candidate tag covered by the acquired Federate grant.
        tag: Tag,
    },
    /// Publish a newly safe Federate-wide completion frontier.
    Complete(
        /// Aggregate monotonic completion frontier.
        FederateCompletion,
    ),
    /// Ask every participant to confirm the candidate revision.
    Probe {
        /// Candidate revision being confirmed.
        revision: CoordinationRevision,
    },
    /// Ask every probed participant to park at the revision.
    Park {
        /// Candidate revision being parked.
        revision: CoordinationRevision,
    },
    /// Ask every parked participant to recheck the revision.
    Recheck {
        /// Candidate revision being rechecked.
        revision: CoordinationRevision,
    },
    /// Resume participants after a changed candidate invalidates fixed-point work.
    Resume {
        /// New candidate revision that invalidated the fixed point.
        revision: CoordinationRevision,
    },
    /// Stop the Federate after an unchanged fixed point or explicit request.
    Stop,
    /// Abort the Federate after the first supervision failure.
    Abort,
}

/// Invalid construction or transition at the pure coordination boundary.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CoordinationStateError {
    /// Construction received no compiled participants.
    #[error("a Federate coordination state requires at least one compiled Enclave")]
    NoParticipants,
    /// Construction received the same compiled participant more than once.
    #[error("compiled Enclave {enclave:?} occurs more than once in the Federate")]
    DuplicateEnclave {
        /// Repeated compiled participant identity.
        enclave: EnclaveIndex,
    },
    /// A transition names an identity outside this compiled Federate.
    #[error("compiled Enclave {enclave:?} does not belong to the Federate")]
    UnknownEnclave {
        /// Unrecognized compiled participant identity.
        enclave: EnclaveIndex,
    },
    /// A current-revision acknowledgement is ahead of the active phase.
    #[error("observation {observation:?} is invalid while coordination is {phase:?}")]
    InvalidObservationTransition {
        /// Phase that rejected the acknowledgement.
        phase: CoordinationPhase,
        /// Out-of-order acknowledgement that was rejected.
        observation: Observation,
    },
}

/// Mutable candidate, completion, and observation state for one participant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ParticipantState {
    /// Most recently published optional next logical event.
    candidate: Option<Tag>,
    /// Greatest logical completion tag reported by this participant.
    completed: Option<Tag>,
    /// Whether the candidate participates in the current aggregate publication.
    published: bool,
    /// Current phase acknowledgement, if this participant has supplied one.
    observation: Option<Observation>,
}

/// Pure coordination state for every compiled participant in one Federate.
#[derive(Debug)]
pub(crate) struct FederateCoordinationState {
    /// Sparse participant state keyed by original compiled identity.
    participants: TinySecondaryMap<EnclaveIndex, ParticipantState>,
    /// Version of the current aggregate candidate.
    revision: CoordinationRevision,
    /// Current publication awaiting acquisition, if any.
    pending_publication: Option<FederatePublication>,
    /// Greatest completion frontier emitted for the whole Federate.
    completed_frontier: Option<Tag>,
    /// Greatest valid backend grant retained across candidate revisions.
    grant_horizon: Option<Tag>,
    /// Typed origin of the first failure when one was supplied.
    first_failure: Option<EnclaveIndex>,
    /// Idle behavior selected by the runtime adapter.
    lifecycle: LifecyclePolicy,
    /// Revision of the final local fixed point authorized by the backend.
    idle_authorization: Option<CoordinationRevision>,
    /// Current active, fixed-point, or terminal phase.
    phase: CoordinationPhase,
}

impl FederateCoordinationState {
    /// Constructs state for a non-empty set of unique compiled participant identities.
    pub(crate) fn new(
        enclaves: impl IntoIterator<Item = EnclaveIndex>,
        lifecycle: LifecyclePolicy,
    ) -> Result<Self, CoordinationStateError> {
        let mut participants = TinySecondaryMap::new();
        for enclave in enclaves {
            if participants
                .insert(enclave, ParticipantState::default())
                .is_some()
            {
                return Err(CoordinationStateError::DuplicateEnclave { enclave });
            }
        }
        if participants.is_empty() {
            return Err(CoordinationStateError::NoParticipants);
        }

        Ok(Self {
            participants,
            revision: CoordinationRevision::new(0),
            pending_publication: None,
            completed_frontier: None,
            grant_horizon: None,
            first_failure: None,
            lifecycle,
            idle_authorization: None,
            phase: CoordinationPhase::Active,
        })
    }

    #[cfg(test)]
    /// Returns the current aggregate candidate revision for state-machine tests.
    pub(crate) const fn revision(&self) -> CoordinationRevision {
        self.revision
    }

    /// Returns the greatest valid Federate grant retained for control authorization.
    #[cfg(test)]
    pub(crate) const fn grant_horizon(&self) -> Option<Tag> {
        self.grant_horizon
    }

    /// Returns the current lifecycle or fixed-point phase.
    #[cfg(test)]
    pub(crate) const fn phase(&self) -> CoordinationPhase {
        self.phase
    }

    /// Returns whether stop or failure has made this state terminal.
    pub(crate) const fn is_stopped(&self) -> bool {
        matches!(self.phase, CoordinationPhase::Stopped)
    }

    /// Returns the typed participant origin retained from the first failure.
    #[cfg(test)]
    pub(crate) const fn first_failure(&self) -> Option<EnclaveIndex> {
        self.first_failure
    }

    /// Returns a known participant's optional candidate, preserving unknown identity separately.
    pub(crate) fn candidate(&self, enclave: EnclaveIndex) -> Option<Option<Tag>> {
        self.participants
            .get(enclave)
            .map(|participant| participant.candidate)
    }

    /// Returns the aggregate publication retained for acquisition assertions.
    #[cfg(test)]
    fn pending_publication(&self) -> Option<FederatePublication> {
        self.pending_publication
    }

    /// Applies one scheduler-originated transition and returns its semantic actions.
    pub(crate) fn handle_scheduler(
        &mut self,
        message: SchedulerMessage,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        match message {
            SchedulerMessage::Active { enclave } => self.activate(enclave),
            SchedulerMessage::Publish {
                enclave,
                next_event,
            } => self.publish(enclave, next_event),
            SchedulerMessage::CompleteTag { enclave, tag } => self.complete(enclave, tag),
            SchedulerMessage::ParticipantStopped { enclave } => {
                self.participant(enclave)?;
                Ok(self.stop())
            }
            SchedulerMessage::Failed { enclave } => self.fail(enclave),
        }
    }

    /// Applies an acquired Federate grant when it matches the current pending revision.
    pub(crate) fn handle_acquisition(
        &mut self,
        acquisition: FederateAcquisition,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        if acquisition.revision() != self.revision
            || self
                .pending_publication
                .map(|publication| publication.revision())
                != Some(self.revision)
        {
            return Ok(Vec::new());
        }

        let granted = acquisition.granted();
        let horizon = self
            .grant_horizon
            .map_or(granted, |existing| existing.max(granted));
        self.grant_horizon = Some(horizon);
        let mut actions = vec![CoordinationAction::AdvanceHorizon { tag: horizon }];
        actions.extend(
            self.participants
                .iter()
                .filter_map(|(enclave, participant)| {
                    participant
                        .published
                        .then_some(participant.candidate)
                        .flatten()
                        .filter(|candidate| *candidate <= granted)
                        .map(|candidate| CoordinationAction::Grant {
                            enclave,
                            tag: candidate,
                        })
                })
                .collect::<Vec<_>>(),
        );

        for action in &actions {
            if let CoordinationAction::Grant { enclave, .. } = *action {
                self.participants[enclave].published = false;
            }
        }
        self.pending_publication = None;

        Ok(actions)
    }

    /// Applies one revision-bound participant acknowledgement.
    ///
    /// Stale and obsolete acknowledgements are harmless; future acknowledgements for the current
    /// phase remain typed errors. A phase action is emitted only after every participant confirms.
    pub(crate) fn handle_observation(
        &mut self,
        enclave: EnclaveIndex,
        revision: CoordinationRevision,
        observation: Observation,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        self.participant(enclave)?;
        if self.is_stopped()
            || revision != self.revision
            || self.phase == CoordinationPhase::AwaitingIdleConfirmation
        {
            return Ok(Vec::new());
        }
        if matches!(
            (self.phase, observation),
            (CoordinationPhase::Parking, Observation::Probed)
                | (
                    CoordinationPhase::Rechecking,
                    Observation::Probed | Observation::Parked
                )
        ) {
            return Ok(Vec::new());
        }

        let expected = match self.phase {
            CoordinationPhase::Probing => Observation::Probed,
            CoordinationPhase::Parking => Observation::Parked,
            CoordinationPhase::Rechecking => Observation::Rechecked,
            phase => {
                return Err(CoordinationStateError::InvalidObservationTransition {
                    phase,
                    observation,
                });
            }
        };
        if observation != expected {
            return Err(CoordinationStateError::InvalidObservationTransition {
                phase: self.phase,
                observation,
            });
        }
        if self.participants[enclave].observation == Some(observation) {
            return Ok(Vec::new());
        }

        self.participants[enclave].observation = Some(observation);
        if self
            .participants
            .values()
            .any(|participant| participant.observation != Some(observation))
        {
            return Ok(Vec::new());
        }
        for (_, participant) in self.participants.iter_mut() {
            participant.observation = None;
        }

        let action = match observation {
            Observation::Probed => {
                self.phase = CoordinationPhase::Parking;
                CoordinationAction::Park { revision }
            }
            Observation::Parked => {
                self.phase = CoordinationPhase::Rechecking;
                CoordinationAction::Recheck { revision }
            }
            Observation::Rechecked => {
                if self.lifecycle == LifecyclePolicy::CoordinatedTermination
                    && self.idle_authorization != Some(revision)
                {
                    self.phase = CoordinationPhase::AwaitingIdleConfirmation;
                    return Ok(Vec::new());
                }
                return Ok(self.stop());
            }
        };
        Ok(vec![action])
    }

    /// Revision awaiting external idle authority, if any.
    pub(crate) fn idle_confirmation_revision(&self) -> Option<CoordinationRevision> {
        (self.phase == CoordinationPhase::AwaitingIdleConfirmation).then_some(self.revision)
    }

    /// Applies current backend authority, then requires a fresh full local fixed point.
    pub(crate) fn handle_idle_confirmation(
        &mut self,
        revision: CoordinationRevision,
    ) -> Vec<CoordinationAction> {
        if self.idle_confirmation_revision() != Some(revision) {
            return Vec::new();
        }
        // A new revision prevents delayed acknowledgements from the initial idle check
        // from satisfying this final check. Intervening work advances it again.
        self.revision = self.revision.next();
        self.pending_publication = None;
        self.idle_authorization = Some(self.revision);
        self.phase = CoordinationPhase::Probing;
        vec![CoordinationAction::Probe {
            revision: self.revision,
        }]
    }

    /// Enters terminal stop and emits `Stop` at most once.
    pub(crate) fn stop(&mut self) -> Vec<CoordinationAction> {
        if self.is_stopped() {
            return Vec::new();
        }
        self.phase = CoordinationPhase::Stopped;
        self.pending_publication = None;
        vec![CoordinationAction::Stop]
    }

    /// Invalidates an idle or fixed-point candidate before the next tag is known.
    fn activate(
        &mut self,
        enclave: EnclaveIndex,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        self.participant(enclave)?;
        if self.is_stopped() || self.phase == CoordinationPhase::Active {
            return Ok(Vec::new());
        }

        self.revision = self.revision.next();
        self.pending_publication = None;
        self.phase = CoordinationPhase::Active;
        for (_, participant) in self.participants.iter_mut() {
            participant.observation = None;
        }
        self.participants[enclave].published = false;

        Ok(vec![CoordinationAction::Resume {
            revision: self.revision,
        }])
    }

    /// Resolves a compiled participant or returns an identity error without mutation.
    fn participant(
        &self,
        enclave: EnclaveIndex,
    ) -> Result<&ParticipantState, CoordinationStateError> {
        self.participants
            .get(enclave)
            .ok_or(CoordinationStateError::UnknownEnclave { enclave })
    }

    /// Advances and emits the monotonic minimum participant completion frontier.
    fn complete(
        &mut self,
        enclave: EnclaveIndex,
        tag: Tag,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        let participant = self.participant(enclave)?;
        if self.is_stopped()
            || participant
                .completed
                .is_some_and(|completed| tag <= completed)
        {
            return Ok(Vec::new());
        }
        self.participants[enclave].completed = Some(tag);

        let Some(frontier) = self
            .participants
            .values()
            .map(|participant| participant.completed)
            .min()
            .flatten()
        else {
            return Ok(Vec::new());
        };
        if self
            .completed_frontier
            .is_some_and(|completed| frontier <= completed)
        {
            return Ok(Vec::new());
        }
        self.completed_frontier = Some(frontier);
        Ok(vec![CoordinationAction::Complete(FederateCompletion::new(
            frontier,
        ))])
    }

    /// Latches the first optional typed failure origin and emits `Abort` at most once.
    fn fail(
        &mut self,
        enclave: Option<EnclaveIndex>,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        if let Some(enclave) = enclave {
            self.participant(enclave)?;
        }
        if self.is_stopped() {
            return Ok(Vec::new());
        }
        self.first_failure = enclave;
        self.phase = CoordinationPhase::Stopped;
        self.pending_publication = None;
        Ok(vec![CoordinationAction::Abort])
    }

    /// Records a participant candidate and publishes when all candidates are available.
    fn publish(
        &mut self,
        enclave: EnclaveIndex,
        next_event: Option<Tag>,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        let participant = self.participant(enclave)?;
        if self.is_stopped() {
            return Ok(Vec::new());
        }
        let changed = participant.candidate != next_event;
        let was_published = participant.published;
        let resume = changed && self.phase != CoordinationPhase::Active;

        if changed {
            self.revision = self.revision.next();
            self.pending_publication = None;
            self.phase = CoordinationPhase::Active;
            for (_, participant) in self.participants.iter_mut() {
                participant.observation = None;
            }
        } else if was_published {
            let action = match self.phase {
                CoordinationPhase::Probing => Some(CoordinationAction::Probe {
                    revision: self.revision,
                }),
                CoordinationPhase::Parking => Some(CoordinationAction::Park {
                    revision: self.revision,
                }),
                CoordinationPhase::Rechecking => Some(CoordinationAction::Recheck {
                    revision: self.revision,
                }),
                CoordinationPhase::Active
                | CoordinationPhase::Parked
                | CoordinationPhase::AwaitingIdleConfirmation
                | CoordinationPhase::Stopped => None,
            };
            return Ok(action.into_iter().collect());
        }

        let participant = &mut self.participants[enclave];
        participant.candidate = next_event;
        participant.published = true;

        if self
            .participants
            .values()
            .any(|participant| !participant.published)
        {
            return Ok(Vec::new());
        }

        let next_event = self
            .participants
            .values()
            .filter_map(|participant| participant.candidate)
            .min();
        let publication = FederatePublication::new(self.revision, next_event);
        self.pending_publication = Some(publication);
        let mut actions = vec![CoordinationAction::Publish(publication)];
        if resume {
            actions.push(CoordinationAction::Resume {
                revision: self.revision,
            });
        }
        self.phase = match (next_event, self.lifecycle) {
            (None, LifecyclePolicy::KeepAlive) => CoordinationPhase::Parked,
            (
                None,
                LifecyclePolicy::TerminateWhenIdle | LifecyclePolicy::CoordinatedTermination,
            ) => {
                actions.push(CoordinationAction::Probe {
                    revision: self.revision,
                });
                CoordinationPhase::Probing
            }
            (Some(_), _) => CoordinationPhase::Active,
        };
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    //! Focused invariant tests for the pure Federate coordination state.

    use super::*;
    use crate::{image::EnclaveIndex, Duration, Tag};

    /// Publishes one candidate and returns the emitted actions.
    fn publish(
        state: &mut FederateCoordinationState,
        enclave: EnclaveIndex,
        next_event: Option<Tag>,
    ) -> Vec<CoordinationAction> {
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event,
            })
            .unwrap()
    }

    /// Applies one fixed-point observation without hiding typed transition errors.
    fn observe(
        state: &mut FederateCoordinationState,
        enclave: EnclaveIndex,
        revision: CoordinationRevision,
        observation: Observation,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        state.handle_observation(enclave, revision, observation)
    }

    /// Reports one participant completion and returns the emitted actions.
    fn complete(
        state: &mut FederateCoordinationState,
        enclave: EnclaveIndex,
        tag: Tag,
    ) -> Vec<CoordinationAction> {
        state
            .handle_scheduler(SchedulerMessage::CompleteTag { enclave, tag })
            .unwrap()
    }

    /// Reports one typed failure origin and returns the emitted actions.
    fn fail(
        state: &mut FederateCoordinationState,
        enclave: EnclaveIndex,
    ) -> Vec<CoordinationAction> {
        state
            .handle_scheduler(SchedulerMessage::Failed {
                enclave: Some(enclave),
            })
            .unwrap()
    }

    /// Observable candidate state captured alongside one transition's actions.
    #[derive(Debug, Eq, PartialEq)]
    struct CandidateSnapshot {
        /// Aggregate candidate revision after the transition.
        revision: CoordinationRevision,
        /// Requested participant candidates, including unknown identities.
        candidates: Vec<(EnclaveIndex, Option<Option<Tag>>)>,
        /// Aggregate publication awaiting a current-revision acquisition.
        pending_publication: Option<FederatePublication>,
        /// Literal semantic actions emitted by the transition.
        actions: Vec<CoordinationAction>,
    }

    /// Captures candidate-facing state and the supplied transition actions.
    fn snapshot(
        state: &FederateCoordinationState,
        enclaves: impl IntoIterator<Item = EnclaveIndex>,
        actions: Vec<CoordinationAction>,
    ) -> CandidateSnapshot {
        CandidateSnapshot {
            revision: state.revision(),
            candidates: enclaves
                .into_iter()
                .map(|enclave| (enclave, state.candidate(enclave)))
                .collect(),
            pending_publication: state.pending_publication(),
            actions,
        }
    }

    /// Verifies an acquisition grants only covered non-contiguous participant candidates.
    #[test]
    fn current_acquisition_grants_covered_candidates() {
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let first_tag = Tag::new(Duration::seconds(1), 0);
        let second_tag = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();

        let first_publication = publish(&mut state, first, Some(first_tag));
        let first_revision = state.revision();
        assert_eq!(
            snapshot(&state, [first, second], first_publication),
            CandidateSnapshot {
                revision: first_revision,
                candidates: vec![(first, Some(Some(first_tag))), (second, Some(None))],
                pending_publication: None,
                actions: vec![],
            }
        );
        let publication = publish(&mut state, second, Some(second_tag));
        let revision = state.revision();
        assert_eq!(
            snapshot(&state, [first, second], publication),
            CandidateSnapshot {
                revision,
                candidates: vec![
                    (first, Some(Some(first_tag))),
                    (second, Some(Some(second_tag)))
                ],
                pending_publication: Some(FederatePublication::new(revision, Some(first_tag))),
                actions: vec![CoordinationAction::Publish(FederatePublication::new(
                    revision,
                    Some(first_tag),
                ))],
            }
        );
        let grant = state
            .handle_acquisition(FederateAcquisition::new(revision, first_tag))
            .unwrap();
        assert_eq!(
            snapshot(&state, [first, second], grant),
            CandidateSnapshot {
                revision,
                candidates: vec![
                    (first, Some(Some(first_tag))),
                    (second, Some(Some(second_tag)))
                ],
                pending_publication: None,
                actions: vec![
                    CoordinationAction::AdvanceHorizon { tag: first_tag },
                    CoordinationAction::Grant {
                        enclave: first,
                        tag: first_tag,
                    },
                ],
            }
        );
        let republished = publish(&mut state, first, Some(first_tag));
        assert_eq!(
            snapshot(&state, [first, second], republished),
            CandidateSnapshot {
                revision,
                candidates: vec![
                    (first, Some(Some(first_tag))),
                    (second, Some(Some(second_tag)))
                ],
                pending_publication: Some(FederatePublication::new(revision, Some(first_tag))),
                actions: vec![CoordinationAction::Publish(FederatePublication::new(
                    revision,
                    Some(first_tag),
                ))],
            }
        );
    }

    /// Verifies a stale acquisition leaves current candidate state unchanged.
    #[test]
    fn stale_acquisition_is_ignored() {
        let enclave = EnclaveIndex::new(3);
        let first_tag = Tag::new(Duration::seconds(1), 0);
        let revised_tag = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        publish(&mut state, enclave, Some(first_tag));
        let stale = state.revision();
        let revised = publish(&mut state, enclave, Some(revised_tag));
        let current = state.revision();
        let horizon_before = state.grant_horizon();

        assert_ne!(stale, current);
        assert_eq!(
            snapshot(&state, [enclave], revised),
            CandidateSnapshot {
                revision: current,
                candidates: vec![(enclave, Some(Some(revised_tag)))],
                pending_publication: Some(FederatePublication::new(current, Some(revised_tag))),
                actions: vec![CoordinationAction::Publish(FederatePublication::new(
                    current,
                    Some(revised_tag),
                ))],
            }
        );
        let ignored = state
            .handle_acquisition(FederateAcquisition::new(stale, revised_tag))
            .unwrap();
        assert_eq!(
            snapshot(&state, [enclave], ignored),
            CandidateSnapshot {
                revision: current,
                candidates: vec![(enclave, Some(Some(revised_tag)))],
                pending_publication: Some(FederatePublication::new(current, Some(revised_tag))),
                actions: vec![],
            }
        );
        assert_eq!(state.grant_horizon(), horizon_before);
    }

    /// Verifies only a changed candidate advances the aggregate revision.
    #[test]
    fn changed_publication_revises_candidate() {
        let enclave = EnclaveIndex::new(7);
        let tag = Tag::new(Duration::seconds(1), 0);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        publish(&mut state, enclave, Some(tag));
        let published = state.revision();
        let unchanged = publish(&mut state, enclave, Some(tag));
        assert_eq!(
            snapshot(&state, [enclave], unchanged),
            CandidateSnapshot {
                revision: published,
                candidates: vec![(enclave, Some(Some(tag)))],
                pending_publication: Some(FederatePublication::new(published, Some(tag))),
                actions: vec![],
            }
        );
        let changed = publish(&mut state, enclave, None);
        assert_eq!(
            snapshot(&state, [enclave], changed),
            CandidateSnapshot {
                revision: published.next(),
                candidates: vec![(enclave, Some(None))],
                pending_publication: Some(FederatePublication::new(published.next(), None)),
                actions: vec![CoordinationAction::Publish(FederatePublication::new(
                    published.next(),
                    None,
                ))],
            }
        );
    }

    /// Verifies unanimous idle emits the exact action sequence selected by lifecycle policy.
    #[test]
    fn all_idle_obeys_lifecycle_policy() {
        // Mutation caught: omit the Probe action or enter idle before every participant publishes.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let idle_revision = CoordinationRevision::new(0);
        for (policy, expected_phase, expected_actions) in [
            (
                LifecyclePolicy::KeepAlive,
                CoordinationPhase::Parked,
                vec![CoordinationAction::Publish(FederatePublication::new(
                    idle_revision,
                    None,
                ))],
            ),
            (
                LifecyclePolicy::TerminateWhenIdle,
                CoordinationPhase::Probing,
                vec![
                    CoordinationAction::Publish(FederatePublication::new(idle_revision, None)),
                    CoordinationAction::Probe {
                        revision: idle_revision,
                    },
                ],
            ),
        ] {
            let mut state = FederateCoordinationState::new([first, second], policy).unwrap();
            assert!(publish(&mut state, first, None).is_empty());
            assert_eq!(state.phase(), CoordinationPhase::Active);
            let actions = publish(&mut state, second, None);
            assert_eq!(actions, expected_actions);
            assert_eq!(state.phase(), expected_phase);
            assert!(!state.is_stopped());

            let resumed_revision = idle_revision.next();
            assert_eq!(
                state
                    .handle_scheduler(SchedulerMessage::Active { enclave: first })
                    .unwrap(),
                vec![CoordinationAction::Resume {
                    revision: resumed_revision,
                }]
            );
            assert_eq!(state.phase(), CoordinationPhase::Active);
            assert_eq!(state.revision(), resumed_revision);
            assert_eq!(state.pending_publication(), None);

            if policy == LifecyclePolicy::KeepAlive {
                let wake_tag = Tag::new(Duration::seconds(1), 0);
                let published_revision = resumed_revision.next();
                assert_eq!(
                    state
                        .handle_scheduler(SchedulerMessage::Publish {
                            enclave: first,
                            next_event: Some(wake_tag),
                        })
                        .unwrap(),
                    vec![CoordinationAction::Publish(FederatePublication::new(
                        published_revision,
                        Some(wake_tag),
                    ))]
                );
                assert_eq!(state.phase(), CoordinationPhase::Active);
                assert!(!state.is_stopped());
            }
        }
    }

    /// Catches local idle stop without backend authority and reuse after intervening work.
    #[test]
    fn coordinated_idle_requires_current_authority_and_a_new_fixed_point() {
        let enclave = EnclaveIndex::new(3);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::CoordinatedTermination)
                .unwrap();
        publish(&mut state, enclave, None);
        let revision = state.revision();
        for observation in [
            Observation::Probed,
            Observation::Parked,
            Observation::Rechecked,
        ] {
            observe(&mut state, enclave, revision, observation).unwrap();
        }
        assert!(!state.is_stopped());
        assert_eq!(state.idle_confirmation_revision(), Some(revision));
        // Duplicate reports from the completed first check remain harmless while waiting.
        for observation in [
            Observation::Probed,
            Observation::Parked,
            Observation::Rechecked,
        ] {
            assert!(observe(&mut state, enclave, revision, observation)
                .unwrap()
                .is_empty());
        }
        assert!(state.handle_idle_confirmation(revision.next()).is_empty());
        assert_eq!(
            state.handle_idle_confirmation(revision),
            vec![CoordinationAction::Probe {
                revision: revision.next(),
            }]
        );
        assert!(!state.is_stopped());
        // Old acknowledgements must not satisfy the post-confirmation fixed point.
        assert!(
            observe(&mut state, enclave, revision, Observation::Rechecked)
                .unwrap()
                .is_empty()
        );
        state
            .handle_scheduler(SchedulerMessage::Active { enclave })
            .unwrap();
        publish(&mut state, enclave, None);
        let current = state.revision();
        for observation in [
            Observation::Probed,
            Observation::Parked,
            Observation::Rechecked,
        ] {
            observe(&mut state, enclave, current, observation).unwrap();
        }
        assert!(!state.is_stopped());
        assert_eq!(state.idle_confirmation_revision(), Some(current));
        assert!(state.handle_idle_confirmation(revision).is_empty());
        state.handle_idle_confirmation(current);
        let confirmed = state.revision();
        observe(&mut state, enclave, confirmed, Observation::Probed).unwrap();
        observe(&mut state, enclave, confirmed, Observation::Parked).unwrap();
        assert!(!state.is_stopped());
        assert_eq!(
            observe(&mut state, enclave, confirmed, Observation::Rechecked).unwrap(),
            vec![CoordinationAction::Stop]
        );
    }

    /// Verifies each fixed-point phase requires unanimous, idempotent acknowledgements.
    #[test]
    fn unchanged_fixed_point_commits_once() {
        // Mutation caught: advance a phase before every participant confirms it.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::TerminateWhenIdle)
                .unwrap();
        assert!(publish(&mut state, first, None).is_empty());
        publish(&mut state, second, None);
        let revision = state.revision();
        assert_eq!(
            observe(&mut state, first, revision, Observation::Parked),
            Err(CoordinationStateError::InvalidObservationTransition {
                phase: CoordinationPhase::Probing,
                observation: Observation::Parked,
            })
        );
        assert!(observe(&mut state, first, revision, Observation::Probed)
            .unwrap()
            .is_empty());
        assert_eq!(
            publish(&mut state, first, None),
            vec![CoordinationAction::Probe { revision }]
        );
        assert_eq!(state.revision(), revision);
        assert!(observe(&mut state, first, revision, Observation::Probed)
            .unwrap()
            .is_empty());
        assert_eq!(
            observe(&mut state, second, revision, Observation::Probed).unwrap(),
            vec![CoordinationAction::Park { revision }]
        );
        assert!(observe(&mut state, first, revision, Observation::Probed)
            .unwrap()
            .is_empty());
        assert_eq!(
            observe(&mut state, first, revision, Observation::Rechecked),
            Err(CoordinationStateError::InvalidObservationTransition {
                phase: CoordinationPhase::Parking,
                observation: Observation::Rechecked,
            })
        );
        assert!(observe(&mut state, first, revision, Observation::Parked)
            .unwrap()
            .is_empty());
        assert_eq!(
            observe(&mut state, second, revision, Observation::Parked).unwrap(),
            vec![CoordinationAction::Recheck { revision }]
        );
        assert!(observe(&mut state, first, revision, Observation::Parked)
            .unwrap()
            .is_empty());
        assert!(observe(&mut state, first, revision, Observation::Rechecked)
            .unwrap()
            .is_empty());
        assert_eq!(
            observe(&mut state, second, revision, Observation::Rechecked).unwrap(),
            vec![CoordinationAction::Stop]
        );
        assert!(observe(&mut state, first, revision, Observation::Rechecked)
            .unwrap()
            .is_empty());
    }

    /// Verifies completion advances only at the monotonic aggregate safe frontier.
    #[test]
    fn completion_advances_only_at_safe_frontier() {
        // Mutation caught: publish an individual completion instead of the monotonic minimum.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let one = Tag::new(Duration::seconds(1), 0);
        let two = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();
        assert!(complete(&mut state, first, two).is_empty());
        assert_eq!(
            complete(&mut state, second, one),
            vec![CoordinationAction::Complete(FederateCompletion::new(one))]
        );
        assert_eq!(
            complete(&mut state, second, two),
            vec![CoordinationAction::Complete(FederateCompletion::new(two))]
        );
    }

    /// Verifies stop is terminal and emits its action at most once.
    #[test]
    fn stop_is_terminal_and_idempotent() {
        // Mutation caught: reject a scheduler-observed stop or emit stop more than once.
        let enclave = EnclaveIndex::new(3);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        assert_eq!(
            state
                .handle_scheduler(SchedulerMessage::ParticipantStopped { enclave })
                .unwrap(),
            vec![CoordinationAction::Stop]
        );
        assert!(state.stop().is_empty());
        assert!(state
            .handle_scheduler(SchedulerMessage::ParticipantStopped { enclave })
            .unwrap()
            .is_empty());
    }

    /// Verifies failure retains the first typed origin and emits one abort.
    #[test]
    fn first_failure_is_retained() {
        // Mutation caught: overwrite the first typed origin or emit more than one abort.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();
        assert_eq!(fail(&mut state, first), vec![CoordinationAction::Abort]);
        assert!(fail(&mut state, second).is_empty());
        assert_eq!(state.first_failure(), Some(first));
    }
}
