//! Test data transport across hierarchy.

use boomerang::prelude::*;

#[reactor]
fn Source(#[output] out: u32) -> impl Reactor2 {
    let t = builder.add_timer("t", TimerSpec::default())?;
    builder
        .add_reaction2(Some("SourceReactionOut"))
        .with_trigger(t)
        .with_effect(out)
        .with_reaction_fn(|_ctx, _state, (_t, mut out)| {
            *out = Some(1);
        })
        .finish()?;
}

#[derive(typed_builder::TypedBuilder)]
struct GainParams {
    #[builder(default = 1)]
    gain: u32,
}

#[reactor]
fn Gain(#[input] inp: u32, #[output] out: u32, #[param(default = 1)] gain: u32) -> impl Reactor2 {
    builder
        .add_reaction2(Some("GainReactionIn"))
        .with_trigger(inp)
        .with_effect(out)
        .with_reaction_fn(move |_ctx, _state, (inp, mut out)| {
            *out = Some(inp.unwrap() * gain);
        })
        .finish()?;
}

#[reactor]
fn Print(#[input] inp: u32) -> impl Reactor2 {
    builder
        .add_reaction2(Some("PrintReactionIn"))
        .with_trigger(inp)
        .with_reaction_fn(|_ctx, _state, (inp,)| {
            let value = *inp;
            assert!(matches!(value, Some(2u32)));
            println!("Received {}", value.unwrap());
        })
        .finish()?;
}

#[reactor]
fn GainContainer(#[input] inp: u32, #[output] out: u32, #[output] out2: u32) -> impl Reactor2 {
    let gain = builder.add_child_reactor2(Gain(2), "gain", (), false)?;
    builder.connect_port(inp, gain.inp, None, false)?;
    builder.connect_port(gain.out, out, None, false)?;
    builder.connect_port(gain.out, out2, None, false)?;
}

#[reactor]
fn Hierarchy() -> impl Reactor2 {
    let source = builder.add_child_reactor2(Source(), "source", (), false)?;
    let container = builder.add_child_reactor2(GainContainer(), "container", (), false)?;
    let print = builder.add_child_reactor2(Print(), "print", (), false)?;
    let print2 = builder.add_child_reactor2(Print(), "print2", (), false)?;

    builder.connect_port(source.out, container.inp, None, false)?;
    builder.connect_port(container.out, print.inp, None, false)?;
    builder.connect_port(container.out2, print2.inp, None, false)?;
}

#[test]
fn hierarchy() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor2(Hierarchy(), "hierarchy", (), config)
        .unwrap();
}
