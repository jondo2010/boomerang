#![allow(dead_code)]
#![allow(unused_variables)]

use boomerang::{
    builder::{Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct ABuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<()>,
    #[reactor(port = "output")]
    y: TypedPortKey<()>,
    reaction_x1: TypedReactionKey<AReactionX<'static>>,
    reaction_x2: TypedReactionKey<AReactionX<'static>>,
}

#[derive(Reaction)]
struct AReactionX<'a> {
    x: &'a runtime::Port<()>,
    y: &'a mut runtime::Port<()>,
}

impl<'a> Trigger for AReactionX<'a> {
    type Reactor = ABuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct BBuilder {
    #[reactor(port = "input")]
    x: TypedPortKey<()>,
    #[reactor(port = "output")]
    y: TypedPortKey<()>,
    reaction_x: TypedReactionKey<BReactionX>,
    reaction_startup: TypedReactionKey<BReactionStartup<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(port = "x"))]
struct BReactionX;

impl Trigger for BReactionX {
    type Reactor = BBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct BReactionStartup<'a> {
    y: &'a mut runtime::Port<()>,
}

impl Trigger for BReactionStartup<'_> {
    type Reactor = BBuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Clone, Reactor)]
#[reactor(
    state = "()",
    connection(from = "a.y", to = "b.x"),
    connection(from = "b.y", to = "a.x")
)]
struct CycleBuilder {
    #[reactor(child = ())]
    a: ABuilder,
    #[reactor(child = ())]
    b: BBuilder,
}

#[test]
fn cycle() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<CycleBuilder>("cycle", (), true, false)
        .unwrap();
}
