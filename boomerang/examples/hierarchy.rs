#![allow(dead_code)]

use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey},
    runtime, Reactor,
};
use boomerang_util::build_and_run_reactor;

// Test data transport across hierarchy.

#[derive(Reactor)]
#[reactor(state = "Source")]
struct SourceBuilder {
    #[reactor(output())]
    out: BuilderPortKey<u32>,
    #[reactor(timer())]
    t: BuilderActionKey,
    #[reactor(reaction(function = "Source::reaction_out"))]
    reaction_out: runtime::ReactionKey,
}

struct Source;
impl Source {
    #[boomerang::reaction(reactor = "SourceBuilder", triggers(timer = "t"))]
    fn reaction_out(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects)] out: &mut runtime::Port<u32>,
    ) {
        *out.get_mut() = Some(1);
    }
}

#[derive(Reactor)]
#[reactor(state = "Gain")]
struct GainBuilder {
    #[reactor(input())]
    inp: BuilderPortKey<u32>,
    #[reactor(output())]
    out: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Gain::reaction_in"))]
    reaction_in: runtime::ReactionKey,
}

struct Gain {
    gain: u32,
}
impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
    #[boomerang::reaction(reactor = "GainBuilder")]
    fn reaction_in(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(triggers)] inp: &runtime::Port<u32>,
        #[reactor::port(effects)] out: &mut runtime::Port<u32>,
    ) {
        *out.get_mut() = inp.map(|inp| inp * self.gain);
    }
}

#[derive(Reactor)]
#[reactor(state = "Print")]
struct PrintBuilder {
    #[reactor(input())]
    inp: BuilderPortKey<u32>,
    #[reactor(action())]
    a: BuilderActionKey,
    #[reactor(reaction(function = "Print::reaction_in"))]
    reaction_in: runtime::ReactionKey,
}

struct Print;
impl Print {
    #[boomerang::reaction(reactor = "PrintBuilder")]
    fn reaction_in(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(triggers)] inp: &runtime::Port<u32>,
        #[reactor::action(effects, rename = "a")] mut _a: runtime::ActionMut,
    ) {
        let value = inp.get();
        assert!(matches!(value, Some(2u32)));
        println!("Received {}", value.unwrap());
    }
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "inp", to = "gain.inp"),
    connection(from = "gain.out", to = "out"),
    connection(from = "gain.out", to = "out2")
)]
struct GainContainerBuilder {
    #[reactor(input())]
    inp: BuilderPortKey<u32>,
    #[reactor(output())]
    out: BuilderPortKey<u32>,
    #[reactor(output())]
    out2: BuilderPortKey<u32>,
    #[reactor(child(state = "Gain::new(2)"))]
    gain: GainBuilder,
}

#[derive(Reactor)]
#[reactor(
    state = "()",
    connection(from = "source.out", to = "container.inp"),
    connection(from = "container.out", to = "print.inp"),
    connection(from = "container.out2", to = "print2.inp")
)]
struct HierarchyBuilder {
    #[reactor(child(state = "Source{}"))]
    source: SourceBuilder,
    #[reactor(child(state = "()"))]
    container: GainContainerBuilder,
    #[reactor(child(state = "Print"))]
    print: PrintBuilder,
    #[reactor(child(state = "Print"))]
    print2: PrintBuilder,
}

fn main() {
    tracing_subscriber::fmt::init();
    let _ = build_and_run_reactor::<HierarchyBuilder>("hierarchy", ()).unwrap();
}
