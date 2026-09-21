//! Bounded, snapshot-oriented scheduler observations.
//!
//! This module deliberately has no transport, serialization, or subscriber
//! dependency. A scheduler writes an [`ObservationState`] while hosted code
//! samples it through [`ObservationState::snapshot`] without participating in
//! scheduler execution.

use std::{
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::Instant,
};

const SNAPSHOT_ATTEMPTS: usize = 32;

/// The scheduler activity currently being observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SchedulerPhase {
    /// The scheduler has no active operation to attribute.
    Idle = 0,
    /// User reaction callbacks are executing.
    Reaction = 1,
    /// Scheduler-managed work other than user callbacks is executing.
    Framework = 2,
    /// The scheduler is waiting for physical time.
    PhysicalWait = 3,
    /// The scheduler is waiting for an external asynchronous event.
    ExternalWait = 4,
    /// The scheduler is waiting for Federate coordination.
    CoordinationWait = 5,
}

impl SchedulerPhase {
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Reaction,
            2 => Self::Framework,
            3 => Self::PhysicalWait,
            4 => Self::ExternalWait,
            5 => Self::CoordinationWait,
            _ => Self::Idle,
        }
    }
}

/// The scheduler lifecycle as observed independently of telemetry transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SchedulerLifecycle {
    /// Scheduler startup has not begun.
    NotStarted = 0,
    /// Scheduler startup completed and execution may still be progressing logically.
    Running = 1,
    /// The scheduler completed normal shutdown finalization.
    Stopped = 2,
    /// The scheduler terminated through a runtime error path.
    Failed = 3,
}

impl SchedulerLifecycle {
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Running,
            2 => Self::Stopped,
            3 => Self::Failed,
            _ => Self::NotStarted,
        }
    }
}

/// A coherent point-in-time view of one scheduler's observations.
///
/// Elapsed times and cumulative work counters saturate at [`u64::MAX`]. Queue
/// gauges describe the instant sampled, while peak occupancy covers the period
/// for which queue observation has been enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationSnapshot {
    /// Lifecycle state reported by the scheduler, not packet delivery state.
    pub lifecycle: SchedulerLifecycle,
    /// Current scheduler phase, including a phase still in progress.
    pub current_phase: SchedulerPhase,
    /// Monotonic nanoseconds after the observation origin at which the current phase began.
    pub current_phase_started_ns: u64,
    /// Elapsed nanoseconds attributed to reaction callbacks.
    pub reaction_elapsed_ns: u64,
    /// Elapsed nanoseconds attributed to framework work.
    pub framework_elapsed_ns: u64,
    /// Elapsed nanoseconds spent waiting for physical time.
    pub physical_wait_elapsed_ns: u64,
    /// Elapsed nanoseconds spent waiting for external asynchronous input.
    pub external_wait_elapsed_ns: u64,
    /// Elapsed nanoseconds spent waiting for Federate coordination.
    pub coordination_wait_elapsed_ns: u64,
    /// Cumulative scheduler tag-processing steps.
    pub processed_tags: u64,
    /// Cumulative enabled reaction callbacks selected for invocation.
    pub processed_reactions: u64,
    /// Cumulative asynchronous scheduler events handled.
    pub processed_events: u64,
    /// Cumulative present-port observations.
    pub set_ports: u64,
    /// Cumulative actions requested by reaction outcomes.
    pub scheduled_actions: u64,
    /// Events currently retained across the scheduler's root and modal queues.
    pub event_queue_occupancy: u64,
    /// Slots currently reserved by those growable event queues, not a size limit.
    pub event_queue_reserved_capacity: u64,
    /// Runtime-enforced queue limit, or `None` for the current growable queues.
    pub event_queue_enforced_limit: Option<u64>,
    /// Largest aggregate event-queue occupancy seen while observation was enabled.
    pub event_queue_peak_occupancy: u64,
    /// Number of scheduler tags completed since observation began.
    pub completed_logical_tags: u64,
    /// Monotonic nanoseconds after the observation origin at which a tag last completed.
    pub last_logical_progress_ns: Option<u64>,
}

/// Bounded atomic observation storage for one scheduler writer and concurrent samplers.
///
/// The state owns no transport or publication queue. Snapshot attempts use a
/// fixed retry budget, so sampling cannot block scheduler execution.
#[derive(Debug)]
pub struct ObservationState {
    origin: Instant,
    /// An odd value means the single scheduler writer is changing snapshot fields.
    generation: AtomicU64,
    lifecycle: AtomicU8,
    phase: AtomicU8,
    phase_started_ns: AtomicU64,
    reaction_elapsed_ns: AtomicU64,
    framework_elapsed_ns: AtomicU64,
    physical_wait_elapsed_ns: AtomicU64,
    external_wait_elapsed_ns: AtomicU64,
    coordination_wait_elapsed_ns: AtomicU64,
    processed_tags: AtomicU64,
    processed_reactions: AtomicU64,
    processed_events: AtomicU64,
    set_ports: AtomicU64,
    scheduled_actions: AtomicU64,
    event_queue_occupancy: AtomicU64,
    event_queue_reserved_capacity: AtomicU64,
    event_queue_peak_occupancy: AtomicU64,
    completed_logical_tags: AtomicU64,
    last_logical_progress_ns: AtomicU64,
}

