//! This test checks the correctness of the cycle between two enclaves.
//!
//! Ported from LF https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/enclave/EnclaveCycle.lf

use boomerang::prelude::*;

#[reactor]
fn Ping(
    #[input] input: i32,
    #[output] output: i32,
    #[state] counter: i32,
    #[state] received: bool,
) -> impl Reactor {
    let t = builder.add_timer(
        "t",
        TimerSpec::default().with_period(Duration::milliseconds(100)),
    )?;
    let shutdown = builder.get_shutdown_action();

    builder
        .add_reaction(Some("ReactionT"))
        .with_trigger(t)
        .with_effect(output)
        .with_reaction_fn(|_ctx, state, (_t, mut output)| {
            let elapsed = _ctx.get_elapsed_logical_time();
            println!("Ping Sent {} at {elapsed}", state.counter);
            *output = Some(state.counter);
            state.counter += 1;
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionIn"))
        .with_trigger(input)
        .with_reaction_fn(|ctx, state, (input,)| {
            state.received = true;
            let value = *input;
            let elapsed = ctx.get_elapsed_logical_time();
            println!("Ping Received {value:?} at {elapsed}");
            let expected = Duration::milliseconds(50 + 100 * value.unwrap() as i64);
            assert_eq!(
                elapsed, expected,
                "Ping expected value at {expected} but received it at {elapsed}",
            );
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionShutdown"))
        .with_trigger(shutdown)
        .with_reaction_fn(|_ctx, state, _| {
            if !state.received {
                panic!("Nothing received.");
            }
        })
        .finish()?;
}

#[reactor]
fn Pong(
    #[input] input: i32,
    #[output] output: i32,
    #[state] received: bool,
) -> impl Reactor<PongState, Ports = PongPorts> {
    builder
        .add_reaction(Some("RectionIn"))
        .with_trigger(input)
        .with_effect(output)
        .with_reaction_fn(|_ctx, state, (input, mut output)| {
            state.received = true;
            let value = *input;
            let elapsed = _ctx.get_elapsed_logical_time();
            println!("Pong Received {value:?} at {elapsed}");
            let expected = Duration::milliseconds(100 * value.unwrap() as i64);
            assert_eq!(
                elapsed, expected,
                "Pong expected value at {expected} but received it at {elapsed}",
            );
            *output = value;
        })
        .finish()?;

    builder
        .add_reaction(None)
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, _| {
            if !state.received {
                panic!("Nothing received.");
            }
        })
        .finish()?;
}

#[reactor]
fn MainReactor() -> impl Reactor {
    let ping = builder.add_child_reactor(Ping(), "ping", PingState::default(), true)?;
    let pong = builder.add_child_reactor(Pong(), "pong", PongState::default(), true)?;
    builder.connect_port(ping.output, pong.input, None, false)?;
    builder.connect_port(
        pong.output,
        ping.input,
        Some(Duration::milliseconds(50)),
        false,
    )?;
}

#[test]
fn enclave_cycle() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(1));
    let (_, _env) =
        boomerang_util::runner::build_and_test_reactor(MainReactor(), "enclave_cycle", (), config)
            .unwrap();
}
