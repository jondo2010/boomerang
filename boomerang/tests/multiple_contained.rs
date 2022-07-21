/// Test that a reaction can react to and send two multiple ports of a contained reactor.
use boomerang::{builder::BuilderPortKey, runtime, Reactor, boomerang_test_body};

#[derive(Reactor)]
struct ContainedBuilder {
    #[reactor(output())]
    trigger: BuilderPortKey<u32>,
    #[reactor(input())]
    in1: BuilderPortKey<u32>,
    #[reactor(input())]
    in2: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Contained::reaction_startup",))]
    reaction_startup: runtime::ReactionKey,
    #[reactor(reaction(function = "Contained::reaction_in1"))]
    reaction_in1: runtime::ReactionKey,
    #[reactor(reaction(function = "Contained::reaction_in2"))]
    reaction_in2: runtime::ReactionKey,
}
struct Contained;
impl Contained {
    #[boomerang::reaction(reactor = "ContainedBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(effects)] trigger: &mut runtime::Port<u32>,
    ) {
        *trigger.get_mut() = Some(42);
    }
    #[boomerang::reaction(reactor = "ContainedBuilder")]
    fn reaction_in1(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers)] in1: &runtime::Port<u32>,
    ) {
        println!("in1 received {:?}", *in1.get());
        assert_eq!(*in1.get(), Some(42), "FAILED: Expected 42.");
    }
    #[boomerang::reaction(reactor = "ContainedBuilder")]
    fn reaction_in2(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers)] in2: &runtime::Port<u32>,
    ) {
        println!("in1 received {:?}", *in2.get());
        assert_eq!(*in2.get(), Some(42), "FAILED: Expected 42.");
    }
}

#[derive(Reactor)]
struct MultipleContainedBuilder {
    #[reactor(child(state = "Contained"))]
    c: ContainedBuilder,
    #[reactor(reaction(function = "MultipleContained::reaction_c_trigger",))]
    reaction_c_trigger: runtime::ReactionKey,
}
struct MultipleContained;
impl MultipleContained {
    #[boomerang::reaction(reactor = "MultipleContainedBuilder")]
    fn reaction_c_trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers, path = "c.trigger")] c_trigger: &runtime::Port<u32>,
        #[reactor::port(effects, path = "c.in1")] c_in1: &mut runtime::Port<u32>,
        #[reactor::port(effects, path = "c.in2")] c_in2: &mut runtime::Port<u32>,
    ) {
        *c_in1.get_mut() = *c_trigger.get();
        *c_in2.get_mut() = *c_trigger.get();
    }
}

boomerang_test_body!(multiple_contained, MultipleContainedBuilder, MultipleContained);