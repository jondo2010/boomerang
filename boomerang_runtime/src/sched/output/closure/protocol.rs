//! Bounded evidence for one scheduler-owned computational proof.
//!
//! Members come only from existing upstream barriers. The flat table retains no
//! edges or alternate topology. Frames never authorize execution or acknowledge
//! another reactor's output. Ordinary scheduling remains in SchedulerCore.
use crate::{
    clock::{ExecutionEpoch, PhysicalInstant},
    progress::ProgressBound,
    AsyncEvent, EnclaveKey, OutputWaitError,
};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

/// Shared identity and cancellation for one proof's independently running members.
/// This lease contains no sender, so a queued frame cannot retain its own mailbox.
#[derive(Debug)]
pub(crate) struct ProofLease {
    /// Immutable execution identity of the requesting owner.
    pub(crate) epoch: ExecutionEpoch,
    /// Existing runtime enclave identity; never converted to a compiled index.
    pub(crate) root: EnclaveKey,
    /// Root-local sequence, checked before construction to prevent wraparound.
    pub(crate) sequence: u64,
    /// First typed protocol failure, shared because participants can fail independently.
    pub(crate) error: OnceLock<OutputWaitError>,
    /// Fixed control scratch bound derived from the initiating endpoint capacity.
    pub(crate) controls: usize,
    /// Root teardown releases reservations even if an explicit frame cannot be queued.
    released: AtomicBool,
}
impl ProofLease {
    /// Matches the immutable execution/root/sequence identity carried by every frame.
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.epoch == other.epoch && self.root == other.root && self.sequence == other.sequence
    }

    /// Reads cancellation or failure without inferring success from liveness.
    pub(crate) fn ended(&self) -> bool {
        self.released.load(Ordering::Acquire) || self.error.get().is_some()
    }
    /// Records the first failure. The caller also wakes the root's ordinary mailbox.
    pub(crate) fn fail(&self, error: OutputWaitError) {
        let _ = self.error.set(error);
    }
}

/// Opaque control value carried by the existing scheduler mailbox.
/// Only runtime code can construct it; external senders cannot forge a certificate.
#[derive(Debug)]
pub struct ProofFrame {
    /// Sender-free shared proof identity and abort state.
    pub(crate) lease: Arc<ProofLease>,
    /// Fresh round generation; a stale acknowledgment cannot close a later round.
    pub(crate) round: u64,
    /// Typed operation whose construction remains private to the runtime.
    pub(crate) kind: ProofKind,
}

/// Small inline controls. Only explicit discovery clones an existing peer route.
#[derive(Debug)]
pub(crate) enum ProofKind {
    /// Reserve and expand one cut. Self-directed commands omit their reply sender.
    Reserve {
        /// Inclusive time cut to reserve and enumerate.
        cut: PhysicalInstant,
        /// Root route, omitted only for a self-directed reservation.
        reply: Option<crate::Sender<AsyncEvent>>,
    },
    /// One predecessor from the authoritative local barrier table.
    Discover {
        /// Upstream scheduler identity from the member's existing barrier table.
        key: EnclaveKey,
        /// Required predecessor cut after checked route-delay inversion.
        cut: PhysicalInstant,
        /// Existing upstream mailbox, omitted when it is the root itself.
        sender: Option<crate::Sender<AsyncEvent>>,
    },
    /// The member has enumerated every predecessor of this exact cut.
    Expanded {
        /// Scheduler whose computational cut this message concerns.
        key: EnclaveKey,
        /// Exact time cut, used to reject acknowledgments for weaker expansions.
        cut: PhysicalInstant,
    },
    /// Admission, grants, pacing and local draining currently cover this cut.
    Ready {
        /// Scheduler whose computational cut this message concerns.
        key: EnclaveKey,
        /// Exact time cut, used to reject acknowledgments for weaker expansions.
        cut: PhysicalInstant,
    },
    /// Pause between rounds while preserving the reservation and durable cutoff.
    Park,
    /// Stop payload-producing work at the next scheduler safe point.
    Freeze,
    /// No callback or route send is still executing on this member.
    Frozen {
        /// Scheduler that reached the frozen safe point.
        key: EnclaveKey,
    },
    /// Observe the mailbox after every member acknowledged Frozen.
    Check,
    /// Report actual queued eligibility while remaining frozen.
    Checked {
        /// Scheduler whose ordinary mailbox consumed the fresh Check marker.
        key: EnclaveKey,
        /// Whether all actual executable contributions through its cut are absent.
        clean: bool,
    },
    /// Install final computational progress after every fresh check is clean.
    Commit,
    /// The member installed its final frontier and published local barrier releases.
    Committed {
        /// Scheduler whose durable final frontier is now installed.
        key: EnclaveKey,
    },
    /// An ordinary event needs a smaller final time certificate to cross a barrier.
    NeedFrontier {
        /// Scheduler whose computational cut this message concerns.
        key: EnclaveKey,
        /// Exact time cut, used to reject acknowledgments for weaker expansions.
        cut: PhysicalInstant,
    },
}

