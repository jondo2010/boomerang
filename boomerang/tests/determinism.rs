use boomerang::prelude::*;

#[reactor]
fn Source(#[output] y: i32) -> impl Reactor2 {
    let t = builder.add_timer("t", TimerSpec::STARTUP)?;
    builder
        .add_reaction2(Some("SourceReactonT"))
        .with_trigger(t)
        .with_effect(y)
        .with_reaction_fn(|_ctx, _state, (_t, mut y)| {
            *y = Some(1);
        })
        .finish()?;
}

#[reactor]
fn Destination(#[input] x: i32, #[input] y: i32) -> impl Reactor2 {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_trigger(y)
        .with_reaction_fn(|_ctx, _state, (x, y)| {
            let mut sum = 0;
            if let Some(x) = *x {
                sum += x;
            }
            if let Some(y) = *y {
                sum += y;
            }
            println!("Received {sum}");
            assert_eq!(sum, 2, "FAILURE: Expected 2.");
        })
        .finish()?;
}

#[reactor]
fn Pass(#[input] x: i32, #[output] y: i32) -> impl Reactor2<(), Ports = PassPorts> {
    builder
        .add_reaction2(None)
        .with_trigger(x)
        .with_effect(y)
        .with_reaction_fn(|_ctx, _state, (x, mut y)| {
            *y = *x;
        })
        .finish()?;
}

#[reactor]
fn Determinism() -> impl Reactor2 {
    let s = builder.add_child_reactor2(Source(), "s", (), false)?;
    let d = builder.add_child_reactor2(Destination(), "d", (), false)?;
    let p1 = builder.add_child_reactor2(Pass(), "p1", (), false)?;
    let p2 = builder.add_child_reactor2(Pass(), "p2", (), false)?;
    builder.connect_port(s.y, d.y, None, false)?;
    builder.connect_port(s.y, p1.x, None, false)?;
    builder.connect_port(p1.y, p2.x, None, false)?;
    builder.connect_port(p2.y, d.x, None, false)?;
}

#[test]
fn main() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor2(Determinism(), "determinism", (), config)
            .unwrap();
}
