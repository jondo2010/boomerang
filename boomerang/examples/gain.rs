// Example in the Wiki.

use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey},
    runtime, Reactor, run
};

#[derive(Reactor)]
#[reactor(state = "Scale")]
struct ScaleBuilder {
    #[reactor(input())]
    x: BuilderPortKey<u32>,
    #[reactor(output())]
    y: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Scale::reaction_x"))]
    reaction_x: runtime::ReactionKey,
}

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
    x: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Test::reaction_x"))]
    reaction_x: runtime::ReactionKey,
}

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
    tim: BuilderActionKey,

    #[reactor(reaction(function = "Gain::reaction_tim",))]
    reaction_tim: runtime::ReactionKey,
}

#[derive(Debug)]
struct Gain;
impl Gain {
    #[boomerang::reaction(reactor = "GainBuilder", triggers(timer = "tim"))]
    fn reaction_tim(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects, path = "g.x")] g_x: &mut runtime::Port<u32>,
    ) {
        *g_x.get_mut() = Some(1);
    }
}

fn main() {
    tracing_subscriber::fmt::init();
    let _ = run::build_and_run_reactor::<GainBuilder>("gain", Gain).unwrap();
}
