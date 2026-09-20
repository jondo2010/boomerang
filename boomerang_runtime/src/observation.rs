//! Bounded, snapshot-oriented scheduler observations.
//!
//! This module deliberately has no transport, serialization, or subscriber
//! dependency. Hosted code can sample [`Observation`] without participating in
//! scheduler execution.

use std::{
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::Instant,
};

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

/// A coherent point-in-time view of scheduler phase accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationSnapshot {
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
}

/// Atomic scheduler-observation storage independent of any sharing strategy.
#[derive(Debug)]
pub struct ObservationState {
    origin: Instant,
    /// An odd value means the single scheduler writer is changing phase fields.
    generation: AtomicU64,
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
}

/// Hosted convenience ownership for independently running scheduler and exporter code.
pub type ObservationHandle = Arc<ObservationState>;

impl ObservationState {
    /// Starts observing one scheduler from `origin`.
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            generation: AtomicU64::new(0),
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
        }
    }

    /// Transitions the scheduler to `phase` at `now`, closing the previous phase once.
    pub fn enter(&self, phase: SchedulerPhase, now: Instant) {
        let state = self;
        state.generation.fetch_add(1, Ordering::AcqRel);
        let now_ns = elapsed_ns(state.origin, now);
        let previous = SchedulerPhase::from_u8(state.phase.load(Ordering::Relaxed));
        let started_ns = state.phase_started_ns.load(Ordering::Relaxed);
        add_elapsed(state, previous, now_ns.saturating_sub(started_ns));
        state.phase.store(phase as u8, Ordering::Relaxed);
        state.phase_started_ns.store(now_ns, Ordering::Relaxed);
        state.generation.fetch_add(1, Ordering::Release);
    }

    /// Records one completed scheduler tag.
    pub fn increment_processed_tags(&self) {
        saturating_add(&self.processed_tags, 1);
    }

    /// Adds enabled reaction callbacks to the cumulative work counter.
    pub fn add_processed_reactions(&self, count: u64) {
        saturating_add(&self.processed_reactions, count);
    }

    /// Records one handled asynchronous scheduler event.
    pub fn increment_processed_events(&self) {
        saturating_add(&self.processed_events, 1);
    }

    /// Records one present-port observation.
    pub fn increment_set_ports(&self) {
        saturating_add(&self.set_ports, 1);
    }

    /// Adds requested actions to the cumulative work counter.
    pub fn add_scheduled_actions(&self, count: u64) {
        saturating_add(&self.scheduled_actions, count);
    }

    /// Captures closed phase totals plus the elapsed portion of the current phase.
    pub fn snapshot(&self, now: Instant) -> ObservationSnapshot {
        let state = self;
        loop {
            let before = state.generation.load(Ordering::Acquire);
            if before % 2 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let phase = SchedulerPhase::from_u8(state.phase.load(Ordering::Relaxed));
            let started_ns = state.phase_started_ns.load(Ordering::Relaxed);
            let mut snapshot = ObservationSnapshot {
                current_phase: phase,
                current_phase_started_ns: started_ns,
                reaction_elapsed_ns: state.reaction_elapsed_ns.load(Ordering::Relaxed),
                framework_elapsed_ns: state.framework_elapsed_ns.load(Ordering::Relaxed),
                physical_wait_elapsed_ns: state.physical_wait_elapsed_ns.load(Ordering::Relaxed),
                external_wait_elapsed_ns: state.external_wait_elapsed_ns.load(Ordering::Relaxed),
                coordination_wait_elapsed_ns: state
                    .coordination_wait_elapsed_ns
                    .load(Ordering::Relaxed),
                processed_tags: state.processed_tags.load(Ordering::Relaxed),
                processed_reactions: state.processed_reactions.load(Ordering::Relaxed),
                processed_events: state.processed_events.load(Ordering::Relaxed),
                set_ports: state.set_ports.load(Ordering::Relaxed),
                scheduled_actions: state.scheduled_actions.load(Ordering::Relaxed),
            };
            add_snapshot_elapsed(
                &mut snapshot,
                phase,
                elapsed_ns(state.origin, now).saturating_sub(started_ns),
            );
            if state.generation.load(Ordering::Acquire) == before {
                return snapshot;
            }
        }
    }
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
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
