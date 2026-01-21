//! Events are scheduled with increasing additional delays of 0, 100, 300, 600 msec on a logical action with a minimum delay of 100 msec.  
//!
//! The use of the logical action ensures the elapsed time jumps exactly from 0 to 100, 300, 600, and 1000 msec.
//!
//! Ported from https://github.com/lf-lang/lingua-franca/blob/master/test/C/src/SlowingClock.lf
use boomerang::prelude::*;

#[reactor]
fn SlowingClock(
    #[state(default = Duration::milliseconds(100))] interval: Duration,
    #[state(default = Duration::milliseconds(100))] expected_time: Duration,
) -> impl Reactor {
    let a = builder.add_logical_action("a", Some(Duration::milliseconds(100)))?;

    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(a)
        .with_reaction_fn(|ctx, _state, (_startup, mut a)| {
            println!("startup");
            ctx.schedule_action(&mut a, (), None);
        })
        .finish()?;

    builder
        .add_reaction(Some("A"))
        .with_trigger(a)
        .with_reaction_fn(|ctx, state, (mut a,)| {
            let elapsed_logical_time = ctx.get_elapsed_logical_time();
            println!("Logical time since start: {elapsed_logical_time}.",);
            assert!(
                elapsed_logical_time == state.expected_time,
                "ERROR: Expected time to be: {}.",
                state.expected_time
            );

            ctx.schedule_action(&mut a, (), Some(state.interval));
            state.expected_time += Duration::milliseconds(100) + state.interval;
            state.interval += Duration::milliseconds(100);
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert_eq!(
                state.expected_time.whole_milliseconds(),
                1500,
                "ERROR: Expected the next expected_time to be: 1500ms. It was: {}.",
                state.expected_time
            );
            println!("Test passes.");
        })
        .finish()?;
}

#[test]
fn slowing_clock() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::milliseconds(1000));
    let _ = boomerang_util::runner::build_and_test_reactor(
        SlowingClock(),
        "slowing_clock",
        Default::default(),
        config,
    )
    .unwrap();
}
