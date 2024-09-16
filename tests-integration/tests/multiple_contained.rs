/// Test that a reaction can react to and send to multiple ports of a contained reactor.
use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

#[derive(Reactor)]
#[reactor(
    state = "()",
    reaction = "ReactionIn1",
    reaction = "ReactionIn2",
    // ReactionStartup needs to be last due to implicit dependency on all reactions before it.
    reaction = "ReactionStartup"
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

impl Trigger<Contained> for ReactionStartup<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.trigger = Some(42);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Contained")]
struct ReactionIn1<'a> {
    in1: runtime::InputRef<'a, u32>,
}

impl Trigger<Contained> for ReactionIn1<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        //println!("{} received {:?}", self.in1.get_name(), *self.in1);
        assert_eq!(*self.in1, Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reaction)]
#[reaction(reactor = "Contained")]
struct ReactionIn2<'a> {
    in2: runtime::InputRef<'a, u32>,
}

impl Trigger<Contained> for ReactionIn2<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        //println!("{} received {:?}", self.in2.get_name(), *self.in2.get());
        assert_eq!(*self.in2, Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "ReactionCTrigger")]
struct MultipleContained {
    #[reactor(child = "()")]
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

impl Trigger<MultipleContained> for ReactionCTrigger<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.c_in1 = *self.c_trigger;
        *self.c_in2 = *self.c_trigger;
    }
}

#[test]
fn multiple_contained() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<MultipleContained>(
        "multiple_contained",
        (),
        true,
        false,
    )
    .unwrap();
}
