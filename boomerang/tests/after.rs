//! This checks that the after keyword adjusts logical time, not using physical time.

use boomerang::prelude::*;

#[reactor]
fn Foo(#[input] x: i32, #[output] y: i32) -> impl Reactor {
    builder
        .add_reaction(None)
        .with_trigger(x)
        .with_effect(y)
        .with_reaction_fn(|_ctx, _state, (x, mut y)| {
            *y = x.map(|x| 2 * x);
        })
        .finish()?;
}

#[reactor]
fn Print(
    #[state(default = Duration::milliseconds(10))] expected_time: Duration,
    #[state] i: usize,
    #[input] x: i32,
) -> impl Reactor {
    builder
        .add_reaction(None)
        .with_trigger(x)
        .with_reaction_fn(|ctx, state, (x,)| {
            state.i += 1;
            let elapsed_time = ctx.get_elapsed_logical_time();
            println!("Result is {:?}", *x);
            assert_eq!(*x, Some(84), "Expected result to be 84");
            println!("Current logical time is: {}", elapsed_time);
            println!("Current physical time is: {:?}", ctx.get_physical_time());
            assert_eq!(
                elapsed_time, state.expected_time,
                "Expected logical time to be {}",
                state.expected_time
            );
            state.expected_time += Duration::seconds(1);
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            println!("Final result is {}", state.i);
            assert!(state.i != 0, "ERROR: Final reactor received no data.");
        })
        .finish()?;
}

#[reactor]
fn After() -> impl Reactor {
    let f = builder.add_child_reactor(Foo(), "foo", Default::default(), false)?;
    let p = builder.add_child_reactor(Print(), "print", Default::default(), false)?;
    let t = builder.add_timer("t", TimerSpec::default().with_period(Duration::SECOND))?;
    builder.connect_port(f.y, p.x, Some(Duration::milliseconds(10)), false)?;

    builder
        .add_reaction(None)
        .with_trigger(t)
        .with_effect(f.x)
        .with_reaction_fn(|ctx, _state, (_t, mut x)| {
            *x = Some(42);
            let elapsed_time = ctx.get_elapsed_logical_time();
            println!("Timer @ {elapsed_time}!");
        })
        .finish()?;
}

#[test]
fn main() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(3));
    let _ = boomerang_util::runner::build_and_test_reactor(
        After(),
        "after",
        Default::default(),
        config,
    )
    .unwrap();
}
