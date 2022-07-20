use boomerang::{
    builder::{ActionPart, BuilderInputPort, BuilderOutputPort, EnvBuilder, Reactor},
    runtime, Reactor,
};

/// Test logical action with delay.

#[derive(Reactor)]
struct GeneratedDelayBuilder {
    y_in: BuilderInputPort<u32>,
    y_out: BuilderOutputPort<u32>,
    #[action(physical = "false", min_delay = "100 msec")]
    act: ActionPart,
    #[reaction(function = "GeneratedDelay::reaction_y_in")]
    reaction_y_in: runtime::ReactionKey,
    #[reaction(function = "GeneratedDelay::reaction_act")]
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
    out: BuilderOutputPort<u32>,
    #[reaction(function = "Source::reaction_startup")]
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
    inp: BuilderInputPort<u32>,
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

#[test]
fn action_delay() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let mut env_builder = EnvBuilder::new();
    let _ = ActionDelayBuilder::build("action_delay", (), None, &mut env_builder).unwrap();

    // let gv = graphviz::build(&env_builder).unwrap();
    // let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    // std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    // let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    // let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    // std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let (env, dep_info) = env_builder.try_into().unwrap();

    runtime::check_consistency(&env, &dep_info);
    runtime::debug_info(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}
