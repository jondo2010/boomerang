use boomerang::prelude::*;

#[derive(Default)]
struct SourceState {
    value: i32,
}

#[derive(Default)]
struct SinkState {
    expected: i32,
    seen: bool,
}

#[reactor(state = SourceState)]
fn Source(#[output] out: i32) -> impl Reactor {
    ctx.add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(out)
        .with_reaction_fn(|_ctx, state, (_startup, mut out)| {
            *out = Some(state.value);
        })
        .finish()?;
}

#[reactor(state = SinkState)]
fn Sink(width: usize, #[input(len = width)] input: i32) -> impl Reactor {
    ctx.add_reaction(Some("inputs"))
        .with_trigger(input)
        .with_reaction_fn(|_ctx, state, (input,): (runtime::InputBankRef<i32>,)| {
            for port in input.iter() {
                assert_eq!(port.as_ref().copied(), Some(state.expected));
            }
            state.seen = true;
        })
        .finish()?;

    ctx.add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert!(state.seen);
        })
        .finish()?;
}

#[reactor]
fn BroadcastMain() -> impl Reactor {
    let source = ctx.add_child_reactor(Source(), "source", SourceState { value: 7 }, false)?;
    let sink = ctx.add_child_reactor(
        Sink(3),
        "sink",
        SinkState {
            expected: 7,
            seen: false,
        },
        false,
    )?;

    ctx.connect_broadcast(source.out, sink.input.iter(), None, false)?;
}

#[reactor]
fn CartesianMain() -> impl Reactor {
    let source = ctx.add_child_reactor(Source(), "source", SourceState { value: 9 }, false)?;
    let sink = ctx.add_child_reactor(
        Sink(3),
        "sink",
        SinkState {
            expected: 9,
            seen: false,
        },
        false,
    )?;

    ctx.connect_cartesian(std::iter::once(source.out), sink.input.iter(), None, false)?;
}

#[test]
fn connect_broadcast() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        BroadcastMain(),
        "connect_broadcast",
        (),
        config,
    )
    .unwrap();
}

#[test]
fn connect_cartesian() {
    let _ = tracing_subscriber::fmt::try_init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        CartesianMain(),
        "connect_cartesian",
        (),
        config,
    )
    .unwrap();
}
