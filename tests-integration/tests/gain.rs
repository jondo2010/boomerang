// Example in the Wiki.

use boomerang::{
    builder::{Input, Output, TimerActionKey, Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

struct Scale(u32);

#[derive(Clone, Reactor)]
#[reactor(state = Scale)]
struct ScaleBuilder {
    x: TypedPortKey<u32, Input>,
    y: TypedPortKey<u32, Output>,
    reaction_x: TypedReactionKey<ScaleReactionX<'static>>,
}

#[derive(Reaction)]
struct ScaleReactionX<'a> {
    x: runtime::InputRef<'a, u32>,
    y: runtime::OutputRef<'a, u32>,
}

impl Trigger for ScaleReactionX<'_> {
    type Reactor = ScaleBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Scale) {
        *self.y = Some(state.0 * self.x.unwrap());
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct TestBuilder {
    x: TypedPortKey<u32, Input>,
    reaction_x: TypedReactionKey<TestReactionX<'static>>,
}

#[derive(Reaction)]
struct TestReactionX<'a> {
    x: runtime::InputRef<'a, u32>,
}

impl Trigger for TestReactionX<'_> {
    type Reactor = TestBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("Received {:?}", *self.x);
        assert_eq!(*self.x, Some(2), "Expected Some(2)!");
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
    g_x: runtime::OutputRef<'a, u32>,
}

impl Trigger for GainReactionTim<'_> {
    type Reactor = GainBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.g_x = Some(1);
    }
}

#[test]
fn gain() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<GainBuilder>("gain", (), true, false)
        .unwrap();
}
