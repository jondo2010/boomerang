// Example in the Wiki.

use boomerang::{
    builder::{TimerActionKey, Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

struct Scale(u32);

#[derive(Clone, Reactor)]
#[reactor(state = Scale)]
struct ScaleBuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<u32>,
    #[reactor(port = "output")]
    y: TypedPortKey<u32>,
    reaction_x: TypedReactionKey<ScaleReactionX<'static>>,
}

#[derive(Reaction)]
struct ScaleReactionX<'a> {
    x: &'a runtime::Port<u32>,
    y: &'a mut runtime::Port<u32>,
}

impl Trigger for ScaleReactionX<'_> {
    type Reactor = ScaleBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Scale) {
        *self.y.get_mut() = Some(state.0 * self.x.get().unwrap());
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct TestBuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<u32>,
    reaction_x: TypedReactionKey<TestReactionX<'static>>,
}

#[derive(Reaction)]
struct TestReactionX<'a> {
    x: &'a runtime::Port<u32>,
}

impl Trigger for TestReactionX<'_> {
    type Reactor = TestBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("Received {:?}", self.x.get());
        assert_eq!(*self.x.get(), Some(2), "Expected Some(2)!");
    }
}

#[derive(Clone, Reactor)]
#[reactor(
    state = (),
    connection(from = "g.y", to = "t.x")
)]
struct GainBuilder {
    #[reactor(child= Scale(2))]
    g: ScaleBuilder,

    #[reactor(child = ())]
    #[allow(dead_code)]
    t: TestBuilder,

    #[reactor(timer())]
    tim: TimerActionKey,

    reaction_tim: TypedReactionKey<GainReactionTim<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "tim"))]
struct GainReactionTim<'a> {
    #[reaction(path = "g.x")]
    g_x: &'a mut runtime::Port<u32>,
}

impl Trigger for GainReactionTim<'_> {
    type Reactor = GainBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.g_x.get_mut() = Some(1);
    }
}

#[test]
fn gain() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<GainBuilder>("gain", (), true, false)
        .unwrap();
}
