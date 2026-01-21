//! This tests actions with payloads by delaying an input by a fixed amount.
//!
//! Ported from <https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/DelayInt.lf>

use boomerang::prelude::*;

#[reactor]
fn Delay(
    #[input] in_: u32,
    #[output] out: u32,
    delay: Duration,
) -> impl Reactor<(), Ports = DelayPorts> {
    let d = builder.add_logical_action::<u32>("d", None)?;
    builder
        .add_reaction(None)
        .with_trigger(in_)
        .with_effect(d)
        .with_reaction_fn(move |ctx, _state, (in_, mut d)| {
            ctx.schedule_action(&mut d, in_.unwrap(), Some(delay));
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_trigger(d)
        .with_effect(out)
        .with_reaction_fn(|ctx, _state, (mut d, mut out)| {
            *out = ctx.get_action_value(&mut d).cloned();
        })
        .finish()?;
}

#[reactor]
pub fn Test(
    #[input] in_: u32,
    #[state(default = std::time::Instant::now())] start_time: std::time::Instant,
) -> impl Reactor<TestState, Ports = TestPorts> {
    builder
        .add_reaction(None)
        .with_startup_trigger()
        .with_reaction_fn(move |ctx, state, _| state.start_time = ctx.get_logical_time())
        .finish()?;

    builder
        .add_reaction(None)
        .with_trigger(in_)
        .with_reaction_fn(|ctx, state, (in_,)| {
            println!("Received: {}", in_.unwrap());
            // Check the time of the input.
            let current_time = ctx.get_logical_time();
            let elapsed = current_time - state.start_time;
            println!("After {elapsed:?} of logical time.");
            assert_eq!(
                elapsed,
                Duration::milliseconds(100),
                "Expected 100ms delay."
            );
            assert_eq!(*in_, Some(42), "Expected input to be 42.");
        })
        .finish()?;
}

#[reactor]
fn DelayInt() -> impl Reactor<(), Ports = DelayIntPorts> {
    let t = builder.get_startup_action();
    let d = builder.add_child_reactor(Delay(Duration::milliseconds(100)), "d", (), false)?;
    let test = builder.add_child_reactor(Test(), "test", TestState::default(), false)?;
    builder.connect_port(d.out, test.in_, None, false)?;
    builder
        .add_reaction(None)
        .with_trigger(t)
        .with_effect(d.in_)
        .with_reaction_fn(|_ctx, _state, (_t, mut d_in)| {
            *d_in = Some(42);
        })
        .finish()?;
}

#[test]
fn delay_int() {
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(1));
    let (_, _env) =
        boomerang_util::runner::build_and_test_reactor(DelayInt(), "delay_int", (), config)
            .unwrap();
}
