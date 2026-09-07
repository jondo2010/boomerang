// The adapter that consumes this pure core is added in a later task.
#![allow(dead_code)]

use tinymap::TinySecondaryMap;

use super::{CoordinationRevision, FederateAcquisition, FederateCompletion, FederatePublication};
use crate::{image::EnclaveIndex, Tag};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecyclePolicy {
    KeepAlive,
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
    #[error("this scheduler transition belongs to a later coordination-state task")]
    DeferredSchedulerTransition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParticipantState {
    candidate: Option<Tag>,
    published: bool,
}

impl ParticipantState {
    const fn new() -> Self {
        Self {
            candidate: None,
            published: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct FederateCoordinationState {
    participants: TinySecondaryMap<EnclaveIndex, ParticipantState>,
    revision: CoordinationRevision,
    pending_publication: Option<FederatePublication>,
    _lifecycle: LifecyclePolicy,
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
            _lifecycle: lifecycle,
        })
    }

    pub(crate) const fn revision(&self) -> CoordinationRevision {
        self.revision
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
            SchedulerMessage::CompleteTag { enclave, .. }
            | SchedulerMessage::ParticipantStopped { enclave }
            | SchedulerMessage::Failed {
                enclave: Some(enclave),
            } => {
                self.participant(enclave)?;
                Err(CoordinationStateError::DeferredSchedulerTransition)
            }
            SchedulerMessage::Failed { enclave: None } => {
                Err(CoordinationStateError::DeferredSchedulerTransition)
            }
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

    fn participant(
        &self,
        enclave: EnclaveIndex,
    ) -> Result<&ParticipantState, CoordinationStateError> {
        self.participants
            .get(enclave)
            .ok_or(CoordinationStateError::UnknownEnclave { enclave })
    }

    fn publish(
        &mut self,
        enclave: EnclaveIndex,
        next_event: Option<Tag>,
    ) -> Result<Vec<CoordinationAction>, CoordinationStateError> {
        let participant = self.participant(enclave)?;
        let changed = participant.candidate != next_event;
        let was_published = participant.published;

        if changed {
            self.revision = self.revision.next();
            self.pending_publication = None;
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
        Ok(vec![CoordinationAction::Publish(publication)])
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
}