/// One member's safe-point state, independent of its real event queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticipationPhase {
    /// Permits ordinary work while ingress and grant checks remain incomplete.
    Draining,
    /// The cut is admitted and paced, with no eligible work at the last safe point.
    Ready,
    /// Payload production is stopped; a fresh mailbox Check is still required.
    Frozen,
    /// The current round has inspected its queue while every producer was frozen.
    Checked(
        /// Whether the checked mailbox contains no executable work through the cut.
        bool,
    ),
    /// Between rounds, no payload work runs until the new cut is reserved.
    Parked,
}

/// A reservation on the scheduler that receives proof commands.
#[derive(Debug)]
pub(crate) struct Participation {
    /// Sender-free lease shared with the root and other participants.
    pub(crate) lease: Arc<ProofLease>,
    /// Root route retained only by active scheduler state, never by the shared lease.
    pub(crate) reply: crate::Sender<AsyncEvent>,
    /// Current command generation.
    pub(crate) round: u64,
    /// Current computational cut; stronger installed ingress cuts remain durable.
    pub(crate) cut: PhysicalInstant,
    /// Whether this phase permits ordinary payload work.
    pub(crate) phase: ParticipationPhase,
    /// Outstanding demands for this lease and round, bounded by upstream edge count.
    frontier_demands: Vec<FrontierDemand>,
}

/// One demand uses the proof identity and round of its owning participation.
#[derive(Debug)]
struct FrontierDemand {
    /// Upstream Enclave whose computational progress is required.
    upstream: EnclaveKey,
    /// Inclusive physical time required from that Enclave.
    through: PhysicalInstant,
}

impl Participation {
    /// Reserves demand storage for the existing upstream dependency edges.
    pub(crate) fn new(
        lease: Arc<ProofLease>,
        reply: crate::Sender<AsyncEvent>,
        round: u64,
        cut: PhysicalInstant,
        upstream_count: usize,
    ) -> Self {
        Self {
            lease,
            reply,
            round,
            cut,
            phase: ParticipationPhase::Draining,
            frontier_demands: Vec::with_capacity(upstream_count),
        }
    }

    /// Suppresses an unchanged demand. A different cut can require a predecessor proof.
    /// The existing outbox owns retries.
    pub(crate) fn request_frontier(
        &mut self,
        upstream: EnclaveKey,
        through: PhysicalInstant,
        outbox: &mut ProofOutbox,
    ) {
        if let Some(demand) = self
            .frontier_demands
            .iter_mut()
            .find(|d| d.upstream == upstream)
        {
            if demand.through == through {
                return;
            }
            demand.through = through;
        } else if self.frontier_demands.len() < self.frontier_demands.capacity() {
            self.frontier_demands
                .push(FrontierDemand { upstream, through });
        } else {
            self.lease.fail(OutputWaitError::Capacity);
            return;
        }
        outbox.send(
            &self.reply,
            &self.lease,
            self.round,
            ProofKind::NeedFrontier {
                key: upstream,
                cut: through,
            },
        );
    }

