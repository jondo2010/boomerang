//! Test data transport across hierarchy.

use boomerang::prelude::*;

#[boomerang_derive::reactor]
fn Source(#[output] out: u32) -> impl Reactor2 {
    let t = builder.add_timer("t", TimerSpec::default())?;
    builder
        .add_reaction2(Some("SourceReactionOut"))
        .with_trigger(t)
        .with_effect(out)
        .with_reaction_fn(|_ctx, _state, (_t, mut out)| {
            *out = Some(1);
        })
        .finish()?;
}

#[derive(typed_builder::TypedBuilder)]
struct GainParams {
    #[builder(default = 1)]
    gain: u32,
}

#[boomerang_derive::reactor]
fn Gain(#[input] inp: u32, #[output] out: u32, gain: u32) -> impl Reactor2 {
    builder
        .add_reaction2(Some("GainReactionIn"))
        .with_trigger(inp)
        .with_effect(out)
        .with_reaction_fn(move |_ctx, _state, (inp, mut out)| {
            *out = Some(inp.unwrap() * gain);
        })
        .finish()?;
}

#[boomerang_derive::reactor]
fn Print(#[input] inp: u32) -> impl Reactor2 {
    builder
        .add_reaction2(Some("PrintReactionIn"))
        .with_trigger(inp)
        .with_reaction_fn(|_ctx, _state, (inp,)| {
            let value = *inp;
            assert!(matches!(value, Some(2u32)));
            println!("Received {}", value.unwrap());
        })
        .finish()?;
}

#[boomerang_derive::reactor]
fn GainContainer(#[input] inp: u32, #[output] out: u32, #[output] out2: u32) -> impl Reactor2 {
    let gain = builder.add_child_reactor2(Gain(), "gain", (), false)?;
    builder.connect_port(inp, gain.inp, None, false)?;
    builder.connect_port(gain.out, out, None, false)?;
    builder.connect_port(gain.out, out2, None, false)?;
}

#[boomerang_derive::reactor]
fn Hierarchy() -> impl Reactor2 {
    let source = builder.add_child_reactor2(Source(), "source", (), false)?;
    let container = builder.add_child_reactor2(GainContainer(), "container", (), false)?;
    let print = builder.add_child_reactor2(Print(), "print", (), false)?;
    let print2 = builder.add_child_reactor2(Print(), "print2", (), false)?;

    builder.connect_port(source.out, container.inp, None, false)?;
    builder.connect_port(container.out, print.inp, None, false)?;
    builder.connect_port(container.out2, print2.inp, None, false)?;
}

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
    #[reactor(child(state = Gain::new(2)))]
    gain: GainBuilder,
}

#[derive(Reactor)]
#[reactor(
    state = (),
    connection(from = "source.out", to = "container.inp"),
    connection(from = "container.out", to = "print.inp"),
    connection(from = "container.out2", to = "print2.inp")
)]
struct HierarchyBuilder {
    #[reactor(child(state = ()))]
    source: SourceBuilder,
    #[reactor(child(state = ()))]
    container: GainContainerBuilder,
    #[reactor(child(state = ()))]
    print: PrintBuilder,
    #[reactor(child(state = ()))]
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
