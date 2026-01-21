//! Test that a reaction can react to and send to multiple ports of a contained reactor.

use boomerang::prelude::*;

#[reactor]
fn Contained(#[input] in1: u32, #[input] in2: u32, #[output] trigger: u32) -> impl Reactor {
    builder
        .add_reaction(Some("ReactionStartup"))
        .with_startup_trigger()
        .with_effect(trigger)
        .with_reaction_fn(|_ctx, _state, (_startup, mut trigger)| {
            *trigger = Some(42);
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionIn1"))
        .with_trigger(in1)
        .with_reaction_fn(|_ctx, _state, (in1,)| {
            println!("{} received {:?}", in1.name(), *in1);
            assert_eq!(*in1, Some(42), "FAILED: Expected 42.");
        })
        .finish()?;

    builder
        .add_reaction(Some("ReactionIn2"))
        .with_trigger(in2)
        .with_reaction_fn(|_ctx, _state, (in2,)| {
            println!("{} received {:?}", in2.name(), *in2);
            assert_eq!(*in2, Some(42), "FAILED: Expected 42.");
        })
        .finish()?;
}

#[reactor]
fn MultipleContained() -> impl Reactor {
    let c = builder.add_child_reactor(Contained(), "c", (), false)?;
    builder
        .add_reaction(None)
        .with_trigger(c.trigger)
        .with_effect(c.in1)
        .with_effect(c.in2)
        .with_reaction_fn(|_ctx, _state, (c_trigger, mut c_in1, mut c_in2)| {
            *c_in1 = *c_trigger;
            *c_in2 = *c_trigger;
        })
        .finish()?;
}

#[test]
fn multiple_contained() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor(
        MultipleContained(),
        "multiple_contained",
        (),
        config,
    )
    .unwrap();
}