/// Shared ownership of one scheduler-local observation source.
///
/// The handle may be cloned for samplers, but must be attached to exactly one
/// scheduler writer for its lifetime because gauges and lifecycle are source-local.
pub type ObservationHandle = Arc<ObservationState>;

impl ObservationState {
    /// Creates observation state whose monotonic timestamps are measured from `origin`.
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            generation: AtomicU64::new(0),
            lifecycle: AtomicU8::new(SchedulerLifecycle::NotStarted as u8),
            phase: AtomicU8::new(SchedulerPhase::Idle as u8),
            phase_started_ns: AtomicU64::new(0),
            reaction_elapsed_ns: AtomicU64::new(0),
            framework_elapsed_ns: AtomicU64::new(0),
            physical_wait_elapsed_ns: AtomicU64::new(0),
            external_wait_elapsed_ns: AtomicU64::new(0),
            coordination_wait_elapsed_ns: AtomicU64::new(0),
            processed_tags: AtomicU64::new(0),
            processed_reactions: AtomicU64::new(0),
            processed_events: AtomicU64::new(0),
            set_ports: AtomicU64::new(0),
            scheduled_actions: AtomicU64::new(0),
            event_queue_occupancy: AtomicU64::new(0),
            event_queue_reserved_capacity: AtomicU64::new(0),
            event_queue_peak_occupancy: AtomicU64::new(0),
            completed_logical_tags: AtomicU64::new(0),
            last_logical_progress_ns: AtomicU64::new(u64::MAX),
        }
    }

    /// Transitions the scheduler to `phase` at `now`, closing the previous phase once.
    pub fn enter(&self, phase: SchedulerPhase, now: Instant) {
        let state = self;
        state.begin_update();
        let now_ns = elapsed_ns(state.origin, now);
        let previous = SchedulerPhase::from_u8(state.phase.load(Ordering::SeqCst));
        let started_ns = state.phase_started_ns.load(Ordering::SeqCst);
        add_elapsed(state, previous, now_ns.saturating_sub(started_ns));
        state.phase.store(phase as u8, Ordering::SeqCst);
        state.phase_started_ns.store(now_ns, Ordering::SeqCst);
        state.end_update();
    }

    /// Marks completion of successful scheduler startup.
    pub fn mark_running(&self) {
        self.begin_update();
        self.lifecycle
            .store(SchedulerLifecycle::Running as u8, Ordering::SeqCst);
        self.end_update();
    }

    /// Marks completion of normal scheduler shutdown finalization.
    pub fn mark_stopped(&self) {
        self.begin_update();
        self.lifecycle
            .store(SchedulerLifecycle::Stopped as u8, Ordering::SeqCst);
        self.end_update();
    }

    /// Marks scheduler termination through a runtime error path.
    pub fn mark_failed(&self) {
        self.begin_update();
        self.lifecycle
            .store(SchedulerLifecycle::Failed as u8, Ordering::SeqCst);
        self.end_update();
    }

    /// Records one completed scheduler tag and its corresponding logical progress.
    pub fn record_completed_tag(&self, now: Instant) {
        self.begin_update();
        saturating_add(&self.processed_tags, 1);
        saturating_add(&self.completed_logical_tags, 1);
        self.last_logical_progress_ns
            .store(elapsed_ns(self.origin, now), Ordering::SeqCst);
        self.end_update();
    }

    /// Records one completed scheduler tag.
    pub fn increment_processed_tags(&self) {
        self.begin_update();
        saturating_add(&self.processed_tags, 1);
        self.end_update();
    }

    /// Adds enabled reaction callbacks to the cumulative work counter.
    pub fn add_processed_reactions(&self, count: u64) {
        self.begin_update();
        saturating_add(&self.processed_reactions, count);
        self.end_update();
    }

    /// Records one handled asynchronous scheduler event.
    pub fn increment_processed_events(&self) {
        self.begin_update();
        saturating_add(&self.processed_events, 1);
        self.end_update();
    }

    /// Records one present-port observation.
    pub fn increment_set_ports(&self) {
        self.begin_update();
        saturating_add(&self.set_ports, 1);
        self.end_update();
    }

    /// Adds requested actions to the cumulative work counter.
    pub fn add_scheduled_actions(&self, count: u64) {
        self.begin_update();
        saturating_add(&self.scheduled_actions, count);
        self.end_update();
    }

    /// Records the aggregate state of the scheduler's growable event queues.
    pub fn record_event_queue(&self, occupancy: u64, reserved_capacity: u64, peak_occupancy: u64) {
        self.begin_update();
        self.event_queue_occupancy
            .store(occupancy, Ordering::SeqCst);
        self.event_queue_reserved_capacity
            .store(reserved_capacity, Ordering::SeqCst);
        self.event_queue_peak_occupancy
            .fetch_max(peak_occupancy.max(occupancy), Ordering::SeqCst);
        self.end_update();
    }

    /// Captures closed phase totals plus the elapsed portion of the current phase.
    ///
    /// Returns `None` when scheduler updates prevent a coherent sample within
    /// the fixed retry budget. Callers may skip that sample and retry later.
    pub fn snapshot(&self, now: Instant) -> Option<ObservationSnapshot> {
        let state = self;
        for _ in 0..SNAPSHOT_ATTEMPTS {
            // Every access participating in the snapshot protocol is sequentially
            // consistent. This places both generation checks and every field access
            // in one total order, so an accepted even generation cannot contain
            // field values moved across either validation boundary on weak memory.
            let before = state.generation.load(Ordering::SeqCst);
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            let phase = SchedulerPhase::from_u8(state.phase.load(Ordering::SeqCst));
            let started_ns = state.phase_started_ns.load(Ordering::SeqCst);
            let mut snapshot = ObservationSnapshot {
                lifecycle: SchedulerLifecycle::from_u8(state.lifecycle.load(Ordering::SeqCst)),
                current_phase: phase,
                current_phase_started_ns: started_ns,
                reaction_elapsed_ns: state.reaction_elapsed_ns.load(Ordering::SeqCst),
                framework_elapsed_ns: state.framework_elapsed_ns.load(Ordering::SeqCst),
                physical_wait_elapsed_ns: state.physical_wait_elapsed_ns.load(Ordering::SeqCst),
                external_wait_elapsed_ns: state.external_wait_elapsed_ns.load(Ordering::SeqCst),
                coordination_wait_elapsed_ns: state
                    .coordination_wait_elapsed_ns
                    .load(Ordering::SeqCst),
                processed_tags: state.processed_tags.load(Ordering::SeqCst),
                processed_reactions: state.processed_reactions.load(Ordering::SeqCst),
                processed_events: state.processed_events.load(Ordering::SeqCst),
                set_ports: state.set_ports.load(Ordering::SeqCst),
                scheduled_actions: state.scheduled_actions.load(Ordering::SeqCst),
                event_queue_occupancy: state.event_queue_occupancy.load(Ordering::SeqCst),
                event_queue_reserved_capacity: state
                    .event_queue_reserved_capacity
                    .load(Ordering::SeqCst),
                event_queue_enforced_limit: None,
                event_queue_peak_occupancy: state.event_queue_peak_occupancy.load(Ordering::SeqCst),
                completed_logical_tags: state.completed_logical_tags.load(Ordering::SeqCst),
                last_logical_progress_ns: match state
                    .last_logical_progress_ns
                    .load(Ordering::SeqCst)
                {
                    u64::MAX => None,
                    progress => Some(progress),
                },
            };
            add_snapshot_elapsed(
                &mut snapshot,
                phase,
                elapsed_ns(state.origin, now).saturating_sub(started_ns),
            );
            if state.generation.load(Ordering::SeqCst) == before {
                return Some(snapshot);
            }
        }
        None
    }

    fn begin_update(&self) {
        loop {
            let generation = self.generation.load(Ordering::SeqCst);
            if !generation.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            if self
                .generation
                .compare_exchange_weak(
                    generation,
                    generation.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    fn end_update(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.saturating_add(value))
        })
        .expect("saturating observation update always returns a value");
}

