//! Test data transport across hierarchy.

use boomerang::prelude::*;

#[derive(Reactor)]
#[reactor(state = "()", reaction = "SourceReactionOut")]
struct SourceBuilder {
    out: TypedPortKey<u32, Output>,
    #[reactor(timer())]
    t: TimerActionKey,
}

#[derive(Reaction)]
#[reaction(reactor = "SourceBuilder", triggers(action = "t"))]
struct SourceReactionOut<'a> {
    out: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<()> for SourceReactionOut<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, _state: &mut ()) {
        *self.out = Some(1);
    }
}

#[derive(Debug, Default)]
struct Gain {
    gain: u32,
}

impl Gain {
    pub fn new(gain: u32) -> Self {
        Self { gain }
    }
}

#[derive(Reactor)]
#[reactor(state = "Gain", reaction = "GainReactionIn")]
struct GainBuilder {
    inp: TypedPortKey<u32, Input>,
    out: TypedPortKey<u32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "GainBuilder")]
struct GainReactionIn<'a> {
    inp: runtime::InputRef<'a, u32>,
    out: runtime::OutputRef<'a, u32>,
}

impl runtime::Trigger<Gain> for GainReactionIn<'_> {
    fn trigger(mut self, _ctx: &mut runtime::Context, state: &mut Gain) {
        *self.out = self.inp.map(|inp| inp * state.gain);
    }
}

#[derive(Reactor)]
#[reactor(state = "()", reaction = "PrintReactionIn")]
struct PrintBuilder {
    inp: TypedPortKey<u32, Input>,
    #[reactor(action())]
    a: TypedActionKey<()>,
}

#[derive(Reaction)]
#[reaction(reactor = "PrintBuilder")]
struct PrintReactionIn<'a> {
    inp: runtime::InputRef<'a, u32>,
    #[reaction(path = "a")]
    _a: runtime::ActionRef<'a>,
}

impl runtime::Trigger<()> for PrintReactionIn<'_> {
    fn trigger(self, _ctx: &mut runtime::Context, _state: &mut ()) {
        let value = *self.inp;
        assert!(matches!(value, Some(2u32)));
        println!("Received {}", value.unwrap());
    }
}

#[derive(Reactor)]
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
    #[reactor(child = "Gain::new(2)")]
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
    #[reactor(child = "()")]
    source: SourceBuilder,
    #[reactor(child = "()")]
    container: GainContainerBuilder,
    #[reactor(child = "()")]
    print: PrintBuilder,
    #[reactor(child = "()")]
    print2: PrintBuilder,
}

#[test]
fn hierarchy() {
    tracing_subscriber::fmt::init();
    let config = runtime::Config::default().with_fast_forward(true);
    let _ =
        boomerang_util::runner::build_and_test_reactor::<HierarchyBuilder>("hierarchy", (), config)
            .unwrap();
}
