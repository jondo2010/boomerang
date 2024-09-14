//#![allow(dead_code)]

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

// Test data transport across hierarchy.

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct SourceBuilder {
    out: TypedPortKey<u32, Output>,
    #[reactor(timer())]
    t: TimerActionKey,
    reaction_out: TypedReactionKey<SourceReactionOut<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(action = "t"))]
struct SourceReactionOut<'a> {
    out: runtime::OutputRef<'a, u32>,
}

impl Trigger for SourceReactionOut<'_> {
    type Reactor = SourceBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.out = Some(1);
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
    inp: TypedPortKey<u32, Input>,
    out: TypedPortKey<u32, Output>,
    reaction_in: TypedReactionKey<GainReactionIn<'static>>,
}

#[derive(Reaction)]
struct GainReactionIn<'a> {
    inp: runtime::InputRef<'a, u32>,
    out: runtime::OutputRef<'a, u32>,
}

impl Trigger for GainReactionIn<'_> {
    type Reactor = GainBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, state: &mut Gain) {
        *self.out = self.inp.map(|inp| inp * state.gain);
    }
}

#[derive(Clone, Reactor)]
#[reactor(state = ())]
struct PrintBuilder {
    inp: TypedPortKey<u32, Input>,
    #[reactor(action())]
    a: TypedActionKey<()>,
    reaction_in: TypedReactionKey<PrintReactionIn<'static>>,
}

#[derive(Reaction)]
struct PrintReactionIn<'a> {
    inp: runtime::InputRef<'a, u32>,
    #[reaction(path = "a")]
    _a: runtime::ActionRef<'a>,
}

impl Trigger for PrintReactionIn<'_> {
    type Reactor = PrintBuilder;
    fn trigger(&mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        let value = *self.inp;
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
    inp: TypedPortKey<u32, Input>,
    out: TypedPortKey<u32, Output>,
    out2: TypedPortKey<u32, Output>,
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
