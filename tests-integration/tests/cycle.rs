#![allow(dead_code)]
#![allow(unused_variables)]

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

#[derive(Reactor)]
#[reactor(state = "()", reaction = "AReactionX")]
struct ABuilder {
    x: TypedPortKey<(), Input>,
    y: TypedPortKey<(), Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "ABuilder")]
struct AReactionX<'a> {
    x: runtime::InputRef<'a, ()>,
    y: runtime::OutputRef<'a, ()>,
}

impl<'a> Trigger<ABuilder> for AReactionX<'a> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "BReactionStartup", reaction = "BReactionX")]
struct BBuilder {
    x: TypedPortKey<(), Input>,
    y: TypedPortKey<(), Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "BBuilder", triggers(port = "x"))]
struct BReactionX;

impl Trigger<BBuilder> for BReactionX {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Reaction)]
#[reaction(reactor = "BBuilder", triggers(startup))]
struct BReactionStartup<'a> {
    y: runtime::OutputRef<'a, ()>,
}

impl Trigger<BBuilder> for BReactionStartup<'_> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut ()) {}
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "a.y", to = "b.x"),
    connection(from = "b.y", to = "a.x")
)]
struct CycleBuilder {
    #[reactor(child = "()")]
    a: ABuilder,
    #[reactor(child = "()")]
    b: BBuilder,
}

#[test]
fn cycle() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<CycleBuilder>("cycle", (), true, false)
        .unwrap();
}
