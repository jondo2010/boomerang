use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey, EnvBuilder, Reactor},
    runtime, Reactor, boomerang_test_body,
};

/// Test logical action with delay.

#[derive(Reactor)]
struct GeneratedDelayBuilder {
    #[reactor(input())]
    y_in: BuilderPortKey<u32>,
    #[reactor(output())]
    y_out: BuilderPortKey<u32>,
    #[reactor(action(physical = "false", min_delay = "100 msec"))]
    act: BuilderActionKey,
    #[reactor(reaction(function = "GeneratedDelay::reaction_y_in"))]
    reaction_y_in: runtime::ReactionKey,
    #[reactor(reaction(function = "GeneratedDelay::reaction_act"))]
    reaction_act: runtime::ReactionKey,
}

struct GeneratedDelay {
    y_state: u32,
}

impl GeneratedDelay {
    fn new() -> Self {
        Self { y_state: 0 }
    }

    /// y_in -> act
    #[boomerang::reaction(reactor = "GeneratedDelayBuilder")]
    fn reaction_y_in(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::port(triggers)] y_in: &runtime::Port<u32>,
        #[reactor::action(effects)] mut act: runtime::ActionMut,
    ) {
        self.y_state = y_in.get().unwrap();
        ctx.schedule_action(&mut act, None, None);
    }

    /// act -> y_out
    #[boomerang::reaction(reactor = "GeneratedDelayBuilder", triggers(action = "act"))]
    fn reaction_act(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects)] y_out: &mut runtime::Port<u32>,
    ) {
        *y_out.get_mut() = Some(self.y_state);
    }
}

#[derive(Reactor)]
struct SourceBuilder {
    #[reactor(output())]
    out: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Source::reaction_startup",))]
    reaction_startup: runtime::ReactionKey,
}

struct Source;
impl Source {
    #[boomerang::reaction(reactor = "SourceBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        _ctx: &runtime::Context,
        #[reactor::port(effects)] out: &mut runtime::Port<u32>,
    ) {
        *out.get_mut() = Some(1);
    }
}

#[derive(Reactor)]
struct SinkBuilder {
    #[reactor(input())]
    inp: BuilderPortKey<u32>,
    #[reactor(reaction(function = "Sink::reaction_in"))]
    reaction_in: runtime::ReactionKey,
}
struct Sink;
impl Sink {
    #[boomerang::reaction(reactor = "SinkBuilder")]
    fn reaction_in(
        &mut self,
        ctx: &runtime::Context,
        #[reactor::port(triggers, path = "inp")] _inp: &runtime::Port<u32>,
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

#[derive(Reactor)]
#[reactor(
    connection(from = "source.out", to = "g.y_in"),
    connection(from = "g.y_out", to = "sink.inp")
)]
#[allow(dead_code)]
struct ActionDelayBuilder {
    #[reactor(child(state = "Source{}"))]
    source: SourceBuilder,
    #[reactor(child(state = "Sink{}"))]
    sink: SinkBuilder,
    #[reactor(child(state = "GeneratedDelay::new()"))]
    g: GeneratedDelayBuilder,
}

boomerang_test_body!(action_delay, ActionDelayBuilder, ());