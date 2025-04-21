// Example in the Wiki.

use boomerang::prelude::*;

#[reactor]
fn Scale(#[input] x: u32, #[output] y: u32, scale: u32) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_effect(y)
        .with_reaction_fn(move |_ctx, _state, (x, mut y)| {
            *y = Some(scale * x.unwrap());
        })
        .finish()?;
}

#[reactor]
fn Test(#[input] x: u32) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_reaction_fn(move |_ctx, _state, (x,)| {
            println!("Received {:?}", *x);
            assert_eq!(*x, Some(2), "Expected Some(2)!");
        })
        .finish()?;
}

#[reactor]
fn Gain() -> impl Reactor2 {
    let g = builder.add_child_reactor2(Scale(2), "g", (), false)?;
    let t = builder.add_child_reactor2(Test(), "t", (), false)?;
    let tim = builder.add_timer("tim", TimerSpec::STARTUP)?;
    builder.connect_port(g.y, t.x, None, false)?;
    builder
        .add_reaction2(None)
        .with_trigger(tim)
        .with_effect(g.x)
        .with_reaction_fn(move |_ctx, _state, (_tim, mut g_x)| {
            *g_x = Some(1);
        })
        .finish()?;
}

#[test]
fn test_gain() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor2(Gain(), "gain", (), config).unwrap();
}
