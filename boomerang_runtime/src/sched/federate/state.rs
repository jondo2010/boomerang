// The adapter that consumes this pure core is added in a later task.
#![allow(dead_code)]

use tinymap::TinySecondaryMap;

use super::{CoordinationRevision, FederateAcquisition, FederateCompletion, FederatePublication};
use crate::{image::EnclaveIndex, Tag};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecyclePolicy {
    KeepAlive,
    TerminateWhenIdle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationPhase {
    Active,
    Probing,
    Parking,
    Rechecking,
    Parked,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Observation {
    Probed,
    Parked,
    Rechecked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchedulerMessage {
    Publish {
        enclave: EnclaveIndex,
        next_event: Option<Tag>,
    },
    CompleteTag {
        enclave: EnclaveIndex,
        tag: Tag,
    },
    ParticipantStopped {
        enclave: EnclaveIndex,
    },
    Failed {
        enclave: Option<EnclaveIndex>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationAction {
    Publish(FederatePublication),
    Grant { enclave: EnclaveIndex, tag: Tag },
    Complete(FederateCompletion),
    Probe { revision: CoordinationRevision },
    Park { revision: CoordinationRevision },
    Recheck { revision: CoordinationRevision },
    Resume { revision: CoordinationRevision },
    Stop,
    Abort,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CoordinationStateError {
    #[error("a Federate coordination state requires at least one compiled Enclave")]
    NoParticipants,
    #[error("compiled Enclave {enclave:?} occurs more than once in the Federate")]
    DuplicateEnclave { enclave: EnclaveIndex },
    #[error("compiled Enclave {enclave:?} does not belong to the Federate")]
    UnknownEnclave { enclave: EnclaveIndex },
    #[error("observation {observation:?} is invalid while coordination is {phase:?}")]
    InvalidObservationTransition {
        phase: CoordinationPhase,
        observation: Observation,
    },
    #[error("compiled Enclave {enclave:?} stopped before Federate coordination became terminal")]
    ParticipantStoppedBeforeTerminal { enclave: EnclaveIndex },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParticipantState {
    candidate: Option<Tag>,
    completed: Option<Tag>,
    published: bool,
    observation: Option<Observation>,
}

impl ParticipantState {
    const fn new() -> Self {
        Self {
            candidate: None,
            completed: None,
            published: false,
            observation: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct FederateCoordinationState {
    participants: TinySecondaryMap<EnclaveIndex, ParticipantState>,
    revision: CoordinationRevision,
    pending_publication: Option<FederatePublication>,
    completed_frontier: Option<Tag>,
    first_failure: Option<EnclaveIndex>,
    lifecycle: LifecyclePolicy,
    phase: CoordinationPhase,
}

impl FederateCoordinationState {
    pub(crate) fn new(
        enclaves: impl IntoIterator<Item = EnclaveIndex>,
        lifecycle: LifecyclePolicy,
    ) -> Result<Self, CoordinationStateError> {
        let mut participants = TinySecondaryMap::new();
        for enclave in enclaves {
            if participants
                .insert(enclave, ParticipantState::new())
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
            first_failure: None,
            lifecycle,
            phase: CoordinationPhase::Active,
        })
    }

    pub(crate) const fn revision(&self) -> CoordinationRevision {
        self.revision
    }

    pub(crate) const fn phase(&self) -> CoordinationPhase {
        self.phase
    }

    pub(crate) const fn is_stopped(&self) -> bool {
        matches!(self.phase, CoordinationPhase::Stopped)
    }

    pub(crate) const fn first_failure(&self) -> Option<EnclaveIndex> {
        self.first_failure
    }

    pub(crate) fn candidate(&self, enclave: EnclaveIndex) -> Option<Option<Tag>> {
        self.participants
            .get(enclave)
            .map(|participant| participant.candidate)
    }

    #[cfg(test)]
    fn pending_publication(&self) -> Option<FederatePublication> {
        self.pending_publication
    }

    pub(crate) fn handle_scheduler(
        &mut self,
        message: SchedulerMessage,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        match message {
            SchedulerMessage::Publish {
                enclave,
                next_event,
            } => self.publish(enclave, next_event),
            SchedulerMessage::CompleteTag { enclave, tag } => self.complete(enclave, tag),
            SchedulerMessage::ParticipantStopped { enclave } => {
                self.participant(enclave)?;
                if self.is_stopped() {
                    Ok(Vec::new())
                } else {
                    Err(CoordinationStateError::ParticipantStoppedBeforeTerminal { enclave })
                }
            }
            SchedulerMessage::Failed { enclave } => self.fail(enclave),
        }
    }

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
        let actions = self
            .participants
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
            .collect::<Vec<_>>();

        for action in &actions {
            if let CoordinationAction::Grant { enclave, .. } = *action {
                self.participants[enclave].published = false;
            }
        }
        self.pending_publication = None;

        Ok(actions)
    }

    pub(crate) fn handle_observation(
        &mut self,
        enclave: EnclaveIndex,
        revision: CoordinationRevision,
        observation: Observation,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        self.participant(enclave)?;
        if self.is_stopped() || revision != self.revision {
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
                return Ok(self.stop());
            }
        };
        Ok(vec![action])
    }

    pub(crate) fn stop(&mut self) -> Vec<CoordinationAction> {
        if self.is_stopped() {
            return Vec::new();
        }
        self.phase = CoordinationPhase::Stopped;
        self.pending_publication = None;
        vec![CoordinationAction::Stop]
    }

    fn participant(
        &self,
        enclave: EnclaveIndex,
    ) -> Result<&ParticipantState, CoordinationStateError> {
        self.participants
            .get(enclave)
            .ok_or(CoordinationStateError::UnknownEnclave { enclave })
    }

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
            .collect::<Option<Vec<_>>>()
            .and_then(|completed| completed.into_iter().min())
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
            return Ok(Vec::new());
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
            (None, LifecyclePolicy::TerminateWhenIdle) => {
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
    use super::*;
    use crate::{image::EnclaveIndex, Duration, Tag};

    #[derive(Debug, Eq, PartialEq)]
    struct CandidateSnapshot {
        revision: CoordinationRevision,
        candidates: Vec<(EnclaveIndex, Option<Option<Tag>>)>,
        pending_publication: Option<FederatePublication>,
        actions: Vec<CoordinationAction>,
    }

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

    #[test]
    fn current_acquisition_grants_covered_candidates() {
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let first_tag = Tag::new(Duration::seconds(1), 0);
        let second_tag = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();

        let first_publication = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave: first,
                next_event: Some(first_tag),
            })
            .unwrap();
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
        let publication = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave: second,
                next_event: Some(second_tag),
            })
            .unwrap();
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
                actions: vec![CoordinationAction::Grant {
                    enclave: first,
                    tag: first_tag,
                }],
            }
        );
        let republished = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave: first,
                next_event: Some(first_tag),
            })
            .unwrap();
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

    #[test]
    fn stale_acquisition_is_ignored() {
        let enclave = EnclaveIndex::new(3);
        let first_tag = Tag::new(Duration::seconds(1), 0);
        let revised_tag = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: Some(first_tag),
            })
            .unwrap();
        let stale = state.revision();
        let revised = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: Some(revised_tag),
            })
            .unwrap();
        let current = state.revision();

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
    }

    #[test]
    fn changed_publication_revises_candidate() {
        let enclave = EnclaveIndex::new(7);
        let tag = Tag::new(Duration::seconds(1), 0);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: Some(tag),
            })
            .unwrap();
        let published = state.revision();
        let unchanged = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: Some(tag),
            })
            .unwrap();
        assert_eq!(
            snapshot(&state, [enclave], unchanged),
            CandidateSnapshot {
                revision: published,
                candidates: vec![(enclave, Some(Some(tag)))],
                pending_publication: Some(FederatePublication::new(published, Some(tag))),
                actions: vec![],
            }
        );
        let changed = state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: None,
            })
            .unwrap();
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

    #[test]
    fn all_idle_obeys_lifecycle_policy() {
        // Mutation caught: route unanimous idle through the same phase for both policies.
        let enclave = EnclaveIndex::new(3);
        for (policy, expected_phase) in [
            (LifecyclePolicy::KeepAlive, CoordinationPhase::Parked),
            (
                LifecyclePolicy::TerminateWhenIdle,
                CoordinationPhase::Probing,
            ),
        ] {
            let mut state = FederateCoordinationState::new([enclave], policy).unwrap();
            let actions = state
                .handle_scheduler(SchedulerMessage::Publish {
                    enclave,
                    next_event: None,
                })
                .unwrap();
            assert!(actions.iter().any(|action| matches!(
                action,
                CoordinationAction::Publish(publication) if publication.next_event().is_none()
            )));
            assert_eq!(state.phase(), expected_phase);
            assert!(!state.is_stopped());

            if policy == LifecyclePolicy::KeepAlive {
                let wake_tag = Tag::new(Duration::seconds(1), 0);
                let resumed_revision = CoordinationRevision::new(1);
                assert_eq!(
                    state
                        .handle_scheduler(SchedulerMessage::Publish {
                            enclave,
                            next_event: Some(wake_tag),
                        })
                        .unwrap(),
                    vec![
                        CoordinationAction::Publish(FederatePublication::new(
                            resumed_revision,
                            Some(wake_tag),
                        )),
                        CoordinationAction::Resume {
                            revision: resumed_revision,
                        },
                    ]
                );
                assert_eq!(state.phase(), CoordinationPhase::Active);
                assert!(!state.is_stopped());
            }
        }
    }

    #[test]
    fn unchanged_fixed_point_commits_once() {
        // Mutation caught: skip, reorder, or repeat a revision-bound fixed-point action.
        let enclave = EnclaveIndex::new(3);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::TerminateWhenIdle).unwrap();
        state
            .handle_scheduler(SchedulerMessage::Publish {
                enclave,
                next_event: None,
            })
            .unwrap();
        let revision = state.revision();
        assert_eq!(
            state
                .handle_observation(enclave, revision, Observation::Probed)
                .unwrap(),
            vec![CoordinationAction::Park { revision }]
        );
        assert_eq!(
            state
                .handle_observation(enclave, revision, Observation::Parked)
                .unwrap(),
            vec![CoordinationAction::Recheck { revision }]
        );
        assert_eq!(
            state
                .handle_observation(enclave, revision, Observation::Rechecked)
                .unwrap(),
            vec![CoordinationAction::Stop]
        );
        assert!(state
            .handle_observation(enclave, revision, Observation::Rechecked)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn completion_advances_only_at_safe_frontier() {
        // Mutation caught: publish an individual completion instead of the monotonic minimum.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let one = Tag::new(Duration::seconds(1), 0);
        let two = Tag::new(Duration::seconds(2), 0);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();
        assert!(state
            .handle_scheduler(SchedulerMessage::CompleteTag {
                enclave: first,
                tag: two,
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            state
                .handle_scheduler(SchedulerMessage::CompleteTag {
                    enclave: second,
                    tag: one,
                })
                .unwrap(),
            vec![CoordinationAction::Complete(FederateCompletion::new(one))]
        );
        assert_eq!(
            state
                .handle_scheduler(SchedulerMessage::CompleteTag {
                    enclave: second,
                    tag: two,
                })
                .unwrap(),
            vec![CoordinationAction::Complete(FederateCompletion::new(two))]
        );
    }

    #[test]
    fn stop_is_terminal_and_idempotent() {
        // Mutation caught: emit stop twice or process a scheduler message after terminal stop.
        let enclave = EnclaveIndex::new(3);
        let mut state =
            FederateCoordinationState::new([enclave], LifecyclePolicy::KeepAlive).unwrap();
        assert_eq!(state.stop(), vec![CoordinationAction::Stop]);
        assert!(state.stop().is_empty());
        assert!(state
            .handle_scheduler(SchedulerMessage::ParticipantStopped { enclave })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn first_failure_is_retained() {
        // Mutation caught: overwrite the first typed origin or emit more than one abort.
        let first = EnclaveIndex::new(3);
        let second = EnclaveIndex::new(7);
        let mut state =
            FederateCoordinationState::new([first, second], LifecyclePolicy::KeepAlive).unwrap();
        assert_eq!(
            state
                .handle_scheduler(SchedulerMessage::Failed {
                    enclave: Some(first),
                })
                .unwrap(),
            vec![CoordinationAction::Abort]
        );
        assert!(state
            .handle_scheduler(SchedulerMessage::Failed {
                enclave: Some(second),
            })
            .unwrap()
            .is_empty());
        assert_eq!(state.first_failure(), Some(first));
    }
}
