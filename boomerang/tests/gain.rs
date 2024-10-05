// Example in the Wiki.

use boomerang::prelude::*;

struct Scale(u32);

#[derive(Reactor)]
#[reactor(state = "Scale", reaction = "ScaleReactionX")]
struct ScaleBuilder {
    x: TypedPortKey<u32, Input>,
    y: TypedPortKey<u32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "ScaleBuilder")]
struct ScaleReactionX<'a> {
    x: runtime::InputRef<'a, u32>,
    y: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<Scale> for ScaleReactionX<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut Scale) {
        *self.y = Some(state.0 * self.x.unwrap());
    }
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "TestReactionX")]
struct TestBuilder {
    x: TypedPortKey<u32, Input>,
}

#[derive(Reaction)]
#[reaction(reactor = "TestBuilder")]
struct TestReactionX<'a> {
    x: runtime::InputRef<'a, u32>,
}

impl runtime::Trigger<()> for TestReactionX<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        println!("Received {:?}", *self.x);
        assert_eq!(*self.x, Some(2), "Expected Some(2)!");
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    reaction = "GainReactionTim",
    connection(from = "g.y", to = "t.x")
)]
struct GainBuilder {
    #[reactor(child = "Scale(2)")]
    g: ScaleBuilder,

    #[reactor(child = ())]
    #[allow(dead_code)]
    t: TestBuilder,

    #[reactor(timer())]
    tim: TimerActionKey,
}

#[derive(Reaction)]
#[reaction(reactor = "GainBuilder", triggers(action = "tim"))]
struct GainReactionTim<'a> {
    #[reaction(path = "g.x")]
    g_x: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<()> for GainReactionTim<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.g_x = Some(1);
    }
}

#[test]
fn gain() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor::<GainBuilder>("gain", (), config).unwrap();
}
