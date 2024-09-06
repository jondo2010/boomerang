//#![allow(dead_code)]

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

// Test data transport across hierarchy.

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct SourceBuilder {
    #[reactor(port = "output")]
    out: TypedPortKey<u32>,
    #[reactor(timer())]
    t: TimerActionKey,
    reaction_out: TypedReactionKey<SourceReactionOut<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct SourceReactionOut<'a> {
    out: &'a mut runtime::Port<u32>,
}

impl Trigger for SourceReactionOut<'_> {
    type Reactor = SourceBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.out.get_mut() = Some(1);
    }
}

struct Gain {
    gain: u32,
}

impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = Gain)]
struct GainBuilder {
    #[reactor(port = "input")]
    inp: TypedPortKey<u32>,
    #[reactor(port = "output")]
    out: TypedPortKey<u32>,
    reaction_in: TypedReactionKey<GainReactionIn<'static>>,
}

#[derive(Reaction)]
struct GainReactionIn<'a> {
    inp: &'a runtime::Port<u32>,
    out: &'a mut runtime::Port<u32>,
}

impl Trigger for GainReactionIn<'_> {
    type Reactor = GainBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Gain) {
        *self.out.get_mut() = self.inp.map(|inp| inp * state.gain);
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct PrintBuilder {
    #[reactor(port = "input")]
    inp: TypedPortKey<u32>,
    #[reactor(action())]
    a: TypedActionKey<()>,
    reaction_in: TypedReactionKey<PrintReactionIn<'static>>,
}

#[derive(Reaction)]
struct PrintReactionIn<'a> {
    inp: &'a runtime::Port<u32>,
    #[reaction(path = "a")]
    _a: runtime::ActionRef<'a>,
}

impl Trigger for PrintReactionIn<'_> {
    type Reactor = PrintBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        let value = self.inp.get();
        assert!(matches!(value, Some(2u32)));
        println!("Received {}", value.unwrap());
    }
}

#[derive(Clone, Reactor)]
#[reactor(
    state = (),
    connection(from = "inp", to = "gain.inp"),
    connection(from = "gain.out", to = "out"),
    connection(from = "gain.out", to = "out2")
)]
struct GainContainerBuilder {
    #[reactor(port = "input")]
    inp: TypedPortKey<u32>,
    #[reactor(port = "output")]
    out: TypedPortKey<u32>,
    #[reactor(port = "output")]
    out2: TypedPortKey<u32>,
    #[reactor(child= Gain::new(2))]
    gain: GainBuilder,
}

#[derive(Clone, Reactor)]
#[reactor(
    state = (),
    connection(from = "source.out", to = "container.inp"),
    connection(from = "container.out", to = "print.inp"),
    connection(from = "container.out2", to = "print2.inp")
)]
struct HierarchyBuilder {
    #[reactor(child= ())]
    source: SourceBuilder,
    #[reactor(child= ())]
    container: GainContainerBuilder,
    #[reactor(child = ())]
    print: PrintBuilder,
    #[reactor(child= ())]
    print2: PrintBuilder,
}

#[test]
fn hierarchy() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<HierarchyBuilder>(
        "hierarchy",
        (),
        true,
        false,
    )
    .unwrap();
}