    /// Partial progress does not retire a stronger outstanding demand.
    pub(crate) fn observe_progress(&mut self, upstream: EnclaveKey, progress: ProgressBound) {
        self.frontier_demands.retain(|demand| {
            demand.upstream != upstream || progress < ProgressBound::ThroughTime(demand.through)
        });
    }

    /// Pauses participation and clears demands when a new round starts.
    pub(crate) fn park(&mut self, round: u64) {
        if round > self.round {
            self.frontier_demands.clear();
        }
        self.round = round;
        self.phase = ParticipationPhase::Parked;
    }
}

/// Acknowledgment state for one member in one round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemberAck {
    /// The strongest current cut has not yet been sent to this member.
    Unsent,
    /// Reserve was sent; all authoritative predecessors have not yet been received.
    Expanding,
    /// Every predecessor of this cut was enumerated.
    Expanded,
    /// The member reached a locally empty, admitted and paced safe point.
    Ready,
    /// Payload production has stopped on this member.
    Frozen,
    /// A post-freeze mailbox marker observed whether the current cut was clean.
    Checked(
        /// Whether the checked mailbox contains no executable work through the cut.
        bool,
    ),
    /// The member installed final progress and enqueued its dependency releases.
    Committed,
}

/// One bounded evidence row. Keys are compared directly, without sparse-index growth.
#[derive(Debug)]
pub(crate) struct ProofMember {
    /// Actual runtime enclave identity.
    pub(crate) key: EnclaveKey,
    /// Existing scheduler mailbox, copied from its authoritative barrier edge.
    pub(crate) sender: crate::Sender<AsyncEvent>,
    /// Strongest ancestor cut for the host's requested proof.
    pub(crate) goal: Option<PhysicalInstant>,
    /// Current subcertificate cut, absent for members outside this round.
    pub(crate) cut: Option<PhysicalInstant>,
    /// Current expansion or safe-point acknowledgment.
    pub(crate) ack: MemberAck,
}

/// Protocol phase of the requesting scheduler's proof rendezvous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofPhase {
    /// Every strongest cut must enumerate its authoritative upstream barriers.
    Discover,
    /// Every included member must establish admission, pacing and local readiness.
    Drain,
    /// Every included member must stop payload production at a scheduler safe point.
    Freeze,
    /// Check markers are sent only after every Frozen acknowledgment was consumed.
    Check,
    /// Final frontiers are installed only after every fresh check was clean.
    Commit,
}

/// Distinguishes the requested proof from a smaller barrier predecessor proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoundPurpose {
    /// Certifies the host-requested cut, then checks only that owner's outcomes.
    Output,
    /// Certifies a smaller ordinary barrier predecessor before retrying the goal.
    Frontier,
}

/// Root-owned bounded proof evidence. Drop releases all known reservations.
#[derive(Debug)]
pub(crate) struct ProofRoot {
    /// Shared identity has no back-reference to this root or its mailbox.
    pub(crate) lease: Arc<ProofLease>,
    /// Original requested physical-time closure.
    pub(crate) target: PhysicalInstant,
    /// Flat, capacity-reserved evidence table; no numeric key indexing.
    pub(crate) members: Vec<ProofMember>,
    /// Maximum actual table allocation, inherited from the endpoint mailbox bound.
    pub(crate) capacity: usize,
    /// Monotonic round generation for stale-frame rejection.
    pub(crate) round: u64,
    /// Current protocol phase.
    pub(crate) phase: ProofPhase,
    /// Whether this round can complete the original host request.
    pub(crate) purpose: RoundPurpose,
    /// A smaller virtual predecessor requested while discovery was still in flight.
    pub(crate) needed: Option<(EnclaveKey, PhysicalInstant)>,
}
impl ProofRoot {
    /// Allocates bounded scratch only when the host starts an explicit proof.
    pub(crate) fn new(
        key: EnclaveKey,
        epoch: ExecutionEpoch,
        sequence: u64,
        target: PhysicalInstant,
        sender: crate::Sender<AsyncEvent>,
        capacity: usize,
    ) -> Self {
        let lease = Arc::new(ProofLease {
            epoch,
            root: key,
            sequence,
            error: OnceLock::new(),
            controls: capacity.saturating_mul(4).max(4),
            released: AtomicBool::new(false),
        });
        let mut members = Vec::with_capacity(capacity);
        members.push(ProofMember {
            key,
            sender,
            goal: Some(target),
            cut: Some(target),
            ack: MemberAck::Unsent,
        });
        Self {
            lease,
            target,
            members,
            capacity,
            round: 1,
            phase: ProofPhase::Discover,
            purpose: RoundPurpose::Output,
            needed: None,
        }
    }

