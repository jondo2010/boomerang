#![allow(dead_code)]
#![allow(unused_variables)]

use boomerang::{
    builder::{Input, Output, Trigger, TypedPortKey, TypedReactionKey},
    runtime, Reaction, Reactor,
};

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct ABuilder {
    x: TypedPortKey<(), Input>,
    y: TypedPortKey<(), Output>,
    reaction_x1: TypedReactionKey<AReactionX<'static>>,
    reaction_x2: TypedReactionKey<AReactionX<'static>>,
}

#[derive(Reaction)]
struct AReactionX<'a> {
    x: runtime::InputRef<'a, ()>,
    y: runtime::OutputRef<'a, ()>,
}

impl<'a> Trigger for AReactionX<'a> {
    type Reactor = ABuilder;
    fn trigger(&mut self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct BBuilder {
    x: TypedPortKey<(), Input>,
    y: TypedPortKey<(), Output>,
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
    y: runtime::OutputRef<'a, ()>,
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
