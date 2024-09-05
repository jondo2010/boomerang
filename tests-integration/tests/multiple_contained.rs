/// Test that a reaction can react to and send to multiple ports of a contained reactor.
use boomerang::{
    builder::prelude::*,
    runtime::{self, BasePort},
    Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct Contained {
    #[reactor(port = "input")]
    in1: TypedPortKey<u32>,
    #[reactor(port = "input")]
    in2: TypedPortKey<u32>,
    #[reactor(port = "output")]
    trigger: TypedPortKey<u32>,

    reaction_in1: TypedReactionKey<ReactionIn1<'static>>,
    reaction_in2: TypedReactionKey<ReactionIn2<'static>>,
    // reaction_startup needs to be last due to implicit dependency on all reactions before it.
    reaction_startup: TypedReactionKey<ReactionStartup<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct ReactionStartup<'a> {
    trigger: &'a mut runtime::Port<u32>,
}

impl Trigger for ReactionStartup<'_> {
    type Reactor = Contained;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.trigger.get_mut() = Some(42);
    }
}

#[derive(Reaction)]
struct ReactionIn1<'a> {
    in1: &'a runtime::Port<u32>,
}

impl Trigger for ReactionIn1<'_> {
    type Reactor = Contained;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("{} received {:?}", self.in1.get_name(), *self.in1.get());
        assert_eq!(*self.in1.get(), Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reaction)]
struct ReactionIn2<'a> {
    in2: &'a runtime::Port<u32>,
}

impl Trigger for ReactionIn2<'_> {
    type Reactor = Contained;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("{} received {:?}", self.in2.get_name(), *self.in2.get());
        assert_eq!(*self.in2.get(), Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct MultipleContained {
    #[reactor(child = ())]
    c: Contained,
    reaction_c_trigger: TypedReactionKey<ReactionCTrigger<'static>>,
}

#[derive(Reaction)]
struct ReactionCTrigger<'a> {
    #[reaction(path = "c.trigger")]
    c_trigger: &'a runtime::Port<u32>,
    #[reaction(path = "c.in1")]
    c_in1: &'a mut runtime::Port<u32>,
    #[reaction(path = "c.in2")]
    c_in2: &'a mut runtime::Port<u32>,
}

impl Trigger for ReactionCTrigger<'_> {
    type Reactor = MultipleContained;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.c_in1.get_mut() = *self.c_trigger.get();
        *self.c_in2.get_mut() = *self.c_trigger.get();
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