    /// Adds or strengthens one discovered cut without allocating beyond the fixed bound.
    pub(crate) fn include(
        &mut self,
        key: EnclaveKey,
        cut: PhysicalInstant,
        sender: Option<crate::Sender<AsyncEvent>>,
    ) {
        if let Some(member) = self.members.iter_mut().find(|member| member.key == key) {
            if member.cut.is_none_or(|old| old < cut) {
                member.cut = Some(cut);
                member.ack = MemberAck::Unsent;
            }
            if self.purpose == RoundPurpose::Output {
                member.goal = Some(member.goal.map_or(cut, |old| old.max(cut)));
            }
        } else if self.members.len() < self.capacity {
            if let Some(sender) = sender {
                self.members.push(ProofMember {
                    key,
                    sender,
                    goal: (self.purpose == RoundPurpose::Output).then_some(cut),
                    cut: Some(cut),
                    ack: MemberAck::Unsent,
                });
            } else {
                self.lease.fail(OutputWaitError::InvalidTarget);
            }
        } else {
            self.lease.fail(OutputWaitError::Capacity);
        }
    }

    /// Enqueues one inline frame through the scheduler-owned bounded retry storage.
    pub(crate) fn send(
        &self,
        outbox: &mut ProofOutbox,
        sender: &crate::Sender<AsyncEvent>,
        kind: ProofKind,
    ) {
        outbox.send(sender, &self.lease, self.round, kind);
    }
}
impl Drop for ProofRoot {
    fn drop(&mut self) {
        self.lease.released.store(true, Ordering::Release);
        for member in &self.members {
            let _ = member.sender.try_send(AsyncEvent::OutputWake);
        }
    }
}

/// One retained control, owned only by the active scheduler's bounded scratch.
#[derive(Debug)]
struct PendingControl {
    /// Existing destination mailbox; never stored in the shared cancellation lease.
    sender: crate::Sender<AsyncEvent>,
    /// Identity used to discard abandoned frames before retrying them.
    lease: Arc<ProofLease>,
    /// The original inline event, retained on transient mailbox saturation.
    event: Option<AsyncEvent>,
}

/// FIFO retry storage allocated only on an explicit proof request or reservation.
///
/// Four controls per allowed member cover discovery, phase fanout and acknowledgments.
/// The actual retained allocation is bounded; unusually broad discovery fails Capacity.
/// A scheduler with pending frames must pump its inbox and retry on a bounded host tick.
#[derive(Debug, Default)]
pub(crate) struct ProofOutbox {
    /// Frames preserve enqueue order, including progress before its commit acknowledgment.
    pending: VecDeque<PendingControl>,
    /// Maximum retained frame count for the current reservation.
    limit: usize,
}
impl ProofOutbox {
    /// Prepares scratch when a new request/reservation is accepted, never on an outcome.
    pub(crate) fn prepare(&mut self, lease: &ProofLease) {
        self.pending.retain(|frame| !frame.lease.ended());
        if self.pending.is_empty() && self.limit != lease.controls {
            self.pending = VecDeque::with_capacity(lease.controls);
            self.limit = lease.controls;
        }
    }

