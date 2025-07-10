#![allow(unused)]

use boomerang::prelude::*;

#[reactor]
fn A(#[input] x: (), #[output] y: ()) -> impl Reactor2 {
    builder
        .add_reaction(None)
        .with_trigger(x)
        .with_effect(y)
        .with_reaction_fn(|_, _, _| {})
        .finish()?;
}

#[reactor]
fn B(#[input] x: (), #[output] y: ()) -> impl Reactor2 {
    // The startup reaction needs to be defined first
    builder
        .add_reaction(None)
        .with_startup_trigger()
        .with_effect(y)
        .with_reaction_fn(|ctx, state, (startup, mut y)| {})
        .finish()?;

    builder
        .add_reaction(None)
        .with_trigger(x)
        .with_reaction_fn(|_, _, _| {})
        .finish()?;
}

#[reactor]
fn Cycle() -> impl Reactor2 {
    let a = builder.add_child_reactor(A(), "a", Default::default(), false)?;
    let b = builder.add_child_reactor(B(), "b", Default::default(), false)?;
    builder.connect_port(a.y, b.x, None, false)?;
    builder.connect_port(b.y, a.x, None, false)?;
}

#[test]
fn cycle() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor2(Cycle(), "cycle", (), config).unwrap();
}
