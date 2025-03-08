use boomerang::prelude::*;

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

impl runtime::Trigger<()> for SourceReactionT<'_> {
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

impl runtime::Trigger<()> for DestReactionXY<'_> {
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

impl runtime::Trigger<()> for PassReactionX<'_> {
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
    #[reactor(child(state = ()))]
    s: SourceBuilder,
    #[reactor(child(state = ()))]
    d: DestinationBuilder,
    #[reactor(child(state = ()))]
    p1: PassBuilder,
    #[reactor(child(state = ()))]
    p2: PassBuilder,
}

#[test]
fn determinism() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<DeterminismBuilder>(
        "determinism",
        (),
        config,
    )
    .unwrap();
}
