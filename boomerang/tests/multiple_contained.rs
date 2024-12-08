//! Test that a reaction can react to and send to multiple ports of a contained reactor.

use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(
    state = "()",
    reaction = "ReactionStartup",
    reaction = "ReactionIn1",
    reaction = "ReactionIn2"
)]
struct Contained {
    in1: TypedPortKey<u32, Input>,
    in2: TypedPortKey<u32, Input>,
    trigger: TypedPortKey<u32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "Contained", triggers(startup))]
struct ReactionStartup<'a> {
    trigger: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<()> for ReactionStartup<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.trigger = Some(42);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Contained")]
struct ReactionIn1<'a> {
    in1: runtime::InputRef<'a, u32>,
}

impl runtime::Trigger<()> for ReactionIn1<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("{} received {:?}", self.in1.name(), *self.in1);
        assert_eq!(*self.in1, Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Contained")]
struct ReactionIn2<'a> {
    in2: runtime::InputRef<'a, u32>,
}

impl runtime::Trigger<()> for ReactionIn2<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("{} received {:?}", self.in2.name(), *self.in2);
        assert_eq!(*self.in2, Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reactor)]
#[reactor(state = (), reaction = ReactionCTrigger)]
struct MultipleContained {
    #[reactor(child(state = ()))]
    c: Contained,
}

#[derive(Reaction)]
#[reaction(reactor = "MultipleContained")]
struct ReactionCTrigger<'a> {
    #[reaction(path = "c.trigger")]
    c_trigger: runtime::InputRef<'a, u32>,
    #[reaction(path = "c.in1")]
    c_in1: runtime::OutputRef<'a, u32>,
    #[reaction(path = "c.in2")]
    c_in2: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<()> for ReactionCTrigger<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.c_in1 = *self.c_trigger;
        *self.c_in2 = *self.c_trigger;
    }
}

#[test]
fn multiple_contained() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<MultipleContained>(
        "multiple_contained",
        (),
        config,
    )
    .unwrap();
}
