use boomerang::prelude::*;

#[derive(Default)]
struct SinkState {
    seen: bool,
}

#[reactor]
fn Source<const WIDTH: usize>(#[output(len = WIDTH)] out: i32) -> impl Reactor {
    builder
        .add_reaction(Some("startup"))
        .with_startup_trigger()
        .with_effect(out)
        .with_reaction_fn(|_ctx, _state, (_startup, mut out)| {
            for (idx, port) in out.iter_mut().enumerate() {
                **port = Some(idx as i32);
            }
        })
        .finish()?;
}

#[reactor(state = SinkState)]
fn Sink<const WIDTH: usize>(#[input(len = WIDTH)] input: i32) -> impl Reactor {
    builder
        .add_reaction(Some("inputs"))
        .with_trigger(input)
        .with_reaction_fn(|_ctx, state, (input,)| {
            let sum = input
                .iter()
                .filter_map(|port| port.as_ref().copied())
                .sum::<i32>();
            assert_eq!(sum, 3);
            state.seen = true;
        })
        .finish()?;

    builder
        .add_reaction(Some("shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_shutdown,)| {
            assert!(state.seen);
        })
        .finish()?;
}

#[reactor]
fn Main() -> impl Reactor {
    let source = builder.add_child_reactor(
        Source::<3>(),
        "source",
        (),
        false,
    )?;
    let sink = builder.add_child_reactor(
        Sink::<3>(),
        "sink",
        SinkState { seen: false },
        false,
    )?;

    builder.connect_ports(source.out.iter(), sink.input.iter(), None, false)?;
}

#[test]
fn port_bank_reaction() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        Main(),
        "port_bank_reaction",
        (),
        config,
    )
    .unwrap();
}
