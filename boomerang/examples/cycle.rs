#![allow(dead_code)]
#![allow(unused_variables)]

use boomerang::{
    builder::{BuilderReactionKey, TypedPortKey},
    run, runtime, Reactor,
};

#[derive(Reactor)]
#[reactor(state = "A")]
struct ABuilder {
    #[reactor(input())]
    x: TypedPortKey<()>,
    #[reactor(output())]
    y: TypedPortKey<()>,
    #[reactor(reaction(function = "A::reaction_x"))]
    reaction_x1: BuilderReactionKey,
    #[reactor(reaction(function = "A::reaction_x"))]
    reaction_x2: BuilderReactionKey,
}

struct A;

impl A {
    #[boomerang::reaction(reactor = "ABuilder")]
    fn reaction_x(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<()>,
        #[reactor::port(effects)] y: &mut runtime::Port<()>,
    ) {
    }
}

#[derive(Reactor)]
#[reactor(state = "B")]
struct BBuilder {
    #[reactor(input())]
    x: TypedPortKey<()>,
    #[reactor(output())]
    y: TypedPortKey<()>,
    #[reactor(reaction(function = "B::reaction_x"))]
    reaction_x: BuilderReactionKey,
    #[reactor(reaction(function = "B::reaction_startup"))]
    reaction_startup: BuilderReactionKey,
}

struct B;

impl B {
    #[boomerang::reaction(reactor = "BBuilder")]
    fn reaction_x(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] x: &runtime::Port<()>,
    ) {
    }

    #[boomerang::reaction(reactor = "BBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(effects)] y: &mut runtime::Port<()>,
    ) {
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "a.y", to = "b.x"),
    connection(from = "b.y", to = "a.x")
)]
struct CycleBuilder {
    #[reactor(child(state = "A"))]
    a: ABuilder,
    #[reactor(child(state = "B"))]
    b: BBuilder,
}

fn main() {
    tracing_subscriber::fmt::init();
    let _ = run::build_and_run_reactor::<CycleBuilder>("cycle", ()).unwrap();
}
