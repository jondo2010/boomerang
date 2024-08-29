use boomerang::builder::{Reactor, Trigger, TypedActionKey, TypedPortKey, TypedReactionKey};
use boomerang::{runtime, Reaction, Reactor};

/// Test logical action with delay.

#[derive(Reactor, Clone)]
#[reactor(state = "GeneratedDelayState")]
struct GeneratedDelay {
    #[reactor(port = "input")]
    y_in: TypedPortKey<u32>,

    #[reactor(port = "output")]
    y_out: TypedPortKey<u32>,

    #[reactor(action(physical = "false", min_delay = "100 msec"))]
    act: TypedActionKey,

    reaction_y_in: TypedReactionKey<ReactionYIn<'static>>,
    reaction_act: TypedReactionKey<ReactionAct<'static>>,
}

#[derive(Default)]
struct GeneratedDelayState {
    y_state: u32,
}

#[derive(Reaction)]
struct ReactionYIn<'a> {
    y_in: &'a runtime::Port<u32>,
    #[reaction(effects)]
    act: runtime::ActionRef<'a>,
}

impl Trigger for ReactionYIn<'_> {
    type Reactor = GeneratedDelay;

    fn trigger(
        &mut self,
        ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    ) {
        state.y_state = self.y_in.get().unwrap();
        ctx.schedule_action(&mut self.act, None, None);
    }
}

#[derive(Reaction)]
#[reaction(triggers(action = "act"))]
struct ReactionAct<'a> {
    y_out: &'a mut runtime::Port<u32>,
}

impl Trigger for ReactionAct<'_> {
    type Reactor = GeneratedDelay;

    fn trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        state: &mut <Self::Reactor as Reactor>::State,
    ) {
        *self.y_out.get_mut() = Some(state.y_state);
    }
}

#[derive(Reactor, Clone)]
#[reactor(state = "()")]
struct SourceBuilder {
    #[reactor(port = "output")]
    out: TypedPortKey<u32>,
    reaction_startup: TypedReactionKey<SourceReactionStartup<'static>>,
}

#[derive(Reaction)]
#[reaction(triggers(startup))]
struct SourceReactionStartup<'a> {
    out: &'a mut runtime::Port<u32>,
}

impl Trigger for SourceReactionStartup<'_> {
    type Reactor = SourceBuilder;

    fn trigger(
        &mut self,
        _ctx: &mut runtime::Context,
        _state: &mut <Self::Reactor as Reactor>::State,
    ) {
        *self.out.get_mut() = Some(1);
    }
}

#[derive(Reactor, Clone)]
#[reactor(state = "()")]
struct Sink {
    #[reactor(port = "input")]
    inp: TypedPortKey<u32>,
    reaction_in: TypedReactionKey<SinkReactionIn<'static>>,
}

#[derive(Reaction)]
struct SinkReactionIn<'a> {
    #[reaction(path = inp)]
    _inp: &'a runtime::Port<u32>,
}

impl Trigger for SinkReactionIn<'_> {
    type Reactor = Sink;

    fn trigger(
        &mut self,
        ctx: &mut runtime::Context,
        _state: &mut <Self::Reactor as Reactor>::State,
    ) {
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
    #[reactor(child = ())]
    sink: Sink,
    #[reactor(child = GeneratedDelayState::default())]
    g: GeneratedDelay,
}

#[test]
fn action_delay() {
    tracing_subscriber::fmt::init();
    let _ = boomerang_util::run::build_and_test_reactor::<ActionDelayBuilder>(
        "action_delay",
        (),
        true,
        false,
    )
    .unwrap();
}
