use boomerang::{
    builder::{TimerActionKey, Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct SourceBuilder {
    #[reactor(port = "output")]
    y: TypedPortKey<i32>,
    #[reactor(timer())]
    t: TimerActionKey,
    reaction_t: TypedReactionKey<SourceReactionT<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct SourceReactionT<'a> {
    y: &'a mut runtime::Port<i32>,
}

impl Trigger for SourceReactionT<'_> {
    type Reactor = SourceBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y.get_mut() = Some(1);
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct DestinationBuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<i32>,
    #[reactor(port = "input")]
    y: TypedPortKey<i32>,
    reaction_x_y: TypedReactionKey<DestReactionXY<'static>>,
}

#[derive(Reaction)]
struct DestReactionXY<'a> {
    x: &'a runtime::Port<i32>,
    y: &'a runtime::Port<i32>,
}

impl Trigger for DestReactionXY<'_> {
    type Reactor = DestinationBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        let mut sum = 0;
        if let Some(x) = *self.x.get() {
            sum += x;
        }
        if let Some(y) = *self.y.get() {
            sum += y;
        }
        println!("Received {}", sum);
        assert_eq!(sum, 2, "FAILURE: Expected 2.");
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct PassBuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<i32>,
    #[reactor(port = "output")]
    y: TypedPortKey<i32>,
    reaction_x: TypedReactionKey<PassReactionX<'static>>,
}

#[derive(Reaction)]
struct PassReactionX<'a> {
    x: &'a runtime::Port<i32>,
    y: &'a mut runtime::Port<i32>,
}

impl Trigger for PassReactionX<'_> {
    type Reactor = PassBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.y.get_mut() = *self.x.get();
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
