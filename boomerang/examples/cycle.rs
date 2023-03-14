#![allow(dead_code)]
#![allow(unused_variables)]

use boomerang::{builder::BuilderPortKey, runtime, Reactor, run};

#[derive(Reactor)]
#[reactor(state = "A")]
struct ABuilder {
    #[reactor(input())]
    x: BuilderPortKey<()>,
    #[reactor(output())]
    y: BuilderPortKey<()>,
    #[reactor(reaction(function = "A::reaction_x"))]
    reaction_x1: runtime::ReactionKey,
    #[reactor(reaction(function = "A::reaction_x"))]
    reaction_x2: runtime::ReactionKey,
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
    x: BuilderPortKey<()>,
    #[reactor(output())]
    y: BuilderPortKey<()>,

    #[reactor(reaction(function = "B::reaction_x"))]
    reaction_x: runtime::ReactionKey,

    #[reactor(reaction(function = "B::reaction_startup"))]
    reaction_startup: runtime::ReactionKey,
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
