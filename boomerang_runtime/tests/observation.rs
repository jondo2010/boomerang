use std::time::{Duration, Instant};

use std::sync::Arc;

use boomerang_runtime::{
    Config, ObservationHandle, ObservationState, SchedulerLifecycle, SchedulerPhase,
};

#[test]
fn config_retains_an_opt_in_observation_handle() {
    let observation: ObservationHandle = Arc::new(ObservationState::new(Instant::now()));
    let config = Config::default().with_observation(observation.clone());

    assert!(config.observation().is_some());
    assert_eq!(
        config
            .observation()
            .unwrap()
            .snapshot(Instant::now())
            .unwrap()
            .current_phase,
        SchedulerPhase::Idle
    );
}

#[test]
fn snapshot_includes_an_ongoing_reaction_without_closing_it() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    observation.enter(SchedulerPhase::Reaction, origin + Duration::from_millis(10));
    let snapshot = observation.snapshot(origin + Duration::from_millis(25)).unwrap();

    assert_eq!(snapshot.current_phase, SchedulerPhase::Reaction);
    assert_eq!(snapshot.current_phase_started_ns, 10_000_000);
    assert_eq!(snapshot.reaction_elapsed_ns, 15_000_000);

    let later = observation.snapshot(origin + Duration::from_millis(40)).unwrap();
    assert_eq!(later.current_phase, SchedulerPhase::Reaction);
    assert_eq!(later.reaction_elapsed_ns, 30_000_000);
}

#[test]
fn phase_transition_accounts_elapsed_time_once() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    observation.enter(SchedulerPhase::Framework, origin + Duration::from_millis(4));
    observation.enter(
        SchedulerPhase::PhysicalWait,
        origin + Duration::from_millis(9),
    );
    let snapshot = observation.snapshot(origin + Duration::from_millis(12)).unwrap();

    assert_eq!(snapshot.framework_elapsed_ns, 5_000_000);
    assert_eq!(snapshot.physical_wait_elapsed_ns, 3_000_000);
    assert_eq!(snapshot.reaction_elapsed_ns, 0);
}

#[test]
fn work_counters_are_cumulative_and_saturate_without_wrapping() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    observation.add_processed_reactions(u64::MAX);
    observation.add_processed_reactions(1);
    observation.increment_processed_tags();

    let snapshot = observation.snapshot(origin).unwrap();
    assert_eq!(snapshot.processed_reactions, u64::MAX);
    assert_eq!(snapshot.processed_tags, 1);
}

#[test]
fn event_queue_observation_distinguishes_capacity_limit_and_peak() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    observation.record_event_queue(3, 8);
    observation.record_event_queue(2, 16);

    let snapshot = observation.snapshot(origin).unwrap();
    assert_eq!(snapshot.event_queue_occupancy, 2);
    assert_eq!(snapshot.event_queue_reserved_capacity, 16);
    assert_eq!(snapshot.event_queue_enforced_limit, None);
    assert_eq!(snapshot.event_queue_peak_occupancy, 3);
}

#[test]
fn lifecycle_and_logical_progress_are_independent_of_measurement_time() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    assert_eq!(
        observation.snapshot(origin).unwrap().lifecycle,
        SchedulerLifecycle::NotStarted
    );

    observation.mark_running();
    observation.record_completed_tag(origin + Duration::from_millis(7));

    let running = observation.snapshot(origin + Duration::from_millis(20)).unwrap();
    assert_eq!(running.lifecycle, SchedulerLifecycle::Running);
    assert_eq!(running.completed_logical_tags, 1);
    assert_eq!(running.processed_tags, 1);
    assert_eq!(running.last_logical_progress_ns, Some(7_000_000));

    observation.mark_stopped();
    assert_eq!(
        observation
            .snapshot(origin + Duration::from_millis(25))
            .unwrap()
            .lifecycle,
        SchedulerLifecycle::Stopped
    );
}
