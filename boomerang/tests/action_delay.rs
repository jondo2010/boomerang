use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

#[derive(Default)]
struct GeneratedDelayState {
    y_state: u32,
}

/// Test logical action with delay.

#[derive(Reactor, Clone)]
#[reactor(state = GeneratedDelayState, reaction = "ReactionYIn", reaction = "ReactionAct")]
struct GeneratedDelay {
    y_in: TypedPortKey<u32, Input>,
    y_out: TypedPortKey<u32, Output>,
    #[reactor(action(min_delay = "100 msec"))]
    act: TypedActionKey,
}

#[derive(Reaction)]
#[reaction(reactor = "GeneratedDelay")]
struct ReactionYIn<'a> {
    y_in: runtime::InputRef<'a, u32>,
    #[reaction(effects)]
    act: runtime::ActionRef<'a>,
}

impl Trigger<GeneratedDelay> for ReactionYIn<'_> {
    fn trigger(
        mut self,
        ctx: &mut runtime::Context,
        state: &mut <GeneratedDelay as Reactor>::State,
    ) {
        state.y_state = self.y_in.unwrap();
        ctx.schedule_action(&mut self.act, None, None);
    }
}

#[derive(Reaction)]
#[reaction(reactor = "GeneratedDelay", triggers(action = "act"))]
struct ReactionAct<'a> {
    y_out: runtime::OutputRef<'a, u32>,
}

impl Trigger<GeneratedDelay> for ReactionAct<'_> {
    fn trigger(
        mut self,
        _ctx: &mut runtime::Context,
        state: &mut <GeneratedDelay as Reactor>::State,
    ) {
        *self.y_out = Some(state.y_state);
    }
}

#[derive(Reactor, Clone)]
#[reactor(state = "()", reaction = "SourceReactionStartup")]
struct SourceBuilder {
    out: TypedPortKey<u32, Output>,
}

#[derive(Reaction)]
#[reaction(reactor = "SourceBuilder", triggers(startup))]
struct SourceReactionStartup<'a> {
    out: runtime::OutputRef<'a, u32>,
}

impl Trigger<SourceBuilder> for SourceReactionStartup<'_> {
    fn trigger(
        mut self,
        _ctx: &mut runtime::Context,
        _state: &mut <SourceBuilder as Reactor>::State,
    ) {
        *self.out = Some(1);
    }
}

#[derive(Reactor, Clone)]
#[reactor(state = "bool", reaction = "SinkReactionIn")]
struct Sink {
    inp: TypedPortKey<u32, Input>,
}

#[derive(Reaction)]
#[reaction(reactor = "Sink")]
struct SinkReactionIn<'a> {
    #[reaction(path = inp)]
    _inp: runtime::InputRef<'a, u32>,
}

impl Trigger<Sink> for SinkReactionIn<'_> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <Sink as Reactor>::State) {
        let elapsed_logical = ctx.get_elapsed_logical_time();
        let logical = ctx.get_logical_time();
        let physical = ctx.get_physical_time();
        println!("logical time: {:?}", logical);
        println!("physical time: {:?}", physical);
        println!("elapsed logical time: {:?}", elapsed_logical);
        assert!(
            elapsed_logical == runtime::Duration::from_millis(100),
            "ERROR: Expected 100 msecs but got {:?}",
            elapsed_logical
        );
        println!("SUCCESS. Elapsed logical time is 100 msec.");
        *state = true;
    }
}

#[derive(Reactor, Clone)]
#[reactor(
    state = "()",
    connection(from = "source.out", to = "g.y_in"),
    connection(from = "g.y_out", to = "sink.inp")
)]
#[allow(dead_code)]
struct ActionDelayBuilder {
    #[reactor(child = ())]
    source: SourceBuilder,
    #[reactor(child = false)]
    sink: Sink,
    #[reactor(child = GeneratedDelayState::default())]
    g: GeneratedDelay,
}

#[test]
fn action_delay() {
    tracing_subscriber::fmt::init();
    let (_, sched) = boomerang_util::runner::build_and_test_reactor::<ActionDelayBuilder>(
        "action_delay",
        (),
        true,
        false,
    )
    .unwrap();

    let env = sched.into_env();
    let sink_state = env
        .find_reactor_by_name("sink")
        .and_then(|reactor| reactor.get_state::<bool>())
        .unwrap();
    assert!(sink_state, "SinkReactionIn did not trigger");
}
