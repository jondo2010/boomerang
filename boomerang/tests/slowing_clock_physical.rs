//! In this example, events are scheduled with increasing additional delays of 0, 100, 300, 600 msec
//! on a physical action with a minimum delay of 100 msec.  The use of the physical action makes the
//! elapsed time jumps from 0 to approximately 100 msec, to approximatly 300 msec thereafter,
//! drifting away further with each new event. Modeled after the Lingua-Franca C version of this
//! test. @author Maiko Brants TU Dresden

use boomerang::prelude::*;

#[reactor]
fn SlowingClockPhysical(
    #[state] interval: Duration,
    #[state] expected_time: Duration,
) -> impl Reactor {
    let a = builder.add_physical_action::<()>("a", Some(Duration::milliseconds(100)))?;

    builder
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(a)
        .with_reaction_fn(|ctx, state, (_startup, mut a)| {
            state.expected_time = Duration::milliseconds(100);
            ctx.schedule_action(&mut a, (), None);
        })
        .finish()?;

    builder
        .add_reaction(Some("A"))
        .with_trigger(a)
        .with_reaction_fn(|ctx, state, (mut a,)| {
            let elapsed_logical_time = ctx.get_elapsed_logical_time();
            println!("Logical time since start: {elapsed_logical_time:?}");
            assert!(
                elapsed_logical_time >= state.expected_time,
                "Expected logical time to be at least: {:?}, was {elapsed_logical_time:?}",
                state.expected_time
            );
            state.interval += Duration::milliseconds(100);
            state.expected_time = Duration::milliseconds(100) + state.interval;
            println!(
                "Scheduling next to occur approximately after: {:?}",
                state.interval
            );
            ctx.schedule_action(&mut a, (), Some(state.interval));
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            assert!(
                state.expected_time >= Duration::milliseconds(500),
                "Expected the next expected time to be at least: 500 msec. It was: {:?}",
                state.expected_time
            );
        })
        .finish()?;
}

#[test]
fn slowing_clock_physical() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_keep_alive(true)
        .with_timeout(Duration::milliseconds(1500));
    let _ = boomerang_util::runner::build_and_test_reactor(
        SlowingClockPhysical(),
        "slowing_clock_physical",
        SlowingClockPhysicalState {
            interval: Duration::milliseconds(100),
            expected_time: Duration::milliseconds(200),
        },
        config,
    )
    .unwrap();
}
