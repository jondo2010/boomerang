use std::collections::BTreeMap;

use boomerang_runtime::{EnclaveKey, FrontierPublication, LogicalTimeFrontier, Tag};

use super::FederateCoordinationLayout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoordinationAction {
    RequestNet {
        tag: Tag,
    },
    WakeParticipant {
        participant: EnclaveKey,
        tag: Tag,
        observation_epoch: u64,
    },
    ReleaseAcquire {
        participant: EnclaveKey,
        request_id: u64,
        publication_sequence: u64,
        tag: Tag,
    },
    ReleaseCompletion {
        participant: EnclaveKey,
        request_id: u64,
    },
    ReportLtc {
        tag: Tag,
    },
    FailRequest {
        participant: EnclaveKey,
        request_id: u64,
        reason: String,
    },
    SendStop,
}

#[derive(Debug)]
struct PendingAcquire {
    request_id: u64,
    publication_sequence: u64,
    tag: Tag,
}

#[derive(Debug)]
struct PendingCompletion {
    request_id: u64,
}

#[derive(Debug)]
struct ParticipantState {
    publication_sequence: Option<u64>,
    frontier: Option<LogicalTimeFrontier>,
    pending: Option<PendingAcquire>,
    pending_completion: Option<PendingCompletion>,
    certificate_epoch: Option<u64>,
    completed: Option<Tag>,
    finished: bool,
}

#[derive(Debug)]
pub(crate) struct FederateCoordinationState {
    participants: BTreeMap<EnclaveKey, ParticipantState>,
    advertised_net: Option<Tag>,
    grant_coverage: Option<Tag>,
    round: Option<(Tag, u64)>,
    observation_epoch: u64,
    stopped: bool,
    terminal_error: Option<String>,
}

