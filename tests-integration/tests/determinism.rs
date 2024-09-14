use boomerang::{
    builder::{Input, Output, TimerActionKey, Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct SourceBuilder {
    y: TypedPortKey<i32, Output>,
    #[reactor(timer())]
    t: TimerActionKey,
    reaction_t: TypedReactionKey<SourceReactionT<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct SourceReactionT<'a> {
    y: runtime::OutputRef<'a, i32>,
}

impl Trigger for SourceReactionT<'_> {
    type Reactor = SourceBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y = Some(1);
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct DestinationBuilder {
    x: TypedPortKey<i32, Input>,
    #[reactor(port = "input")]
    y: TypedPortKey<i32, Input>,
    reaction_x_y: TypedReactionKey<DestReactionXY<'static>>,
}

#[derive(Reaction)]
struct DestReactionXY<'a> {
    x: runtime::InputRef<'a, i32>,
    y: runtime::InputRef<'a, i32>,
}

impl Trigger for DestReactionXY<'_> {
    type Reactor = DestinationBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
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

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct PassBuilder {
    x: TypedPortKey<i32, Input>,
    y: TypedPortKey<i32, Output>,
    reaction_x: TypedReactionKey<PassReactionX<'static>>,
}

#[derive(Reaction)]
struct PassReactionX<'a> {
    x: runtime::InputRef<'a, i32>,
    y: runtime::OutputRef<'a, i32>,
}

impl Trigger for PassReactionX<'_> {
    type Reactor = PassBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y = *self.x;
    }
}

#[derive(Clone, Reactor)]
#[reactor(
    state = (),
    connection(from = "s.y", to = "d.y"),
    connection(from = "s.y", to = "p1.x"),
    connection(from = "p1.y", to = "p2.x"),
    connection(from = "p2.y", to = "d.x")
)]
#[allow(dead_code)]
struct DeterminismBuilder {
    #[reactor(child = ())]
    s: SourceBuilder,
    #[reactor(child = ())]
    d: DestinationBuilder,
    #[reactor(child = ())]
    p1: PassBuilder,
    #[reactor(child = ())]
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
