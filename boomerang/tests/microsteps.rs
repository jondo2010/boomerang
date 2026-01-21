//! Example ported from https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/Microsteps.lf
use boomerang::prelude::*;

#[reactor]
fn Destination(#[input] x: u32, #[input] y: u32) -> impl Reactor {
    reaction! {
        (x, y) {
            let elapsed = ctx.get_elapsed_logical_time();
            println!("Time since start: {elapsed:?}");
            assert!(elapsed.is_zero(), "Expected zero elapsed time!");
            let mut count = 0;
            if x.is_some() {
                println!("  x is present");
                count += 1;
            }
            if y.is_some() {
                println!("  y is present");
                count += 1;
            }
            assert_eq!(count, 1, "Expected exactly one input!");
        }
    }
}

#[reactor]
fn Microsteps() -> impl Reactor {
    let start = builder.add_timer("start", TimerSpec::STARTUP)?;
    let repeat = builder.add_logical_action::<()>("repeat", None)?;
    let d = builder.add_child_reactor(Destination(), "d", (), false)?;

    builder
        .add_reaction(None)
        .with_trigger(start)
        .with_effect(d.x)
        .with_effect(repeat)
        .with_reaction_fn(|ctx, _state, (_start, mut d_x, mut repeat)| {
            *d_x = Some(1);
            ctx.schedule_action(&mut repeat, (), None);
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_trigger(repeat)
        .with_effect(d.y)
        .with_reaction_fn(|_ctx, _state, (_repeat, mut d_y)| {
            *d_y = Some(1);
        })
        .finish()?;
}

#[test]
fn main() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(Microsteps(), "microsteps", (), config)
        .unwrap();
}
