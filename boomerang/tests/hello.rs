//! This test checks that logical time is incremented an appropriate amount as a result of an invocation of the
//! schedule_action() function at runtime. It also performs various smoke tests of timing aligned reactions. The first
//! instance has a period of 4 seconds, the second of 2 seconds, and the third (composite) or 1 second.

use boomerang::prelude::*;

#[reactor]
fn Hello(
    _period: Duration,
    message: String,
    #[state] count: usize,
    #[state] previous_time: Duration,
) -> impl Reactor {
    let t = builder.add_timer(
        "t",
        TimerSpec::default()
            .with_offset(Duration::seconds(1))
            .with_period(Duration::seconds(2)),
    )?;
    let a = builder.add_logical_action::<()>("a", None)?;

    let message = message.clone();
    builder
        .add_reaction(Some("T"))
        .with_trigger(t)
        .with_effect(a)
        .with_reaction_fn(move |ctx, state, (_t, mut a)| {
            // Print the current time.
            state.previous_time = ctx.get_elapsed_logical_time();
            ctx.schedule_action(&mut a, (), Some(Duration::milliseconds(200))); // No payload.
            println!("{message} Current time is {:?}", state.previous_time);
        })
        .finish()?;

    builder
        .add_reaction(Some("A"))
        .with_trigger(a)
        .with_reaction_fn(|ctx, state, (_a,)| {
            state.count += 1;
            let time = ctx.get_elapsed_logical_time();
            println!("***** action {} at time {:?}", state.count, time);
            let diff = time - state.previous_time;
            assert_eq!(
                diff,
                Duration::milliseconds(200),
                "FAILURE: Expected 200 msecs of logical time to elapse but got {:?}",
                diff
            );
        })
        .finish()?;
}

#[reactor]
fn Inside(message: String) -> impl Reactor {
    let _third_instance = builder.add_child_reactor(
        Hello(Duration::seconds(1), message.clone()),
        "hello",
        Default::default(),
        false,
    )?;
}

#[reactor]
fn Main() -> impl Reactor {
    let _first_instance = builder.add_child_reactor(
        Hello(Duration::seconds(4), "Hello from first.".to_owned()),
        "hello",
        Default::default(),
        false,
    )?;
    let _second_instance = builder.add_child_reactor(
        Hello(Duration::seconds(2), "Hello from second.".to_owned()),
        "hello",
        Default::default(),
        false,
    )?;
    let _third_instance = builder.add_child_reactor(
        Inside("Hello from composite.".to_owned()),
        "hello",
        Default::default(),
        false,
    )?;
}

#[test]
fn hello() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(10));
    let _ = boomerang_util::runner::build_and_test_reactor(Main(), "hello", (), config).unwrap();
}
