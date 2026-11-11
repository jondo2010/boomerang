//! Time closure at scheduler safe points.
//!
//! `progress` separates admission cutoffs from executed time. `protocol` stores
//! bounded participant evidence and mailbox frames. This module applies that
//! protocol through the existing scheduler; it does not grant execution.

pub(crate) mod progress;
pub(crate) mod protocol;

use self::{
    progress::{IngressSeal, ProgressRelease},
    protocol::*,
};
use crate::sched::core::{ExecutionStorage, Schedule, SchedulerCore, SchedulerError};
use crate::{clock::PhysicalClockError, AsyncEvent, OutputBound, OutputWaitError};

impl<S: Schedule, E: ExecutionStorage<S>> SchedulerCore<'_, '_, S, E> {
    /// Handles opaque proof frames only between complete scheduler operations.
    pub(in crate::sched) fn handle_output_proof(&mut self, frame: ProofFrame) {
        let Some(output) = self.output.as_deref_mut() else {
            frame.lease.fail(OutputWaitError::UnsupportedBackend);
            return;
        };
        let ProofFrame { lease, round, kind } = frame;
        if lease.ended() {
            return;
        }
        match kind {
            ProofKind::Reserve { cut, reply } => {
                if output
                    .proof
                    .participant
                    .as_ref()
                    .is_some_and(|p| p.lease.ended())
                {
                    output.proof.participant = None;
                }
                let route = reply.or_else(|| output.proof_endpoint().map(|(_, _, sender)| sender));
                let Some(route) = route else {
                    lease.fail(OutputWaitError::NotStarted);
                    return;
                };
                if self
                    .federate_coordination
                    .as_deref()
                    .is_some_and(|port| !port.supports_output_proofs())
                {
                    lease.fail(OutputWaitError::UnsupportedBackend);
                    let _ = route.try_send(AsyncEvent::OutputWake);
                    return;
                }
                if output
                    .proof
                    .participant
                    .as_ref()
                    .is_some_and(|p| !p.lease.matches(&lease))
                {
                    lease.fail(OutputWaitError::Capacity);
                    let _ = route.try_send(AsyncEvent::OutputWake);
                    return;
                }
                if output.proof.participant.as_ref().is_some_and(|p| {
                    p.round > round
                        || (p.round == round
                            && p.phase != ParticipationPhase::Parked
                            && p.cut > cut)
                }) {
                    return;
                }
                output.proof.outbox.prepare(&lease);
                if let Some(participant) = output
                    .proof
                    .participant
                    .as_mut()
                    .filter(|p| p.lease.matches(&lease) && p.round == round)
                {
                    participant.cut = cut;
                    participant.phase = ParticipationPhase::Draining;
                } else {
                    output.proof.participant = Some(Participation::new(
                        lease.clone(),
                        route.clone(),
                        round,
                        cut,
                        self.upstream_enclaves.len(),
                    ));
                }
                for (key, barrier) in self.upstream_enclaves.iter() {
                    let needed = OutputBound::ThroughTime(cut).upstream_cut(barrier.delay);
                    match needed {
                        Ok(Some(OutputBound::ThroughTime(time)))
                            if !barrier.completed_through(OutputBound::ThroughTime(time)) =>
                        {
                            output.proof.outbox.send(
                                &route,
                                &lease,
                                round,
                                ProofKind::Discover {
                                    key,
                                    cut: time,
                                    sender: (key != lease.root)
                                        .then(|| barrier.upstream.async_tx.clone()),
                                },
                            );
                        }
                        Ok(_) => {}
                        Err(error) => lease.fail(error),
                    }
                }
                output.proof.outbox.send(
                    &route,
                    &lease,
                    round,
                    ProofKind::Expanded { key: self.key, cut },
                );
            }
            ProofKind::Discover { key, cut, sender } => {
                if let Some(root) = output.proof.root.as_mut().filter(|root| {
                    root.lease.matches(&lease)
                        && root.round == round
                        && root.phase == ProofPhase::Discover
                }) {
                    root.include(key, cut, sender);
                }
            }
            ProofKind::Expanded { key, cut } | ProofKind::Ready { key, cut } => {
                let ready = matches!(kind, ProofKind::Ready { .. });
                if let Some(root) = output
                    .proof
                    .root
                    .as_mut()
                    .filter(|root| root.lease.matches(&lease) && root.round == round)
                {
                    if let Some(member) = root
                        .members
                        .iter_mut()
                        .find(|member| member.key == key && member.cut == Some(cut))
                    {
                        member.ack = if ready {
                            MemberAck::Ready
                        } else {
                            MemberAck::Expanded
                        };
                    }
                }
            }
            ProofKind::Frozen { key }
            | ProofKind::Checked { key, .. }
            | ProofKind::Committed { key } => {
                if let Some(root) = output
                    .proof
                    .root
                    .as_mut()
                    .filter(|root| root.lease.matches(&lease) && root.round == round)
                {
                    if let Some(member) = root
                        .members
                        .iter_mut()
                        .find(|member| member.key == key && member.cut.is_some())
                    {
                        let ack = match kind {
                            ProofKind::Frozen { .. } if root.phase == ProofPhase::Freeze => {
                                Some(MemberAck::Frozen)
                            }
                            ProofKind::Checked { clean, .. } if root.phase == ProofPhase::Check => {
                                Some(MemberAck::Checked(clean))
                            }
                            ProofKind::Committed { .. } if root.phase == ProofPhase::Commit => {
                                Some(MemberAck::Committed)
                            }
                            _ => None,
                        };
                        if let Some(ack) = ack {
                            member.ack = ack;
                        }
                    }
                }
            }
            ProofKind::NeedFrontier { key, cut } => {
                if let Some(root) = output
                    .proof
                    .root
                    .as_mut()
                    .filter(|root| root.lease.matches(&lease) && root.round == round)
                {
                    if root.members.iter().any(|member| member.key == key) {
                        root.needed = Some(
                            root.needed
                                .filter(|(_, old)| *old < cut)
                                .unwrap_or((key, cut)),
                        );
                        if root.phase == ProofPhase::Drain {
                            root.restart(&mut output.proof.outbox, Some((key, cut)), false);
                        }
                    }
                }
            }
            ProofKind::Park => {
                if let Some(p) = output
                    .proof
                    .participant
                    .as_mut()
                    .filter(|p| p.lease.matches(&lease) && p.round <= round)
                {
                    p.park(round);
                }
            }
            ProofKind::Freeze | ProofKind::Check | ProofKind::Commit => {
                let Some(p) = output
                    .proof
                    .participant
                    .as_ref()
                    .filter(|p| p.lease.matches(&lease) && p.round == round)
                else {
                    return;
                };
                let route = p.reply.clone();
                let cut = p.cut;
                let phase = p.phase;
                match kind {
                    ProofKind::Freeze
                        if matches!(
                            phase,
                            ParticipationPhase::Ready | ParticipationPhase::Draining
                        ) && output
                            .progress
                            .closing
                            .is_some_and(|seal| seal.cut >= OutputBound::ThroughTime(cut)) =>
                    {
                        output.proof.participant.as_mut().unwrap().phase =
                            ParticipationPhase::Frozen;
                        output.proof.outbox.send(
                            &route,
                            &lease,
                            round,
                            ProofKind::Frozen { key: self.key },
                        );
                    }
                    ProofKind::Check if phase == ParticipationPhase::Frozen => {
                        let clean = self.events.peek_tag().is_none_or(|tag| {
                            OutputBound::ThroughTag(tag) > OutputBound::ThroughTime(cut)
                        });
                        output.proof.participant.as_mut().unwrap().phase =
                            ParticipationPhase::Checked(clean);
                        output.proof.outbox.send(
                            &route,
                            &lease,
                            round,
                            ProofKind::Checked {
                                key: self.key,
                                clean,
                            },
                        );
                    }
                    ProofKind::Commit if phase == ParticipationPhase::Checked(true) => {
                        let bound = OutputBound::ThroughTime(cut);
                        output.progress.sealed =
                            Some(output.progress.sealed.map_or(bound, |old| old.max(bound)));
                        for (_, context) in self.downstream_enclaves.iter() {
                            output.proof.outbox.event(
                                &context.async_tx,
                                &lease,
                                AsyncEvent::OutputProgress(ProgressRelease {
                                    enclave: self.key,
                                    time: cut,
                                }),
                            );
                        }
                        output.proof.outbox.send(
                            &route,
                            &lease,
                            round,
                            ProofKind::Committed { key: self.key },
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    /// Advances the rendezvous without executing work or waiting for a peer synchronously.
    fn advance_output_root(&mut self) {
        let Some(output) = self.output.as_deref_mut() else {
            return;
        };
        let Some(mut root) = output.proof.root.take() else {
            return;
        };
        if output
            .time_candidate()
            .is_none_or(|time| time > root.target)
        {
            root.lease.fail(OutputWaitError::Canceled);
        }
        if let Some(error) = root.lease.error.get().copied() {
            output.fail_time(root.target, error);
            return;
        }
        match root.phase {
            ProofPhase::Discover => {
                let root_sender = root
                    .members
                    .iter()
                    .find(|member| member.key == root.lease.root)
                    .unwrap()
                    .sender
                    .clone();
                for member in &mut root.members {
                    if let Some(cut) = member.cut.filter(|_| member.ack == MemberAck::Unsent) {
                        output.proof.outbox.send(
                            &member.sender,
                            &root.lease,
                            root.round,
                            ProofKind::Reserve {
                                cut,
                                reply: (member.key != root.lease.root).then(|| root_sender.clone()),
                            },
                        );
                        member.ack = MemberAck::Expanding;
                    }
                }
                if root
                    .members
                    .iter()
                    .filter(|member| member.cut.is_some())
                    .all(|member| matches!(member.ack, MemberAck::Expanded | MemberAck::Ready))
                {
                    if let Some(frontier) = root.needed {
                        root.restart(&mut output.proof.outbox, Some(frontier), false);
                    } else {
                        root.phase = ProofPhase::Drain;
                        // Discovery can finish with every Ready already consumed.
                        // Retain a turn to evaluate Drain before this root parks.
                        let _ = root_sender.try_send(AsyncEvent::OutputWake);
                    }
                }
            }
            ProofPhase::Drain => {
                if root
                    .members
                    .iter()
                    .filter(|member| member.cut.is_some())
                    .all(|member| member.ack == MemberAck::Ready)
                {
                    root.phase = ProofPhase::Freeze;
                    for member in root.members.iter().filter(|member| member.cut.is_some()) {
                        root.send(&mut output.proof.outbox, &member.sender, ProofKind::Freeze);
                    }
                }
            }
            ProofPhase::Freeze => {
                if root
                    .members
                    .iter()
                    .filter(|member| member.cut.is_some())
                    .all(|member| member.ack == MemberAck::Frozen)
                {
                    root.phase = ProofPhase::Check;
                    for member in root.members.iter().filter(|member| member.cut.is_some()) {
                        root.send(&mut output.proof.outbox, &member.sender, ProofKind::Check);
                    }
                }
            }
            ProofPhase::Check => {
                if root
                    .members
                    .iter()
                    .any(|member| member.cut.is_some() && member.ack == MemberAck::Checked(false))
                {
                    if let Some(frontier) = root.needed {
                        root.restart(&mut output.proof.outbox, Some(frontier), false);
                    } else {
                        root.restart(&mut output.proof.outbox, None, false);
                    }
                } else if root
                    .members
                    .iter()
                    .filter(|member| member.cut.is_some())
                    .all(|member| member.ack == MemberAck::Checked(true))
                {
                    root.phase = ProofPhase::Commit;
                    for member in root.members.iter().filter(|member| member.cut.is_some()) {
                        root.send(&mut output.proof.outbox, &member.sender, ProofKind::Commit);
                    }
                }
            }
            ProofPhase::Commit => {
                if root
                    .members
                    .iter()
                    .filter(|member| member.cut.is_some())
                    .all(|member| member.ack == MemberAck::Committed)
                {
                    if root.purpose == RoundPurpose::Output {
                        self.complete_output_bound(OutputBound::ThroughTime(root.target));
                        return;
                    }
                    root.restart(&mut output.proof.outbox, None, true);
                }
            }
        }
        output.proof.root = Some(root);
    }

    /// Services proof safe points before ordinary candidate selection.
    /// Returns `None` only when the existing scheduler should execute real work.
    pub(in crate::sched) fn service_output_proof(
        &mut self,
    ) -> Result<Option<bool>, SchedulerError<E::Error>> {
        self.release_ended_participation();
        let Some(output) = self.output.as_deref_mut() else {
            return Ok(None);
        };
        if output.proof.root.is_none() && output.proof.participant.is_none() {
            if let Some(time) = output.time_candidate() {
                if output
                    .progress
                    .sealed
                    .is_some_and(|bound| bound >= OutputBound::ThroughTime(time))
                {
                    self.complete_output_bound(OutputBound::ThroughTime(time));
                    return Ok(Some(true));
                }
                let Some((epoch, capacity, sender)) = output.proof_endpoint() else {
                    return Ok(None);
                };
                let Some(sequence) = output.proof.sequence.checked_add(1) else {
                    output.fail_time(time, OutputWaitError::Capacity);
                    return Ok(Some(true));
                };
                output.proof.sequence = sequence;
                let root = ProofRoot::new(self.key, epoch, sequence, time, sender, capacity);
                output.proof.outbox.prepare(&root.lease);
                output.proof.root = Some(root);
            }
        }
        output.proof.outbox.flush();
        self.advance_output_root();
        let Some(output) = self.output.as_deref_mut() else {
            return Ok(None);
        };
        if output.proof.outbox.pending() {
            self.wait_output_control()?;
            return Ok(Some(true));
        }
        if output
            .proof
            .participant
            .as_ref()
            .is_some_and(|p| p.lease.ended())
        {
            self.release_ended_participation();
            return Ok(Some(true));
        }
        let Some(p) = output.proof.participant.as_ref() else {
            return Ok(output.proof.root.as_ref().map(|_| true));
        };
        let cut = p.cut;
        let round = p.round;
        let lease = p.lease.clone();
        let phase = p.phase;
        let bound = OutputBound::ThroughTime(cut);
        let horizon = bound.horizon();
        if matches!(
            phase,
            ParticipationPhase::Draining | ParticipationPhase::Ready
        ) {
            // A legacy provisional demand may have been queued before reservation.
            // Its MAX projection contains no payload and is discharged by this proof.
            while self.events.peek_tag().is_some_and(|tag| {
                tag.microstep() == usize::MAX && OutputBound::ThroughTag(tag) <= bound
            }) && self.events.peek_is_control_only()
            {
                let control = self.events.pop_next_event().unwrap();
                self.events.return_reaction_set(control.reactions);
                self.events.return_action_values(control.action_values);
            }
            let eligible = self
                .events
                .peek_tag()
                .is_some_and(|tag| OutputBound::ThroughTag(tag) <= bound);
            if eligible {
                output.proof.participant.as_mut().unwrap().phase = ParticipationPhase::Draining;
                return Ok(None);
            }
            // After earlier real work drains, acquire the full cut and serialize
            // fresh admission. A virtual candidate must not hide real work.
            // Already-admitted batches can still arrive before the marker.
            if output.progress.closing.is_none_or(|seal| seal.cut < bound) {
                if let Some(result) = self.coordinate_next_tag(horizon, false, None)? {
                    return Ok(Some(result));
                }
                if let Some(coordination) = self.federate_coordination.as_deref_mut() {
                    coordination
                        .seal_ingress(bound)
                        .map_err(SchedulerError::FederateCoordination)?;
                    return Ok(Some(true));
                }
                let Some(output) = self.output.as_deref_mut() else {
                    return Ok(None);
                };
                let generation = output
                    .progress
                    .closing
                    .map_or(Some(1), |seal| seal.generation.checked_add(1))
                    .ok_or_else(|| {
                        SchedulerError::Execution(E::Error::from(PhysicalClockError::Overflow))
                    })?;
                output.progress.install(IngressSeal {
                    generation,
                    cut: bound,
                });
            }
        }
        if phase == ParticipationPhase::Draining {
            // A weaker subcertificate still needs its own current grant and pacing.
            if let Some(result) = self.coordinate_next_tag(horizon, false, None)? {
                return Ok(Some(result));
            }
            if let Some(result) = self.synchronize_next_tag(horizon)? {
                return Ok(Some(result));
            }
            let Some(output) = self.output.as_deref_mut() else {
                return Ok(None);
            };
            let Some(p) = output.proof.participant.as_mut() else {
                return Ok(Some(true));
            };
            if p.phase != ParticipationPhase::Draining
                || p.round != round
                || p.cut != cut
                || !p.lease.matches(&lease)
            {
                return Ok(Some(true));
            }
            p.phase = ParticipationPhase::Ready;
            output.proof.outbox.send(
                &p.reply,
                &p.lease,
                p.round,
                ProofKind::Ready {
                    key: self.key,
                    cut: p.cut,
                },
            );
            return Ok(Some(true));
        }
        // A frozen or ready scheduler keeps draining its mailbox. It does not
        // publish idle or execute later events while a certificate is in flight.
        self.wait_output_control()?;
        Ok(Some(true))
    }

    /// Clears only an ended reservation, waking its root even if the failing frame
    /// could not be sent. A full root mailbox already guarantees readable work.
    fn release_ended_participation(&mut self) {
        let Some(output) = self.output.as_deref_mut() else {
            return;
        };
        if output
            .proof
            .participant
            .as_ref()
            .is_some_and(|p| p.lease.ended())
        {
            let participant = output.proof.participant.take().unwrap();
            if participant.lease.root != self.key {
                let _ = participant.reply.try_send(AsyncEvent::OutputWake);
            }
        }
    }

    /// Pumps the existing mailbox while a proof is waiting for controls.
    /// Pending outgoing frames retry every millisecond even if the peer drains without
    /// replying. This host tick is confined to explicit proofs and grants no progress.
    pub(in crate::sched) fn wait_output_control(&mut self) -> Result<(), SchedulerError<E::Error>> {
        let Some(output) = self.output.as_deref_mut() else {
            return Ok(());
        };
        let event = if output.proof.outbox.pending() {
            match self
                .event_rx
                .recv_timeout(std::time::Duration::from_millis(1))
            {
                Ok(event) => Some(event),
                Err(kanal::ReceiveErrorTimeout::Timeout) => None,
                Err(_) => {
                    output.close(OutputWaitError::Disconnected);
                    None
                }
            }
        } else {
            match self.event_rx.recv() {
                Ok(event) => Some(event),
                Err(_) => {
                    output.close(OutputWaitError::Disconnected);
                    None
                }
            }
        };
        if let Some(event) = event {
            self.handle_async_event(event)
                .map_err(SchedulerError::Execution)?;
        }
        Ok(())
    }

    /// Checks execution lifetime after effects and before publishing a tag or time proof.
    pub(in crate::sched) fn complete_output_bound(&mut self, bound: OutputBound) {
        let Some(output) = self.output.as_deref_mut() else {
            return;
        };
        if self.clock.failed() {
            output.close(OutputWaitError::ExecutionFailed);
        } else if self.event_rx.is_closed() {
            output.close(OutputWaitError::Disconnected);
        } else {
            output.complete_bound(bound);
        }
    }
}