fn elapsed_ns(origin: Instant, now: Instant) -> u64 {
    now.checked_duration_since(origin)
        .map(|duration| duration.as_nanos().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn add_elapsed(state: &ObservationState, phase: SchedulerPhase, elapsed_ns: u64) {
    let total = match phase {
        SchedulerPhase::Idle => return,
        SchedulerPhase::Reaction => &state.reaction_elapsed_ns,
        SchedulerPhase::Framework => &state.framework_elapsed_ns,
        SchedulerPhase::PhysicalWait => &state.physical_wait_elapsed_ns,
        SchedulerPhase::ExternalWait => &state.external_wait_elapsed_ns,
        SchedulerPhase::CoordinationWait => &state.coordination_wait_elapsed_ns,
    };
    saturating_add(total, elapsed_ns);
}

fn add_snapshot_elapsed(
    snapshot: &mut ObservationSnapshot,
    phase: SchedulerPhase,
    elapsed_ns: u64,
) {
    let total = match phase {
        SchedulerPhase::Idle => return,
        SchedulerPhase::Reaction => &mut snapshot.reaction_elapsed_ns,
        SchedulerPhase::Framework => &mut snapshot.framework_elapsed_ns,
        SchedulerPhase::PhysicalWait => &mut snapshot.physical_wait_elapsed_ns,
        SchedulerPhase::ExternalWait => &mut snapshot.external_wait_elapsed_ns,
        SchedulerPhase::CoordinationWait => &mut snapshot.coordination_wait_elapsed_ns,
    };
    *total = total.saturating_add(elapsed_ns);
}
