use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

#[derive(Reactor)]
#[reactor(state = "()", reaction = "SourceReactionT")]
struct SourceBuilder {
    y: TypedPortKey<i32, Output>,
    #[reactor(timer())]
    t: TimerActionKey,
}

#[derive(Reaction)]
#[reaction(reactor = "SourceBuilder", triggers(action = "t"))]
struct SourceReactionT<'a> {
    y: runtime::OutputRef<'a, i32>,
}

impl Trigger<SourceBuilder> for SourceReactionT<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y = Some(1);
    }
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "DestReactionXY")]
struct DestinationBuilder {
    x: TypedPortKey<i32, Input>,
    y: TypedPortKey<i32, Input>,
}

#[derive(Reaction)]
#[reaction(reactor = "DestinationBuilder")]
struct DestReactionXY<'a> {
    x: runtime::InputRef<'a, i32>,
    y: runtime::InputRef<'a, i32>,
}

impl Trigger<DestinationBuilder> for DestReactionXY<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        let mut sum = 0;
        if let Some(x) = *self.x {
            sum += x;
        }
        if let Some(y) = *self.y {
            sum += y;
        }
        println!("Received {}", sum);
        assert_eq!(sum, 2, "FAILURE: Expected 2.");
    }
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "PassReactionX")]
struct PassBuilder {
    x: TypedPortKey<i32, Input>,
    y: TypedPortKey<i32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "PassBuilder")]
struct PassReactionX<'a> {
    x: runtime::InputRef<'a, i32>,
    y: runtime::OutputRef<'a, i32>,
}

impl Trigger<PassBuilder> for PassReactionX<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y = *self.x;
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "s.y", to = "d.y"),
    connection(from = "s.y", to = "p1.x"),
    connection(from = "p1.y", to = "p2.x"),
    connection(from = "p2.y", to = "d.x")
)]
#[allow(dead_code)]
struct DeterminismBuilder {
    #[reactor(child = "()")]
    s: SourceBuilder,
    #[reactor(child = "()")]
    d: DestinationBuilder,
    #[reactor(child = "()")]
    p1: PassBuilder,
    #[reactor(child = "()")]
    p2: PassBuilder,
}

#[test]
fn determinism() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<DeterminismBuilder>(
        "determinism",
        (),
        true,
        false,
    )
    .unwrap();
}
