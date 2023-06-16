// Example in the Wiki.

use boomerang::{
    builder::{BuilderReactionKey, TypedActionKey, TypedPortKey},
    runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "Scale")]
struct ScaleBuilder {
    #[reactor(input())]
    x: TypedPortKey<u32>,
    #[reactor(output())]
    y: TypedPortKey<u32>,
    #[reactor(reaction(function = "Scale::reaction_x"))]
    reaction_x: BuilderReactionKey,
}

#[derive(Clone)]
struct Scale(u32);
impl Scale {
    #[boomerang::reaction(reactor = "ScaleBuilder")]
    fn reaction_x(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<u32>,
        #[reactor::port(effects)] y: &mut runtime::Port<u32>,
    ) {
        *y.get_mut() = Some(self.0 * x.get().unwrap());
    }
}

#[derive(Reactor)]
#[reactor(state = "Test")]
struct TestBuilder {
    #[reactor(input())]
    x: TypedPortKey<u32>,
    #[reactor(reaction(function = "Test::reaction_x"))]
    reaction_x: BuilderReactionKey,
}

#[derive(Clone)]
struct Test;
impl Test {
    #[boomerang::reaction(reactor = "TestBuilder")]
    fn reaction_x(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<u32>,
    ) {
        println!("Received {:?}", x.get());
        assert_eq!(*x.get(), Some(2), "Expected Some(2)!");
    }
}

#[derive(Reactor)]
#[reactor(state = "Gain", connection(from = "g.y", to = "t.x"))]
struct GainBuilder {
    #[reactor(child(state = "Scale(2)"))]
    g: ScaleBuilder,

    #[reactor(child(state = "Test"))]
    #[allow(dead_code)]
    t: TestBuilder,

    #[reactor(timer(rename = "tim"))]
    tim: TypedActionKey<()>,

    #[reactor(reaction(function = "Gain::reaction_tim",))]
    reaction_tim: BuilderReactionKey,
}

#[derive(Clone)]
struct Gain;
impl Gain {
    #[boomerang::reaction(reactor = "GainBuilder", triggers(action = "tim"))]
    fn reaction_tim(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects, path = "g.x")] g_x: &mut runtime::Port<u32>,
    ) {
        *g_x.get_mut() = Some(1);
    }
}

#[test_log::test]
#[cfg(not(feature = "federated"))]
fn gain() {
    let _ = boomerang::runner::build_and_test_reactor::<GainBuilder>("gain", Gain, true, false)
        .unwrap();
}
