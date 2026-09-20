use std::time::{Duration, Instant};

use std::sync::Arc;

use boomerang_runtime::{Config, ObservationHandle, ObservationState, SchedulerPhase};

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
            .current_phase,
        SchedulerPhase::Idle
    );
}

#[test]
fn snapshot_includes_an_ongoing_reaction_without_closing_it() {
    let origin = Instant::now();
    let observation = ObservationState::new(origin);

    observation.enter(SchedulerPhase::Reaction, origin + Duration::from_millis(10));
    let snapshot = observation.snapshot(origin + Duration::from_millis(25));

    assert_eq!(snapshot.current_phase, SchedulerPhase::Reaction);
    assert_eq!(snapshot.current_phase_started_ns, 10_000_000);
    assert_eq!(snapshot.reaction_elapsed_ns, 15_000_000);

    let later = observation.snapshot(origin + Duration::from_millis(40));
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
    let snapshot = observation.snapshot(origin + Duration::from_millis(12));

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

    let snapshot = observation.snapshot(origin);
    assert_eq!(snapshot.processed_reactions, u64::MAX);
    assert_eq!(snapshot.processed_tags, 1);
}
