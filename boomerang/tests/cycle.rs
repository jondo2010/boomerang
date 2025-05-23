#![allow(unused)]

use boomerang::prelude::*;

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

impl<'a> runtime::Trigger<()> for AReactionX<'a> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {}
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

impl runtime::Trigger<()> for BReactionX {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {}
}

#[derive(Reaction)]
#[reaction(reactor = "BBuilder", triggers(startup))]
struct BReactionStartup<'a> {
    y: runtime::OutputRef<'a, ()>,
}

impl runtime::Trigger<()> for BReactionStartup<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {}
}

#[derive(Reactor)]
#[reactor(
    state = (),
    connection(from = "a.y", to = "b.x"),
    connection(from = "b.y", to = "a.x")
)]
struct CycleBuilder {
    #[reactor(child(state = ()))]
    a: ABuilder,
    #[reactor(child(state = ()))]
    b: BBuilder,
}

#[test]
fn cycle() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ = boomerang_util::runner::build_and_test_reactor::<CycleBuilder>("cycle", (), config)
        .unwrap();
}