    /// Retains a typed proof frame without blocking a peer scheduler.
    pub(crate) fn send(
        &mut self,
        sender: &crate::Sender<AsyncEvent>,
        lease: &Arc<ProofLease>,
        round: u64,
        kind: ProofKind,
    ) {
        self.event(
            sender,
            lease,
            AsyncEvent::OutputProof(ProofFrame {
                lease: lease.clone(),
                round,
                kind,
            }),
        );
    }

    /// Queues a progress/control event, keeping all earlier controls ahead of it.
    pub(crate) fn event(
        &mut self,
        sender: &crate::Sender<AsyncEvent>,
        lease: &Arc<ProofLease>,
        event: AsyncEvent,
    ) {
        if lease.ended() {
            return;
        }
        let mut pending = PendingControl {
            sender: sender.clone(),
            lease: lease.clone(),
            event: Some(event),
        };
        if self.pending.is_empty() {
            match pending.sender.try_send_option(&mut pending.event) {
                Ok(true) => return,
                Ok(false) => {}
                Err(_) => {
                    lease.fail(OutputWaitError::Disconnected);
                    return;
                }
            }
        }
        if self.pending.len() == self.limit {
            lease.fail(OutputWaitError::Capacity);
        } else {
            self.pending.push_back(pending);
        }
    }

    /// Retries FIFO frames until empty or the first full mailbox, without blocking.
    pub(crate) fn flush(&mut self) {
        while let Some(frame) = self.pending.front_mut() {
            if !frame.lease.ended() {
                match frame.sender.try_send_option(&mut frame.event) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => frame.lease.fail(OutputWaitError::Disconnected),
                }
            }
            self.pending.pop_front();
        }
    }

    /// Whether a bounded retry tick is needed even with an empty local mailbox.
    pub(crate) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Proof state owned by one scheduler; ordinary outcome recording never accesses it.
#[derive(Debug, Default)]
pub(crate) struct ProofState {
    /// Request sequence allocated only by this scheduler.
    pub(crate) sequence: u64,
    /// Active root evidence, if this scheduler initiated the proof.
    pub(crate) root: Option<ProofRoot>,
    /// Bounded pending controls are owned here, never by mailbox frames or the lease.
    pub(crate) outbox: ProofOutbox,
    /// At most one reservation; conflicting roots fail rather than wait holding peers.
    pub(crate) participant: Option<Participation>,
}
impl ProofState {
    /// Terminates proof participation and wakes roots before scheduler-owned state drops.
    pub(crate) fn abort(&mut self, error: OutputWaitError) {
        if let Some(participant) = self.participant.take() {
            participant.lease.fail(error);
            let _ = participant.reply.try_send(AsyncEvent::OutputWake);
        }
        if let Some(root) = self.root.take() {
            root.lease.fail(error);
        }
    }
}

impl ProofRoot {
    /// Starts a fresh expansion round, retaining stronger durable ingress floors.
    pub(crate) fn restart(
        &mut self,
        outbox: &mut ProofOutbox,
        frontier: Option<(EnclaveKey, PhysicalInstant)>,
        restore_goals: bool,
    ) {
        let Some(round) = self.round.checked_add(1) else {
            self.lease.fail(OutputWaitError::Capacity);
            return;
        };
        self.round = round;
        self.phase = ProofPhase::Discover;
        self.needed = None;
        if frontier.is_some() {
            self.purpose = RoundPurpose::Frontier;
        } else if restore_goals {
            self.purpose = RoundPurpose::Output;
        }
        for member in &mut self.members {
            if frontier.is_some() {
                member.cut = frontier
                    .filter(|(key, _)| *key == member.key)
                    .map(|(_, cut)| cut);
            } else if restore_goals {
                member.cut = member.goal;
            }
            member.ack = MemberAck::Unsent;
        }
        for member in &self.members {
            self.send(outbox, &member.sender, ProofKind::Park);
        }
    }
}

#[cfg(test)]
mod tests;