impl FederateCoordinationState {
    pub(crate) fn new(layout: FederateCoordinationLayout) -> Self {
        let participants = layout
            .participants()
            .iter()
            .copied()
            .map(|participant| {
                (
                    participant,
                    ParticipantState {
                        publication_sequence: None,
                        frontier: None,
                        pending: None,
                        pending_completion: None,
                        certificate_epoch: None,
                        completed: None,
                        finished: false,
                    },
                )
            })
            .collect();
        Self {
            participants,
            advertised_net: None,
            grant_coverage: None,
            round: None,
            observation_epoch: 0,
            stopped: false,
            terminal_error: None,
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped
    }

    pub(crate) fn terminal_error(&self) -> Option<&str> {
        self.terminal_error.as_deref()
    }

    pub(crate) fn fail(&mut self, reason: String) -> Vec<CoordinationAction> {
        if self.terminal_error.is_some() {
            return Vec::new();
        }
        self.terminal_error = Some(reason.clone());
        let mut actions = Vec::new();
        for (&participant, state) in &mut self.participants {
            if let Some(pending) = state.pending.take() {
                actions.push(CoordinationAction::FailRequest {
                    participant,
                    request_id: pending.request_id,
                    reason: reason.clone(),
                });
            }
            if let Some(pending) = state.pending_completion.take() {
                actions.push(CoordinationAction::FailRequest {
                    participant,
                    request_id: pending.request_id,
                    reason: reason.clone(),
                });
            }
        }
        if !self.stopped {
            self.stopped = true;
            actions.push(CoordinationAction::SendStop);
        }
        actions
    }

    pub(crate) fn release_request(&mut self, participant: EnclaveKey, request_id: u64) {
        let Some(state) = self.participants.get_mut(&participant) else {
            return;
        };
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            state.pending = None;
        }
        if state
            .pending_completion
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            state.pending_completion = None;
        }
    }

    pub(crate) fn publish(
        &mut self,
        participant: EnclaveKey,
        sequence: u64,
        publication: FrontierPublication,
    ) -> Result<Vec<CoordinationAction>, String> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        let state = self
            .participants
            .get(&participant)
            .ok_or_else(|| format!("unknown participant {participant}"))?;
        if state.finished {
            return if publication.frontier == LogicalTimeFrontier::Finished {
                Ok(Vec::new())
            } else {
                Err(format!("participant {participant} already finished"))
            };
        }
        if state
            .publication_sequence
            .is_some_and(|previous| sequence <= previous)
        {
            return Err(format!(
                "non-monotonic publication sequence {sequence} for participant {participant}"
            ));
        }

        let frontier_changed = state.frontier != Some(publication.frontier);
        let finishing = publication.frontier == LogicalTimeFrontier::Finished;
        let has_certificate = self
            .participants
            .values()
            .any(|state| state.certificate_epoch.is_some());
        let mut actions = Vec::new();
        if frontier_changed && has_certificate {
            self.observation_epoch = self.observation_epoch.wrapping_add(1);
            for state in self.participants.values_mut() {
                state.certificate_epoch = None;
            }
            if finishing {
                self.participants
                    .get_mut(&participant)
                    .expect("checked above")
                    .finished = true;
            }
            if let Some((tag, _)) = self.round {
                self.round = Some((tag, self.observation_epoch));
                actions.extend(self.wake_actions(tag, self.observation_epoch));
            }
        }

        let state = self
            .participants
            .get_mut(&participant)
            .expect("checked above");
        if let Some(pending) = state.pending.take() {
            actions.push(CoordinationAction::FailRequest {
                participant,
                request_id: pending.request_id,
                reason: "superseded by newer frontier publication".into(),
            });
        }
        state.publication_sequence = Some(sequence);
        state.frontier = Some(publication.frontier);
        if let (Some(wake), Some((round_tag, epoch))) = (publication.consumed_wake, self.round) {
            let certifies = match publication.frontier {
                LogicalTimeFrontier::Idle => true,
                LogicalTimeFrontier::Candidate(tag) => tag > round_tag,
                LogicalTimeFrontier::Finished => false,
            };
            if certifies && wake.tag == round_tag && wake.observation_epoch == epoch {
                state.certificate_epoch = Some(epoch);
            }
        }
        if finishing {
            state.finished = true;
            state.certificate_epoch = None;
            if self.participants.values().all(|state| state.finished) && !self.stopped {
                self.stopped = true;
                actions.push(CoordinationAction::SendStop);
                return Ok(actions);
            }
        }
        if frontier_changed && self.round.is_none() {
            if let Some(tag) = self
                .minimum_candidate()
                .filter(|tag| self.grant_coverage.is_some_and(|grant| *tag <= grant))
            {
                self.observation_epoch = self.observation_epoch.wrapping_add(1);
                for state in self.participants.values_mut() {
                    state.certificate_epoch = None;
                }
                self.round = Some((tag, self.observation_epoch));
                actions.extend(self.wake_actions(tag, self.observation_epoch));
            }
        }
        actions.extend(self.recompute_net());
        if let Some((tag, epoch)) = self.round {
            if self
                .participants
                .values()
                .filter(|state| !state.finished)
                .all(|state| state.certificate_epoch == Some(epoch))
            {
                actions.push(CoordinationAction::ReportLtc { tag });
                self.round = None;
                actions.extend(self.recompute_net());
            }
        }
        Ok(actions)
    }

    pub(crate) fn acquire(
        &mut self,
        participant: EnclaveKey,
        request_id: u64,
        publication_sequence: u64,
        tag: Tag,
    ) -> Result<Vec<CoordinationAction>, String> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        let state = self
            .participants
            .get_mut(&participant)
            .ok_or_else(|| format!("unknown participant {participant}"))?;
        if state.publication_sequence != Some(publication_sequence) {
            return Err("acquire does not match latest publication".into());
        }
        let mut actions = Vec::new();
        if let Some(old) = state.pending.take() {
            actions.push(CoordinationAction::FailRequest {
                participant,
                request_id: old.request_id,
                reason: "superseded by newer acquire".into(),
            });
        }
        if self.grant_coverage.is_some_and(|grant| grant >= tag) {
            actions.push(CoordinationAction::ReleaseAcquire {
                participant,
                request_id,
                publication_sequence,
                tag,
            });
        } else {
            state.pending = Some(PendingAcquire {
                request_id,
                publication_sequence,
                tag,
            });
        }
        Ok(actions)
    }

    pub(crate) fn grant(&mut self, grant: Tag) -> Vec<CoordinationAction> {
        if self.stopped {
            return Vec::new();
        }
        self.grant_coverage = Some(self.grant_coverage.map_or(grant, |old| old.max(grant)));
        let mut actions = Vec::new();
        for (&participant, state) in &mut self.participants {
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.tag <= grant)
            {
                let pending = state.pending.take().expect("checked");
                actions.push(CoordinationAction::ReleaseAcquire {
                    participant,
                    request_id: pending.request_id,
                    publication_sequence: pending.publication_sequence,
                    tag: pending.tag,
                });
            }
        }
        if let Some(tag) = self.minimum_candidate().filter(|tag| *tag <= grant) {
            self.observation_epoch = self.observation_epoch.wrapping_add(1);
            for state in self.participants.values_mut() {
                state.certificate_epoch = None;
            }
            self.round = Some((tag, self.observation_epoch));
            actions.extend(self.wake_actions(tag, self.observation_epoch));
        }
        actions
    }

    pub(crate) fn complete(
        &mut self,
        participant: EnclaveKey,
        request_id: u64,
        tag: Tag,
    ) -> Result<Vec<CoordinationAction>, String> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        let state = self
            .participants
            .get(&participant)
            .ok_or_else(|| format!("unknown participant {participant}"))?;
        if state.finished {
            return Err(format!("participant {participant} already finished"));
        }
        if state.completed.is_some_and(|completed| tag < completed) {
            return Err("completion regressed".into());
        }
        let advances = state.completed != Some(tag);
        let has_certificate = self
            .participants
            .values()
            .any(|state| state.certificate_epoch.is_some());
        let mut actions = Vec::new();
        if advances && has_certificate {
            self.observation_epoch = self.observation_epoch.wrapping_add(1);
            for state in self.participants.values_mut() {
                state.certificate_epoch = None;
            }
            if let Some((round_tag, _)) = self.round {
                self.round = Some((round_tag, self.observation_epoch));
                actions.extend(self.wake_actions(round_tag, self.observation_epoch));
            }
        }
        let state = self.participants.get_mut(&participant).expect("checked");
        if let Some(old) = state
            .pending_completion
            .replace(PendingCompletion { request_id })
        {
            actions.push(CoordinationAction::FailRequest {
                participant,
                request_id: old.request_id,
                reason: "superseded by newer completion".into(),
            });
        }
        state.completed = Some(tag);
        if let Some((round_tag, epoch)) = self.round {
            if tag >= round_tag {
                state.certificate_epoch = Some(epoch);
            }
            if self
                .participants
                .values()
                .filter(|state| !state.finished)
                .all(|state| state.certificate_epoch == Some(epoch))
            {
                actions.push(CoordinationAction::ReportLtc { tag: round_tag });
                self.round = None;
                actions.extend(self.recompute_net());
            }
        }
        actions.push(CoordinationAction::ReleaseCompletion {
            participant,
            request_id,
        });
        Ok(actions)
    }

    fn minimum_candidate(&self) -> Option<Tag> {
        self.participants
            .values()
            .filter(|state| !state.finished)
            .filter_map(|state| match state.frontier {
                Some(LogicalTimeFrontier::Candidate(tag)) => Some(tag),
                _ => None,
            })
            .min()
    }

    fn recompute_net(&mut self) -> Vec<CoordinationAction> {
        if self.round.is_some() {
            return Vec::new();
        }
        let live = self
            .participants
            .values()
            .filter(|state| !state.finished)
            .collect::<Vec<_>>();
        if live.is_empty() || live.iter().any(|state| state.frontier.is_none()) {
            return Vec::new();
        }
        let Some(tag) = self.minimum_candidate() else {
            return Vec::new();
        };
        if tag == Tag::FOREVER || self.advertised_net == Some(tag) {
            return Vec::new();
        }
        self.advertised_net = Some(tag);
        vec![CoordinationAction::RequestNet { tag }]
    }

    fn wake_actions(&self, tag: Tag, observation_epoch: u64) -> Vec<CoordinationAction> {
        self.participants
            .iter()
            .filter(|(_, state)| !state.finished)
            .map(|(&participant, _)| CoordinationAction::WakeParticipant {
                participant,
                tag,
                observation_epoch,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boomerang_runtime::{CoordinationWake, FrontierPublication, LogicalTimeFrontier, Tag};

    fn key(value: usize) -> boomerang_runtime::EnclaveKey {
        value.into()
    }
    fn candidate(tag: Tag) -> FrontierPublication {
        FrontierPublication {
            frontier: LogicalTimeFrontier::Candidate(tag),
            consumed_wake: None,
        }
    }

    fn publication(
        frontier: LogicalTimeFrontier,
        consumed_wake: Option<CoordinationWake>,
    ) -> FrontierPublication {
        FrontierPublication {
            frontier,
            consumed_wake,
        }
    }

    fn wake_for(actions: &[CoordinationAction], participant: EnclaveKey) -> CoordinationWake {
        actions
            .iter()
            .find_map(|action| match action {
                CoordinationAction::WakeParticipant {
                    participant: observed,
                    tag,
                    observation_epoch,
                } if *observed == participant => Some(CoordinationWake {
                    tag: *tag,
                    observation_epoch: *observation_epoch,
                }),
                _ => None,
            })
            .expect("participant wake")
    }

    fn has_ltc(actions: &[CoordinationAction], expected: Tag) -> bool {
        actions.iter().any(
            |action| matches!(action, CoordinationAction::ReportLtc { tag } if *tag == expected),
        )
    }

    #[test]
    fn deterministic_minimum_candidate_across_input_reordering() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(1), key(0)]),
        );
        assert!(state
            .publish(key(1), 1, candidate(Tag::FOREVER))
            .unwrap()
            .is_empty());
        assert_eq!(
            state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap(),
            vec![CoordinationAction::RequestNet { tag: Tag::ZERO }]
        );
    }

    #[test]
    fn all_idle_emits_no_finite_net_and_remains_live() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        for participant in [key(0), key(1)] {
            assert!(state
                .publish(
                    participant,
                    1,
                    FrontierPublication {
                        frontier: LogicalTimeFrontier::Idle,
                        consumed_wake: None
                    }
                )
                .unwrap()
                .is_empty());
        }
        assert!(!state.is_stopped());
    }

    #[test]
    fn newer_publication_supersedes_older_acquire() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0)]),
        );
        state.publish(key(0), 1, candidate(Tag::FOREVER)).unwrap();
        state.acquire(key(0), 7, 1, Tag::FOREVER).unwrap();
        let actions = state.publish(key(0), 2, candidate(Tag::ZERO)).unwrap();
        assert!(
            matches!(actions.as_slice(), [CoordinationAction::FailRequest { request_id: 7, .. }, CoordinationAction::RequestNet { tag }] if *tag == Tag::ZERO)
        );
    }

    #[test]
    fn exact_wake_acknowledgement_and_fixed_point_invalidation() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let actions = state.grant(Tag::ZERO);
        let epoch = actions
            .iter()
            .find_map(|action| match action {
                CoordinationAction::WakeParticipant {
                    observation_epoch, ..
                } => Some(*observation_epoch),
                _ => None,
            })
            .unwrap();
        state
            .publish(
                key(1),
                2,
                FrontierPublication {
                    frontier: LogicalTimeFrontier::Idle,
                    consumed_wake: Some(CoordinationWake {
                        tag: Tag::ZERO,
                        observation_epoch: epoch,
                    }),
                },
            )
            .unwrap();
        let invalidated = state.publish(key(0), 2, candidate(Tag::FOREVER)).unwrap();
        assert_eq!(
            invalidated
                .iter()
                .filter(|action| matches!(action, CoordinationAction::WakeParticipant { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn final_finished_emits_one_stop() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0)]),
        );
        assert_eq!(
            state
                .publish(
                    key(0),
                    1,
                    FrontierPublication {
                        frontier: LogicalTimeFrontier::Finished,
                        consumed_wake: None
                    }
                )
                .unwrap(),
            vec![CoordinationAction::SendStop]
        );
        assert!(state
            .publish(
                key(0),
                2,
                FrontierPublication {
                    frontier: LogicalTimeFrontier::Finished,
                    consumed_wake: None
                }
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    fn candidate_at_round_tag_does_not_certify_completion() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        let wake = state
            .grant(Tag::ZERO)
            .into_iter()
            .find_map(|action| match action {
                CoordinationAction::WakeParticipant {
                    tag,
                    observation_epoch,
                    ..
                } => Some(CoordinationWake {
                    tag,
                    observation_epoch,
                }),
                _ => None,
            })
            .unwrap();
        let actions = state
            .publish(
                key(0),
                2,
                FrontierPublication {
                    frontier: LogicalTimeFrontier::Candidate(Tag::ZERO),
                    consumed_wake: Some(wake),
                },
            )
            .unwrap();
        assert!(!actions
            .iter()
            .any(|action| matches!(action, CoordinationAction::ReportLtc { .. })));
    }

    #[test]
    fn lower_candidate_revises_higher_advertised_net() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(later)).unwrap();
        assert_eq!(
            state.publish(key(1), 1, candidate(later)).unwrap(),
            vec![CoordinationAction::RequestNet { tag: later }]
        );
        assert_eq!(
            state.publish(key(1), 2, candidate(Tag::ZERO)).unwrap(),
            vec![CoordinationAction::RequestNet { tag: Tag::ZERO }]
        );
    }

    #[test]
    fn stale_publication_is_rejected_and_finished_is_idempotent() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 2, candidate(Tag::ZERO)).unwrap();
        assert!(state.publish(key(0), 2, candidate(Tag::ZERO)).is_err());
        state
            .publish(key(0), 3, publication(LogicalTimeFrontier::Finished, None))
            .unwrap();
        assert!(state
            .publish(key(0), 1, publication(LogicalTimeFrontier::Finished, None),)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn higher_grant_opens_round_for_current_minimum_and_caches_coverage() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(later)).unwrap();
        state.acquire(key(0), 1, 1, Tag::ZERO).unwrap();
        let actions = state.grant(later);
        assert!(matches!(
            actions.first(),
            Some(CoordinationAction::ReleaseAcquire { tag, .. }) if *tag == Tag::ZERO
        ));
        assert_eq!(wake_for(&actions, key(0)).tag, Tag::ZERO);
        assert!(matches!(
            state.acquire(key(1), 2, 1, later).unwrap().as_slice(),
            [CoordinationAction::ReleaseAcquire { tag, .. }] if *tag == later
        ));
    }

    #[test]
    fn cached_grant_opens_round_when_frontier_moves_within_coverage() {
        let earlier = Tag::new(boomerang_runtime::Duration::ZERO, 1);
        let later = Tag::new(boomerang_runtime::Duration::seconds(1), 0);
        let participant = key(0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([participant]),
        );

        state.publish(participant, 1, candidate(later)).unwrap();
        state.acquire(participant, 1, 1, later).unwrap();
        assert!(state.grant(earlier).is_empty());

        let actions = state.publish(participant, 2, candidate(earlier)).unwrap();
        let wake = wake_for(&actions, participant);
        assert!(actions.iter().any(|action| matches!(
            action,
            CoordinationAction::FailRequest { request_id: 1, .. }
        )));

        assert!(matches!(
            state
                .acquire(participant, 2, 2, earlier)
                .unwrap()
                .as_slice(),
            [CoordinationAction::ReleaseAcquire { request_id: 2, .. }]
        ));
        state
            .publish(
                participant,
                3,
                publication(LogicalTimeFrontier::Candidate(earlier), Some(wake)),
            )
            .unwrap();

        let completion = state.complete(participant, 3, earlier).unwrap();
        assert!(has_ltc(&completion, earlier));
    }

    #[test]
    fn repeated_candidate_after_ltc_does_not_reopen_cached_round() {
        let participant = key(0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([participant]),
        );

        state.publish(participant, 1, candidate(Tag::ZERO)).unwrap();
        state.grant(Tag::ZERO);
        assert!(has_ltc(
            &state.complete(participant, 1, Tag::ZERO).unwrap(),
            Tag::ZERO
        ));

        let repeated = state.publish(participant, 2, candidate(Tag::ZERO)).unwrap();
        assert!(repeated.is_empty());
    }

    #[test]
    fn pre_round_idle_and_wrong_wake_do_not_certify_quiescence() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state
            .publish(key(0), 1, publication(LogicalTimeFrontier::Idle, None))
            .unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        let wake = wake_for(&wakes, key(1));
        assert!(!has_ltc(
            &state
                .publish(
                    key(1),
                    2,
                    publication(
                        LogicalTimeFrontier::Idle,
                        Some(CoordinationWake {
                            tag: wake.tag,
                            observation_epoch: wake.observation_epoch.wrapping_add(1),
                        }),
                    ),
                )
                .unwrap(),
            Tag::ZERO
        ));
        assert!(!has_ltc(
            &state
                .publish(
                    key(0),
                    2,
                    publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(0)))),
                )
                .unwrap(),
            Tag::ZERO
        ));
    }

    #[test]
    fn every_live_participant_must_certify_same_epoch_before_ltc() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        assert!(!has_ltc(
            &state
                .complete(key(0), 1, Tag::ZERO)
                .expect("first participant completion"),
            Tag::ZERO
        ));
        let invalidated = state
            .publish(
                key(1),
                2,
                publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(1)))),
            )
            .unwrap();
        assert!(!has_ltc(&invalidated, Tag::ZERO));
        state.complete(key(0), 1, Tag::ZERO).unwrap();
        assert!(has_ltc(
            &state
                .publish(
                    key(1),
                    3,
                    publication(
                        LogicalTimeFrontier::Idle,
                        Some(wake_for(&invalidated, key(1))),
                    ),
                )
                .unwrap(),
            Tag::ZERO
        ));
    }

    #[test]
    fn completion_after_peer_certificate_invalidates_and_rewakes_all() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        state
            .publish(
                key(1),
                2,
                publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(1)))),
            )
            .unwrap();

        let invalidated = state.complete(key(0), 1, Tag::ZERO).unwrap();

        assert_eq!(
            invalidated
                .iter()
                .filter(|action| matches!(action, CoordinationAction::WakeParticipant { .. }))
                .count(),
            2
        );
        assert!(!has_ltc(&invalidated, Tag::ZERO));
    }

    #[test]
    fn frontier_advance_after_peer_certificate_invalidates_epoch() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        state
            .publish(
                key(1),
                2,
                publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(1)))),
            )
            .unwrap();

        let invalidated = state
            .publish(key(0), 2, publication(LogicalTimeFrontier::Idle, None))
            .unwrap();

        assert_eq!(wake_for(&invalidated, key(0)).observation_epoch, 2);
        assert_eq!(wake_for(&invalidated, key(1)).observation_epoch, 2);
    }

    #[test]
    fn candidate_revision_after_peer_certificate_invalidates_epoch() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(1), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        state
            .publish(
                key(1),
                2,
                publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(1)))),
            )
            .unwrap();

        let invalidated = state.publish(key(0), 2, candidate(later)).unwrap();

        assert_eq!(
            invalidated
                .iter()
                .filter(|action| matches!(action, CoordinationAction::WakeParticipant { .. }))
                .count(),
            2
        );
        assert!(!has_ltc(&invalidated, Tag::ZERO));
    }

    #[test]
    fn zero_and_positive_delay_participants_reach_fixed_points() {
        for round in [
            Tag::ZERO,
            Tag::new(boomerang_runtime::Duration::milliseconds(5), 0),
        ] {
            let mut state = FederateCoordinationState::new(
                crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
            );
            state.publish(key(0), 1, candidate(round)).unwrap();
            state.publish(key(1), 1, candidate(round)).unwrap();
            state.grant(round);
            assert!(!has_ltc(&state.complete(key(0), 1, round).unwrap(), round));
            assert!(!has_ltc(&state.complete(key(1), 2, round).unwrap(), round));
            assert!(has_ltc(&state.complete(key(0), 3, round).unwrap(), round));
        }
    }

    #[test]
    fn finished_participant_leaves_later_frontier_and_round_calculations() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(1), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(later)).unwrap();
        let actions = state
            .publish(key(0), 2, publication(LogicalTimeFrontier::Finished, None))
            .unwrap();
        assert!(actions.iter().any(
            |action| matches!(action, CoordinationAction::RequestNet { tag } if *tag == later)
        ));
        let wakes = state.grant(later);
        assert!(wakes.iter().all(|action| !matches!(
            action,
            CoordinationAction::WakeParticipant { participant, .. } if *participant == key(0)
        )));
    }

    #[test]
    fn finished_invalidator_rewakes_only_remaining_live_participants() {
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let wakes = state.grant(Tag::ZERO);
        state
            .publish(
                key(1),
                2,
                publication(LogicalTimeFrontier::Idle, Some(wake_for(&wakes, key(1)))),
            )
            .unwrap();

        let invalidated = state
            .publish(key(0), 2, publication(LogicalTimeFrontier::Finished, None))
            .unwrap();

        assert!(invalidated.iter().any(|action| matches!(
            action,
            CoordinationAction::WakeParticipant { participant, .. } if *participant == key(1)
        )));
        assert!(invalidated.iter().all(|action| !matches!(
            action,
            CoordinationAction::WakeParticipant { participant, .. } if *participant == key(0)
        )));
    }

    #[test]
    fn superseded_acquire_ignores_late_grant_release() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(1), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0)]),
        );
        state.publish(key(0), 1, candidate(later)).unwrap();
        state.acquire(key(0), 7, 1, later).unwrap();
        state.publish(key(0), 2, candidate(Tag::ZERO)).unwrap();
        assert!(!state.grant(later).iter().any(|action| matches!(
            action,
            CoordinationAction::ReleaseAcquire { request_id: 7, .. }
        )));
    }

    #[test]
    fn next_net_waits_until_current_round_ltc() {
        let later = Tag::new(boomerang_runtime::Duration::milliseconds(10), 0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([key(0), key(1)]),
        );
        state.publish(key(0), 1, candidate(Tag::ZERO)).unwrap();
        state.publish(key(1), 1, candidate(Tag::ZERO)).unwrap();
        let first_wakes = state.grant(Tag::ZERO);
        let first_advance = state
            .publish(
                key(0),
                2,
                publication(
                    LogicalTimeFrontier::Candidate(later),
                    Some(wake_for(&first_wakes, key(0))),
                ),
            )
            .unwrap();
        let invalidated = state
            .publish(
                key(1),
                2,
                publication(
                    LogicalTimeFrontier::Candidate(later),
                    Some(wake_for(&first_wakes, key(1))),
                ),
            )
            .unwrap();
        assert!(first_advance.iter().chain(&invalidated).all(|action| {
            !matches!(action, CoordinationAction::RequestNet { tag } if *tag == later)
        }));
        let second_wake_0 = wake_for(&invalidated, key(0));
        let second_wake_1 = wake_for(&invalidated, key(1));
        let first_certificate = state
            .publish(
                key(0),
                3,
                publication(LogicalTimeFrontier::Candidate(later), Some(second_wake_0)),
            )
            .unwrap();
        assert!(first_certificate.is_empty());

        let fixed_point = state
            .publish(
                key(1),
                3,
                publication(LogicalTimeFrontier::Candidate(later), Some(second_wake_1)),
            )
            .unwrap();

        assert!(matches!(
            fixed_point.as_slice(),
            [
                CoordinationAction::ReportLtc { tag: completed },
                CoordinationAction::RequestNet { tag: requested },
            ] if *completed == Tag::ZERO && *requested == later
        ));
    }

    #[test]
    fn terminal_failure_fans_out_pending_requests_and_preserves_first_error() {
        let participant = key(0);
        let mut state = FederateCoordinationState::new(
            crate::federate_coordination::FederateCoordinationLayout::new([participant]),
        );
        state.publish(participant, 1, candidate(Tag::ZERO)).unwrap();
        state.acquire(participant, 11, 1, Tag::ZERO).unwrap();
        state.complete(participant, 12, Tag::ZERO).unwrap();

        let actions = state.fail("first failure".into());
        assert_eq!(state.terminal_error(), Some("first failure"));
        assert!(actions.iter().any(|action| matches!(
            action,
            CoordinationAction::FailRequest { request_id: 11, reason, .. }
                if reason == "first failure"
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            CoordinationAction::FailRequest { request_id: 12, reason, .. }
                if reason == "first failure"
        )));
        assert!(actions
            .iter()
            .any(|action| matches!(action, CoordinationAction::SendStop)));

        assert!(state.fail("second failure".into()).is_empty());
        assert_eq!(state.terminal_error(), Some("first failure"));
    }
}
