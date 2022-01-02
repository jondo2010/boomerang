use boomerang::{
    builder::{BuilderActionKey, BuilderPortKey},
    runtime, Reactor,
};

// Test data transport across hierarchy.

#[derive(Reactor)]
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
struct PrintBuilder {
    #[reactor(input())]
    inp: BuilderPortKey<u32>,
    #[reactor(action())]
    act: BuilderActionKey,
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
        #[reactor::action(effects)] mut act: runtime::ActionMut,
    ) {
        let value = inp.get();
        assert!(matches!(value, Some(2u32)));
        println!("Received {}", value.unwrap());
    }
}

#[derive(Reactor)]
#[reactor(
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
    connection(from = "source.out", to = "container.inp"),
    connection(from = "container.out", to = "print.inp"),
    connection(from = "container.out", to = "print2.inp")
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

#[test]
fn hierarchy() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    use boomerang::builder::*;
    let mut env_builder = EnvBuilder::new();

    let _ = HierarchyBuilder::build("top", (), None, &mut env_builder).unwrap();

    // boomerang::builder::graphviz::reaction_graph::render_to(&env_builder, &mut stdout());
    // let gv = graphviz::build(&env_builder).unwrap();
    // let mut f = std::fs::File::create(format!("{}.dot", module_path!())).unwrap();
    // std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let gv = graphviz::build_reaction_graph(&env_builder).unwrap();
    let mut f = std::fs::File::create(format!("{}_levels.dot", module_path!())).unwrap();
    std::io::Write::write_all(&mut f, gv.as_bytes()).unwrap();

    let (env, dep_info) = env_builder.try_into().unwrap();
    assert!(env.ports.len() == 2);

    runtime::check_consistency(&env, &dep_info);
    // runtime::debug_info(&env, &dep_info);

    let sched = runtime::Scheduler::new(env, dep_info, true);
    sched.event_loop();
}
